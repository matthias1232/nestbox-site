//! Find the string values of a JSON document by byte span, so a detector can
//! rewrite one value without re-serialising the document.
//!
//! Re-serialising through `serde_json::Value` would reorder keys and reformat
//! numbers. A model provider's prompt cache keys on exact bytes, and an agent's
//! tool definitions are compared by the server as they were sent: a rewritten
//! request must differ in the value that held a finding and in nothing else.
//!
//! The scanner is a byte walker, not a parser: `string_values` first proves the
//! document is JSON (`IgnoredAny`), then walks it only far enough to know where
//! each string value starts and ends and which object keys lead to it. It never
//! decodes numbers and never rebuilds the tree.

/// One string *value* (never an object key) and where it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonString {
    /// Byte offset of the opening quote.
    pub start: usize,
    /// Byte offset one past the closing quote.
    pub end: usize,
    /// The decoded value: escapes resolved, no surrounding quotes.
    pub value: String,
    /// The object keys from the root to this value, e.g. `["messages", "content", "text"]`.
    /// An array adds nothing: the element keeps the key of the array.
    pub path: Vec<String>,
}

/// Protocol fields, ids, binary payloads and role markers. None of them is text
/// a person wrote, and rewriting one breaks the request in ways nobody can read.
pub const SKIPPED_KEYS: &[&str] = &[
    "model",
    "type",
    "role",
    "id",
    "tool_use_id",
    "name",
    "signature",
    "media_type",
    "stop_reason",
    "stop_sequence",
];

/// A value under any of these ancestors is skipped entirely.
///
/// `tools` is the definition list an agent ships with every request: replacing a
/// hostname in a tool description would tell the model about a tool that no
/// longer matches what it can call.
pub const SKIPPED_SUBTREES: &[&str] = &["tools", "tool_choice", "metadata", "cache_control"];

/// Login material. Checking it can only break a login, never protect a person:
/// a refresh token is machine-issued and rotating, and a policy that blocks it
/// signs the user out.
pub const SKIPPED_SECRET_KEYS: &[&str] = &[
    "refresh_token",
    "access_token",
    "code",
    "code_verifier",
    "client_id",
    "state",
    "grant_type",
];

/// Whether the value at `path` is out of scope for the content check.
pub fn is_skipped(path: &[String]) -> bool {
    if let Some(last) = path.last() {
        if SKIPPED_KEYS.contains(&last.as_str()) || SKIPPED_SECRET_KEYS.contains(&last.as_str()) {
            return true;
        }
    }
    path.iter()
        .any(|key| SKIPPED_SUBTREES.contains(&key.as_str()))
}

/// Every string value of `raw`, in document order. `None` when `raw` is not JSON.
pub fn string_values(raw: &str) -> Option<Vec<JsonString>> {
    serde_json::from_str::<serde_json::Value>(raw).ok()?;

    let bytes = raw.as_bytes();
    let mut values = Vec::new();
    // One entry per open container: the object key it was reached under, or
    // `None` at the root and inside an array element that had no key.
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut pending_key: Option<String> = None;
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let end = string_end(bytes, index)?;
                let literal = &raw[index..end];
                let decoded: String = serde_json::from_str(literal).ok()?;
                let mut after = end;
                while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                    after += 1;
                }
                if after < bytes.len() && bytes[after] == b':' {
                    // A key, not a value.
                    pending_key = Some(decoded);
                } else {
                    let mut path: Vec<String> = stack.iter().flatten().cloned().collect();
                    if let Some(key) = pending_key.take() {
                        path.push(key);
                    }
                    values.push(JsonString {
                        start: index,
                        end,
                        value: decoded,
                        path,
                    });
                }
                index = end;
            }
            b'{' | b'[' => {
                stack.push(pending_key.take());
                index += 1;
            }
            b'}' | b']' => {
                stack.pop();
                index += 1;
            }
            b',' => {
                pending_key = None;
                index += 1;
            }
            _ => index += 1,
        }
    }
    Some(values)
}

/// Byte offset one past the closing quote of the string that starts at `start`.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// Replace the given spans with new JSON string literals.
///
/// Spans must come from [`string_values`] on the same text and must not overlap.
/// An out-of-order or overlapping edit is dropped rather than applied to the
/// wrong bytes: a request that is refused is recoverable, one that is subtly
/// corrupted is not.
pub fn replace_spans(raw: &str, edits: &[(usize, usize, String)]) -> String {
    if edits.is_empty() {
        return raw.to_string();
    }
    let mut ordered: Vec<&(usize, usize, String)> = edits.iter().collect();
    ordered.sort_by_key(|(start, _, _)| *start);

    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0usize;
    for (start, end, value) in ordered {
        if *start < cursor || *end > raw.len() || start >= end {
            continue;
        }
        out.push_str(&raw[cursor..*start]);
        match serde_json::to_string(value) {
            Ok(literal) => out.push_str(&literal),
            Err(_) => out.push_str(&raw[*start..*end]),
        }
        cursor = *end;
    }
    out.push_str(&raw[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(values: &[JsonString]) -> Vec<String> {
        values.iter().map(|value| value.path.join(".")).collect()
    }

    #[test]
    fn finds_every_value_and_no_key() {
        let values = string_values(r#"{"a":"x","b":["y",{"c":"z"}]}"#).expect("valid json");
        assert_eq!(paths(&values), vec!["a", "b", "b.c"]);
        assert_eq!(values[0].value, "x");
        assert_eq!(values[2].value, "z");
    }

    #[test]
    fn keys_are_never_values() {
        let values = string_values(r#"{"secret_key":"v"}"#).expect("valid json");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].value, "v");
    }

    #[test]
    fn escapes_are_decoded_and_re_encoded() {
        let raw = r#"{"t":"line\nexample.com \"q\""}"#;
        let values = string_values(raw).expect("valid json");
        assert_eq!(values[0].value, "line\nexample.com \"q\"");
        let out = replace_spans(raw, &[(values[0].start, values[0].end, "<X>".into())]);
        assert_eq!(out, r#"{"t":"<X>"}"#);
        serde_json::from_str::<serde_json::Value>(&out).expect("still json");
    }

    #[test]
    fn an_escape_is_not_split() {
        // The old text replacement produced `\<HOSTNAMES_...>` here, which is
        // not a valid escape and made the provider answer 400.
        let raw = r#"{"t":"a\nexample.com"}"#;
        let values = string_values(raw).expect("valid json");
        let out = replace_spans(raw, &[(values[0].start, values[0].end, "<HOSTNAMES_1>".into())]);
        serde_json::from_str::<serde_json::Value>(&out).expect("still json");
    }

    #[test]
    fn invalid_json_is_refused() {
        assert!(string_values("{nope").is_none());
        assert!(string_values("").is_none());
    }

    #[test]
    fn unicode_is_kept_on_character_boundaries() {
        let raw = r#"{"t":"Müller €","u":"ok"}"#;
        let values = string_values(raw).expect("valid json");
        assert_eq!(values[0].value, "Müller €");
        let out = replace_spans(raw, &[(values[0].start, values[0].end, "M".into())]);
        assert_eq!(out, r#"{"t":"M","u":"ok"}"#);
    }

    #[test]
    fn without_edits_the_text_is_unchanged() {
        let raw = r#"{"a":"x","b":[1,2.50,true,null]}"#;
        assert_eq!(replace_spans(raw, &[]), raw);
    }

    #[test]
    fn skip_rules() {
        let path = |value: &str| -> Vec<String> {
            value.split('.').map(str::to_string).collect()
        };
        assert!(is_skipped(&path("tools.description")));
        assert!(is_skipped(&path("messages.model")));
        assert!(is_skipped(&path("refresh_token")));
        assert!(!is_skipped(&path("messages.content.text")));
        assert!(!is_skipped(&path("system.text")));
    }
}
