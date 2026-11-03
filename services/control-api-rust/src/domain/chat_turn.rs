//! What one model call says, read from its request and response.
//!
//! Pure: no database, no network, no clock. Three dialects are understood —
//! Anthropic Messages, OpenAI Chat Completions and OpenAI Responses — in JSON and
//! in server-sent events, because an agent streams by default and a stored answer
//! is often the stitched stream.
//!
//! The texts handed in are already masked. This module never decides whether a
//! value may be kept; it only reads what is there.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Longest user text a turn keeps, in characters.
pub const USER_TEXT_MAX: usize = 4096;
/// Longest answer text a turn keeps, in characters.
pub const ASSISTANT_TEXT_MAX: usize = 16384;
/// Longest tool input a turn keeps, in characters.
const TOOL_INPUT_MAX: usize = 400;

/// One tool call the model made.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ToolCall {
    pub name: String,
    pub input: String,
}

/// What one model call says.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnFacts {
    pub requested_model: Option<String>,
    pub served_model: Option<String>,
    pub session_id: Option<String>,
    pub user_text: String,
    pub assistant_text: String,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

/// At most `max` characters, cut on a character boundary.
pub fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Whether a request is a call to a language model.
pub fn is_model_call(method: &str, path: &str) -> bool {
    if !method.eq_ignore_ascii_case("POST") {
        return false;
    }
    let path = path.split('?').next().unwrap_or(path);
    if path.ends_with("/count_tokens") {
        return false;
    }
    const ENDINGS: [&str; 5] = [
        "/v1/messages",
        "/v1/chat/completions",
        "/chat/completions",
        "/v1/responses",
        "/responses",
    ];
    ENDINGS.iter().any(|ending| path.ends_with(ending))
}

/// The facts of one call. Either text may be empty.
pub fn turn_from(request: &str, response: &str) -> TurnFacts {
    let parsed = serde_json::from_str::<serde_json::Value>(request).ok();
    let mut facts = TurnFacts::default();

    match &parsed {
        Some(value) => {
            facts.requested_model = string_at(value, &["model"]);
            facts.session_id = session_of(request).or_else(|| session_from_metadata(value));
            facts.user_text = last_user_text(value);
        }
        None => {
            // A request cut at the size limit is not JSON. The model sits at the
            // start and the newest message at the end; both are still readable.
            facts.requested_model = json_string_field(request, "model");
            facts.user_text = json_string_field(request, "content").unwrap_or_default();
            facts.session_id = session_of(request);
        }
    }

    if !response.trim().is_empty() {
        read_response(response, &mut facts);
    }

    facts.user_text = truncate_chars(facts.user_text.trim(), USER_TEXT_MAX);
    facts.assistant_text = truncate_chars(facts.assistant_text.trim(), ASSISTANT_TEXT_MAX);
    facts
}

/// The provider a host belongs to. Host without port, case-insensitive.
pub fn provider_for(host: &str) -> &'static str {
    let host = host
        .split(':')
        .next()
        .unwrap_or(host)
        .trim()
        .to_ascii_lowercase();
    if host == "anthropic.com"
        || host.ends_with(".anthropic.com")
        || host == "claude.ai"
        || host.ends_with(".claude.ai")
    {
        return "anthropic";
    }
    if host == "api.openai.com" || host.ends_with("chatgpt.com") || host.ends_with("openai.azure.com") {
        return "openai";
    }
    if host == "generativelanguage.googleapis.com" {
        return "google";
    }
    if host == "openrouter.ai" {
        return "openrouter";
    }
    if host == "api.deepseek.com" {
        return "deepseek";
    }
    if router_hosts().iter().any(|entry| entry == &host) {
        return "omniroute";
    }
    if host.starts_with("model-") {
        return "local";
    }
    "other"
}

/// Hosts that are this appliance's own router. Configured, because the address is
/// a deployment fact and not a property of the code.
fn router_hosts() -> Vec<String> {
    std::env::var("NESTBOX_ROUTER_HOSTS")
        .unwrap_or_else(|_| "10.90.92.30".to_string())
        .split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// The agent behind a user agent.
pub fn agent_from_user_agent(user_agent: &str) -> &'static str {
    let value = user_agent.to_ascii_lowercase();
    let table: [(&str, &str); 9] = [
        ("claude-cli", "claude-code"),
        ("claude-code", "claude-code"),
        ("codex", "codex"),
        ("openchamber", "openchamber"),
        ("opencode", "opencode"),
        ("hermes", "hermes"),
        ("curl/", "curl"),
        ("python", "sdk-python"),
        ("node", "sdk-js"),
    ];
    for (needle, agent) in table {
        if value.contains(needle) {
            return agent;
        }
    }
    "unknown"
}

fn string_at(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_string)
}

fn integer_at(value: &serde_json::Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64().or_else(|| current.as_f64().map(|number| number as i64))
}

/// The first `"key":"value"` in a text, for a body that does not parse.
fn json_string_field(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote = rest.find('"')?;
    let rest = &rest[quote + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// `session_<uuid>` inside `metadata.user_id`, which is itself a JSON string.
fn session_from_metadata(value: &serde_json::Value) -> Option<String> {
    let user = value.get("metadata")?.get("user_id")?.as_str()?;
    session_of(user)
}

fn session_of(text: &str) -> Option<String> {
    let marker = "session_";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest
        .find(|character: char| !character.is_ascii_hexdigit() && character != '-')
        .unwrap_or(rest.len());
    let candidate = &rest[..end];
    if candidate.len() >= 8 {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// The text of the last user message.
fn last_user_text(value: &serde_json::Value) -> String {
    if let Some(messages) = value.get("messages").and_then(|messages| messages.as_array()) {
        if let Some(message) = messages
            .iter()
            .rev()
            .find(|message| message.get("role").and_then(|role| role.as_str()) == Some("user"))
        {
            return content_text(message.get("content"));
        }
    }
    if let Some(input) = value.get("input") {
        if let Some(text) = input.as_str() {
            return text.to_string();
        }
        if let Some(items) = input.as_array() {
            for item in items.iter().rev() {
                if item.get("role").and_then(|role| role.as_str()) == Some("user") {
                    return content_text(item.get("content").or(Some(item)));
                }
            }
        }
    }
    String::new()
}

/// The readable text of a content value: a string, or blocks of text.
fn content_text(content: Option<&serde_json::Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let Some(blocks) = content.as_array() else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    let mut saw_tool_result = false;
    for block in blocks {
        match block.get("type").and_then(|kind| kind.as_str()) {
            Some("text") | Some("input_text") | Some("output_text") => {
                if let Some(text) = block.get("text").and_then(|text| text.as_str()) {
                    parts.push(text.to_string());
                }
            }
            Some("tool_result") => saw_tool_result = true,
            _ => {}
        }
    }
    if parts.is_empty() && saw_tool_result {
        return "[tool result]".to_string();
    }
    parts.join("\n")
}

/// Read the answer, JSON or streamed.
fn read_response(response: &str, facts: &mut TurnFacts) {
    let trimmed = response.trim_start();
    if trimmed.starts_with("event:") || trimmed.starts_with("data:") || trimmed.contains("\ndata:") {
        read_stream(response, facts);
        return;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(response) {
        read_answer(&value, facts);
    }
}

fn read_stream(stream: &str, facts: &mut TurnFacts) {
    let mut answer = String::new();
    let mut tools: Vec<ToolCall> = Vec::new();
    for line in stream.lines() {
        let line = line.trim_start();
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(payload) else {
            continue;
        };
        match event.get("type").and_then(|kind| kind.as_str()) {
            // Anthropic
            Some("message_start") => {
                facts.served_model = facts
                    .served_model
                    .clone()
                    .or_else(|| string_at(&event, &["message", "model"]));
                facts.input_tokens = facts
                    .input_tokens
                    .or_else(|| integer_at(&event, &["message", "usage", "input_tokens"]));
                facts.cache_read = facts
                    .cache_read
                    .or_else(|| integer_at(&event, &["message", "usage", "cache_read_input_tokens"]));
                facts.cache_write = facts.cache_write.or_else(|| {
                    integer_at(&event, &["message", "usage", "cache_creation_input_tokens"])
                });
            }
            Some("content_block_start") => {
                if let Some(name) = string_at(&event, &["content_block", "name"]) {
                    tools.push(ToolCall { name, input: String::new() });
                }
            }
            Some("content_block_delta") => {
                if let Some(text) = string_at(&event, &["delta", "text"]) {
                    answer.push_str(&text);
                }
                if let Some(fragment) = string_at(&event, &["delta", "partial_json"]) {
                    if let Some(last) = tools.last_mut() {
                        last.input.push_str(&fragment);
                    }
                }
            }
            Some("message_delta") => {
                facts.output_tokens = facts
                    .output_tokens
                    .or_else(|| integer_at(&event, &["usage", "output_tokens"]));
            }
            // OpenAI Responses
            Some("response.output_text.delta") => {
                if let Some(text) = event.get("delta").and_then(|delta| delta.as_str()) {
                    answer.push_str(text);
                }
            }
            Some("response.completed") => {
                facts.served_model = facts
                    .served_model
                    .clone()
                    .or_else(|| string_at(&event, &["response", "model"]));
                facts.input_tokens = facts
                    .input_tokens
                    .or_else(|| integer_at(&event, &["response", "usage", "input_tokens"]));
                facts.output_tokens = facts
                    .output_tokens
                    .or_else(|| integer_at(&event, &["response", "usage", "output_tokens"]));
            }
            _ => {}
        }
        // OpenAI Chat: `choices[0].delta`.
        if let Some(choices) = event.get("choices").and_then(|choices| choices.as_array()) {
            if let Some(choice) = choices.first() {
                if facts.served_model.is_none() {
                    facts.served_model = string_at(&event, &["model"]);
                }
                if let Some(text) = string_at(choice, &["delta", "content"]) {
                    answer.push_str(&text);
                }
                if let Some(calls) = choice
                    .get("delta")
                    .and_then(|delta| delta.get("tool_calls"))
                    .and_then(|calls| calls.as_array())
                {
                    for call in calls {
                        let name = string_at(call, &["function", "name"]).unwrap_or_default();
                        let arguments = string_at(call, &["function", "arguments"]).unwrap_or_default();
                        if let Some(existing) = tools.iter_mut().find(|tool| tool.name == name) {
                            existing.input.push_str(&arguments);
                        } else if !name.is_empty() {
                            tools.push(ToolCall { name, input: arguments });
                        }
                    }
                }
                if facts.output_tokens.is_none() {
                    facts.output_tokens = integer_at(&event, &["usage", "completion_tokens"]);
                }
                if facts.input_tokens.is_none() {
                    facts.input_tokens = integer_at(&event, &["usage", "prompt_tokens"]);
                }
            }
        }
    }
    facts.assistant_text = answer;
    facts.tool_calls = finish_tools(tools);
}

/// Read a complete JSON answer.
fn read_answer(value: &serde_json::Value, facts: &mut TurnFacts) {
    facts.served_model = facts
        .served_model
        .clone()
        .or_else(|| string_at(value, &["model"]));
    facts.input_tokens = facts
        .input_tokens
        .or_else(|| integer_at(value, &["usage", "input_tokens"]))
        .or_else(|| integer_at(value, &["usage", "prompt_tokens"]));
    facts.output_tokens = facts
        .output_tokens
        .or_else(|| integer_at(value, &["usage", "output_tokens"]))
        .or_else(|| integer_at(value, &["usage", "completion_tokens"]));
    facts.cache_read = facts.cache_read.or_else(|| {
        integer_at(value, &["usage", "cache_read_input_tokens"])
            .or_else(|| integer_at(value, &["usage", "prompt_tokens_details", "cached_tokens"]))
    });
    facts.cache_write = facts
        .cache_write
        .or_else(|| integer_at(value, &["usage", "cache_creation_input_tokens"]));

    // Anthropic: content blocks at the top level.
    if let Some(blocks) = value.get("content").and_then(|content| content.as_array()) {
        let mut answer = String::new();
        let mut tools: Vec<ToolCall> = Vec::new();
        for block in blocks {
            match block.get("type").and_then(|kind| kind.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|text| text.as_str()) {
                        answer.push_str(text);
                    }
                }
                Some("tool_use") => tools.push(ToolCall {
                    name: string_at(block, &["name"]).unwrap_or_default(),
                    input: block
                        .get("input")
                        .map(|input| truncate_chars(&input.to_string(), TOOL_INPUT_MAX))
                        .unwrap_or_default(),
                }),
                _ => {}
            }
        }
        if !answer.is_empty() {
            facts.assistant_text = answer;
        }
        facts.tool_calls = finish_tools(tools);
    }

    // OpenAI chat: choices[0].message.
    if let Some(message) = value
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
    {
        if let Some(text) = message.get("content").and_then(|content| content.as_str()) {
            facts.assistant_text = text.to_string();
        }
        if let Some(calls) = message.get("tool_calls").and_then(|calls| calls.as_array()) {
            facts.tool_calls = finish_tools(
                calls
                    .iter()
                    .map(|call| ToolCall {
                        name: string_at(call, &["function", "name"]).unwrap_or_default(),
                        input: string_at(call, &["function", "arguments"])
                            .map(|arguments| truncate_chars(&arguments, TOOL_INPUT_MAX))
                            .unwrap_or_default(),
                    })
                    .collect(),
            );
        }
    }

    // OpenAI responses: output[].content[].
    if facts.assistant_text.is_empty() {
        if let Some(items) = value.get("output").and_then(|output| output.as_array()) {
            let mut answer = String::new();
            for item in items {
                if let Some(blocks) = item.get("content").and_then(|content| content.as_array()) {
                    for block in blocks {
                        if let Some(text) = block.get("text").and_then(|text| text.as_str()) {
                            answer.push_str(text);
                        }
                    }
                }
            }
            facts.assistant_text = answer;
        }
    }
}

fn finish_tools(tools: Vec<ToolCall>) -> Vec<ToolCall> {
    tools
        .into_iter()
        .filter(|tool| !tool.name.is_empty())
        .map(|tool| ToolCall {
            name: tool.name,
            input: truncate_chars(&tool.input, TOOL_INPUT_MAX),
        })
        .take(20)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_request_and_answer() {
        let request = r#"{"model":"claude-x","messages":[{"role":"user","content":"Hallo Welt"},{"role":"assistant","content":"Hi"},{"role":"user","content":"Und noch was"}]}"#;
        let response = r#"{"model":"claude-x-20260101","content":[{"type":"text","text":"Klar."}],"usage":{"input_tokens":12,"output_tokens":3}}"#;
        let facts = turn_from(request, response);
        assert_eq!(facts.requested_model.as_deref(), Some("claude-x"));
        assert_eq!(facts.served_model.as_deref(), Some("claude-x-20260101"));
        assert_eq!(facts.user_text, "Und noch was");
        assert_eq!(facts.assistant_text, "Klar.");
        assert_eq!(facts.input_tokens, Some(12));
        assert_eq!(facts.output_tokens, Some(3));
    }

    #[test]
    fn a_streamed_anthropic_answer_with_a_tool_call() {
        let request = r#"{"model":"claude-x","messages":[{"role":"user","content":[{"type":"text","text":"Mach was"}]}]}"#;
        let response = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"served-x\",\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":80}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Ich \"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"mache.\"}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"name\":\"write\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"a.txt\\\"}\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
            "data: [DONE]\n\n"
        );
        let facts = turn_from(request, response);
        assert_eq!(facts.user_text, "Mach was");
        assert_eq!(facts.assistant_text, "Ich mache.");
        assert_eq!(facts.served_model.as_deref(), Some("served-x"));
        assert_eq!(facts.input_tokens, Some(100));
        assert_eq!(facts.cache_read, Some(80));
        assert_eq!(facts.output_tokens, Some(7));
        assert_eq!(facts.tool_calls.len(), 1);
        assert_eq!(facts.tool_calls[0].name, "write");
        assert_eq!(facts.tool_calls[0].input, r#"{"path":"a.txt"}"#);
    }

    #[test]
    fn a_streamed_openai_answer() {
        let response = concat!(
            "data: {\"model\":\"gpt-x\",\"choices\":[{\"delta\":{\"content\":\"Hal\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        let facts = turn_from(r#"{"model":"gpt-x","messages":[{"role":"user","content":"Hi"}]}"#, response);
        assert_eq!(facts.assistant_text, "Hallo");
        assert_eq!(facts.served_model.as_deref(), Some("gpt-x"));
        assert_eq!(facts.input_tokens, Some(5));
        assert_eq!(facts.output_tokens, Some(2));
    }

    #[test]
    fn a_cut_request_still_yields_the_model_and_the_question() {
        let request = r#"{"model":"claude-x","messages":[{"role":"user","content":"Bitte prüfe das"}"#;
        let facts = turn_from(request, "");
        assert_eq!(facts.requested_model.as_deref(), Some("claude-x"));
        assert_eq!(facts.user_text, "Bitte prüfe das");
    }

    #[test]
    fn counting_tokens_is_not_a_model_call() {
        assert!(!is_model_call("POST", "/v1/messages/count_tokens"));
        assert!(is_model_call("POST", "/v1/messages?beta=true"));
        assert!(is_model_call("POST", "/backend-api/codex/responses"));
        assert!(!is_model_call("GET", "/v1/models"));
    }

    #[test]
    fn a_tool_result_is_named_as_such() {
        let request = r#"{"model":"m","messages":[{"role":"user","content":[{"type":"tool_result","content":"x"}]}]}"#;
        assert_eq!(turn_from(request, "").user_text, "[tool result]");
    }

    #[test]
    fn the_session_comes_from_the_metadata() {
        let request = r#"{"model":"m","metadata":{"user_id":"user_abc_session_1234abcd-0000-1111-2222-333344445555"},"messages":[]}"#;
        assert_eq!(
            turn_from(request, "").session_id.as_deref(),
            Some("1234abcd-0000-1111-2222-333344445555")
        );
    }

    #[test]
    fn providers_are_recognised() {
        assert_eq!(provider_for("api.anthropic.com:443"), "anthropic");
        assert_eq!(provider_for("chatgpt.com"), "openai");
        assert_eq!(provider_for("generativelanguage.googleapis.com"), "google");
        assert_eq!(provider_for("10.90.92.30"), "omniroute");
        assert_eq!(provider_for("evil-anthropic.com"), "other");
    }

    #[test]
    fn agents_are_recognised() {
        assert_eq!(agent_from_user_agent("claude-cli/1.0"), "claude-code");
        assert_eq!(agent_from_user_agent("codex_cli_rs/0.1"), "codex");
        assert_eq!(agent_from_user_agent("OpenCode/1"), "opencode");
        assert_eq!(agent_from_user_agent("curl/8.5.0"), "curl");
    }

    #[test]
    fn truncation_keeps_characters_whole() {
        assert_eq!(truncate_chars("Müller", 2), "Mü");
    }
}
