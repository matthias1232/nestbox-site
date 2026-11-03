//! Content privacy (DLP) for sandbox chat content.
//!
//! # What this layer does, and what it refuses to do
//!
//! A sandbox's chat content is scanned for secrets and personal data before it
//! moves. What happens next is the policy's decision (`domain::privacy`), and
//! what was found is recorded as **a masked excerpt and a SHA-256 fingerprint**
//! — never as the value. That last rule is structural rather than a convention:
//! `privacy_incidents` has no column for the value, so no bug here can store one.
//!
//! # Three decisions worth knowing
//!
//! 1. **The scan is hand-written, not regex.** The control API has no `regex`
//!    dependency, and adding one to the deployment image for ten patterns is a
//!    worse trade than the byte scanners below. Each one validates a checksum
//!    where the format has one (Luhn for cards, mod-97 for IBANs) and requires a
//!    word boundary, so the false-positive rate is a property of the code and
//!    not of the input.
//! 2. **A detector claims its bytes.** A card number is not also a phone number
//!    and an email is not also a hostname: the detectors run in a fixed priority
//!    order and a later one cannot take a span an earlier one claimed. Two
//!    incidents for one secret would double the evidence and the noise.
//! 3. **`review` never refuses.** A reviewer rewrites; only `strict` blocks. The
//!    mode matrix is one function in the domain, and this service never
//!    re-derives it.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use super::access_control::require;
use super::ServiceContext;
use crate::domain::auth::{Permission, Principal};
use crate::domain::privacy::{
    Action, Detector, Finding, IncidentAction, IncidentSource, Marked, Outcome, PrivacyIncident,
    PrivacyMode, PrivacySettings, PrivacyUpdate, apply,
};
use crate::error::{ApiError, ErrorCode};
use crate::store::postgres::privacy as store;
use crate::store::TenantScope;

/// The largest piece of content one inspection accepts.
///
/// 64 KiB is one chat turn with room to spare; a larger body is a file, and a
/// file belongs in the media and attachment paths, where a different policy
/// applies.
const MAX_CONTENT: usize = 64 * 1024;

/// What the proxy hop may send: a whole model request, not a typed paragraph.
///
/// The 64 KiB above is the test button's limit — a person typing into a form.
/// An agent ships its system prompt and tool definitions with every request, so
/// a real one is routinely larger; refusing it turned every inspected request
/// into a 403 and the DLP into an outage (finding A1).
const MAX_PROXY_CONTENT: usize = 8 * 1024 * 1024;

/// How many findings one inspection reports and records.
///
/// Bounded on purpose: a pasted log full of hostnames would otherwise write
/// thousands of incident rows and spend the request in the incident insert loop.
/// The cap is on the *whole* content, so the first matches are the ones recorded
/// — a caller that needs more inspects again with a narrower detector set.
const MAX_FINDINGS: usize = 100;

/// The largest incident page a caller may ask for.
const MAX_INCIDENTS: i64 = 500;

/// The default incident page.
const DEFAULT_INCIDENTS: i64 = 100;

/// Which way the content is travelling.
///
/// Outbound content is what the account is usually worried about (it leaves for
/// a provider); inbound content is what arrived from one. The direction does not
/// change the detectors, only the evidence: whether an incident records a cloud
/// delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Content coming into the sandbox, from a model or an operator.
    Inbound,
    /// Content leaving the sandbox, for a model or a provider.
    Outbound,
}

impl Direction {
    /// The wire and audit spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        }
    }

    /// Read the wire spelling; unknown input is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "inbound" => Some(Self::Inbound),
            "outbound" => Some(Self::Outbound),
            _ => None,
        }
    }
}

/// One finding as the console shows it: what kind, what it looked like, what
/// happened. The masked value is the only trace of the content.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FindingView {
    /// The detector's name, as the settings and the incident row spell it.
    pub detector: String,
    /// First and last characters of the value, stars between.
    pub masked_value: String,
    /// What happened: `blocked`, `redacted`, `replaced` or `allowed`.
    pub action: String,
    /// What the policy asked for. Differs from `action` in `observe`, where the
    /// request went out unchanged although a detector matched.
    pub requested: String,
}

/// The answer to one inspection.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InspectResult {
    /// False means the caller must not send the content on.
    pub permitted: bool,
    /// `allowed`, `replaced` or `blocked`.
    pub action: String,
    /// The content to send instead. Identical to the input when nothing was
    /// rewritten, and the redacted text when the request is blocked — a caller
    /// that ignores `permitted` still never forwards a value.
    pub transformed_content: String,
    pub findings: Vec<FindingView>,
    /// Placeholders to turn back into their values in the answer. Empty for the
    /// test button; only the internal endpoint fills it.
    pub restore: Vec<RestorePair>,
    /// The policy mode that answered: `off`, `observe`, `review` or `strict`.
    ///
    /// The proxy keeps it so that a check it cannot answer later falls back to
    /// the same decision: `observe` may forward, and nothing else may.
    pub mode: String,
    /// What this request was allowed to cost: `enforce`, `record_only` or
    /// `exempt`.
    ///
    /// The broker reads it to decide whether it may change the bytes at all: a
    /// browser's form post is recorded and never rewritten, and a login request
    /// is not read.
    pub enforcement: String,
}

/// The SHA-256 of one matched value, as lowercase hex.
///
/// This is the token the domain's placeholder is built from and the column the
/// incident stores: the same value always yields the same fingerprint, so a
/// repeat leaks the same key is visible, and nothing can be recovered from it.
///
/// Unsalted, so it is only used where no key is available (tests, and a
/// deployment that has not created `privacy-hmac` yet). [`fingerprint_with`] is
/// what the inspection path calls.
pub fn fingerprint(value: &str) -> String {
    fingerprint_with(None, value)
}

/// The fingerprint the inspection stores: an HMAC when a key is given, the plain
/// SHA-256 otherwise.
///
/// The key is what makes the fingerprint useless to anyone who only holds the
/// ledger: without it, a value that is guessable (an email address, a phone
/// number) can be hashed and looked up in `privacy_incidents`. With it, the same
/// guess tells an attacker nothing.
pub fn fingerprint_with(key: Option<&[u8]>, value: &str) -> String {
    match key {
        Some(key) => hmac_sha256(key, value.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        None => Sha256::digest(value.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    }
}

/// The key the incident fingerprints are salted with, read once.
///
/// The file is `privacy-hmac` in the secret directory: 64 hex characters (32
/// bytes). A missing or malformed file is not an error — the layer still works
/// with an unsalted fingerprint — but it warns once, because a silent downgrade
/// of the evidence is exactly what nobody would notice.
static PRIVACY_KEY: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();

/// Guards the one warning: a failed read must not turn into a log line per
/// inspection.
static PRIVACY_KEY_WARNED: std::sync::Once = std::sync::Once::new();

/// The salt, or `None` when the deployment has no usable key file.
///
/// Borrowed from the [`OnceLock`], so an inspection does not copy 32 bytes for a
/// value that never changes after the first read.
fn privacy_key(secret_dir: &std::path::Path) -> Option<&'static [u8]> {
    PRIVACY_KEY
        .get_or_init(|| {
            let path = secret_dir.join("privacy-hmac");
            let Some(bytes) = std::fs::read_to_string(&path).ok().and_then(|raw| hex_bytes(raw.trim()))
            else {
                warn_missing_key(&path);
                return None;
            };
            if bytes.len() != 32 {
                warn_missing_key(&path);
                return None;
            }
            Some(bytes)
        })
        .as_deref()
}

/// The one warning about a missing or malformed salt.
fn warn_missing_key(path: &std::path::Path) {
    PRIVACY_KEY_WARNED.call_once(|| {
        tracing::warn!(
            path = %path.display(),
            "privacy fingerprints are unsalted: the file must hold 64 hex characters"
        );
    });
}

/// Hex to bytes, or `None` when the text is not an even run of hex digits.
fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.chunks(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        out.push(high << 4 | low);
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// HMAC-SHA256, written out because the API has no `hmac` dependency.
///
/// The construction is RFC 2104: pad the key to the block size, hash the inner
/// pad with the message, then hash the outer pad with that digest.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.iter().map(|byte| byte ^ 0x36).collect::<Vec<u8>>());
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.iter().map(|byte| byte ^ 0x5c).collect::<Vec<u8>>());
    outer.update(inner);
    outer.finalize().into()
}

/// A byte span one detector claims in the text.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

/// Whether two spans share at least one byte.
fn overlaps(left: Span, right: Span) -> bool {
    left.start < right.end && right.start < left.end
}

/// The order the detectors run in.
///
/// Specific formats come first: an IBAN contains a long digit run and two
/// letters, a card contains a digit run, an email contains a hostname. Running
/// the specific detector first is what keeps the general one from claiming part
/// of it.
const PRIORITY: [Detector; 10] = [
    Detector::Iban,
    Detector::CreditCards,
    Detector::ApiKeys,
    Detector::Credentials,
    Detector::EmailAddresses,
    Detector::IpAddresses,
    Detector::PhoneNumbers,
    Detector::Hostnames,
    Detector::PersonNames,
    Detector::FinancialData,
];

/// Find every value the given detectors recognise, without ever returning the
/// same value twice.
pub fn detect(text: &str, detectors: &[Detector]) -> Vec<Finding> {
    let bytes = text.as_bytes();
    let mut claimed: Vec<Span> = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();
    for detector in PRIORITY {
        if !detectors.contains(&detector) {
            continue;
        }
        for span in scan(text, detector) {
            if span.start >= span.end || span.end > bytes.len() {
                continue;
            }
            // A span is only usable when it starts and ends on a character
            // boundary; every scanner works on ASCII, so this is a guard rather
            // than an expectation.
            if !text.is_char_boundary(span.start) || !text.is_char_boundary(span.end) {
                continue;
            }
            if claimed.iter().any(|existing| overlaps(*existing, span)) {
                continue;
            }
            let value = &text[span.start..span.end];
            if findings
                .iter()
                .any(|finding| finding.detector == detector && finding.value == value)
            {
                claimed.push(span);
                continue;
            }
            claimed.push(span);
            findings.push(Finding::new(detector, value));
            if findings.len() >= MAX_FINDINGS {
                return findings;
            }
        }
    }
    findings
}

/// Run one detector's scanner.
fn scan(text: &str, detector: Detector) -> Vec<Span> {
    let bytes = text.as_bytes();
    match detector {
        Detector::Credentials => scan_credentials(text, bytes),
        Detector::ApiKeys => scan_api_keys(text, bytes),
        Detector::EmailAddresses => scan_emails(bytes),
        Detector::PersonNames => scan_person_names(bytes),
        Detector::IpAddresses => scan_ipv4(bytes),
        Detector::Hostnames => scan_hostnames(bytes),
        Detector::PhoneNumbers => scan_phones(bytes),
        Detector::Iban => scan_ibans(bytes),
        Detector::CreditCards => scan_cards(bytes),
        Detector::FinancialData => scan_financial(text),
        // Evidence-only kinds: the proxy reports them, no detector looks for
        // them. They find nothing, and saying so here keeps the match exhaustive.
        Detector::CheckUnavailable | Detector::Oversize => Vec::new(),
    }
}

fn is_alnum(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

/// A word boundary on the left: the value does not continue a word.
fn boundary_before(bytes: &[u8], index: usize) -> bool {
    index == 0 || !is_alnum(bytes[index - 1])
}

/// A word boundary on the right.
fn boundary_after(bytes: &[u8], end: usize) -> bool {
    end >= bytes.len() || !is_alnum(bytes[end])
}

fn is_local_part_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-')
}

fn is_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

/// `local@domain.tld`, with a real TLD and a boundary on both sides.
fn scan_emails(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'@' {
            continue;
        }
        let mut start = index;
        while start > 0 && is_local_part_byte(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = index + 1;
        while end < bytes.len() && is_domain_byte(bytes[end]) {
            end += 1;
        }
        while end > index + 1 && matches!(bytes[end - 1], b'.' | b'-' | b'_') {
            end -= 1;
        }
        if start == index || end <= index + 1 {
            continue;
        }
        let domain = &bytes[index + 1..end];
        let Some(dot) = domain.iter().rposition(|candidate| *candidate == b'.') else {
            continue;
        };
        if dot == 0 {
            continue;
        }
        let label = &domain[dot + 1..];
        if label.len() < 2 || label.len() > 24 || !label.iter().all(u8::is_ascii_alphabetic) {
            continue;
        }
        spans.push(Span { start, end });
    }
    spans
}

/// The end of an IPv4 address starting at `start`, or `None`.
fn ipv4_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut position = start;
    for octet in 0..4 {
        let begin = position;
        let mut value: u32 = 0;
        while position < bytes.len() && bytes[position].is_ascii_digit() && position - begin < 3 {
            value = value * 10 + u32::from(bytes[position] - b'0');
            position += 1;
        }
        if position == begin || value > 255 {
            return None;
        }
        if octet < 3 {
            if position >= bytes.len() || bytes[position] != b'.' {
                return None;
            }
            position += 1;
        }
    }
    // A trailing digit or dot means a longer dotted run, not an address.
    if position < bytes.len() && (bytes[position].is_ascii_alphanumeric() || bytes[position] == b'.')
    {
        return None;
    }
    Some(position)
}

/// Four octets, each 0–255, at a word boundary.
fn scan_ipv4(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() || !boundary_before(bytes, index) {
            index += 1;
            continue;
        }
        match ipv4_end(bytes, index) {
            Some(end) => {
                spans.push(Span { start: index, end });
                index = end;
            }
            None => {
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }
    }
    spans
}

/// Whether an IBAN's mod-97 checksum matches.
///
/// The check is what separates an account number from "DE89" followed by a word:
/// the algorithm moves the country and check digits to the end, maps letters to
/// 10–35, and requires the whole number to be 1 modulo 97.
fn iban_checksum_ok(candidate: &[u8]) -> bool {
    let mut remainder: u32 = 0;
    for byte in candidate[4..].iter().chain(candidate[..4].iter()) {
        if byte.is_ascii_digit() {
            remainder = (remainder * 10 + u32::from(byte - b'0')) % 97;
        } else if byte.is_ascii_uppercase() {
            let value = u32::from(byte - b'A') + 10;
            remainder = (remainder * 100 + value) % 97;
        } else {
            return false;
        }
    }
    remainder == 1
}

/// A country code, two check digits, and 11–30 more characters that pass the
/// mod-97 check.
///
/// Grouped IBANs (`DE89 3704 …`) are not matched: a space inside the number
/// makes it indistinguishable from two numbers in a sentence, and a false
/// positive here refuses a legitimate request.
fn scan_ibans(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if !bytes[index].is_ascii_uppercase()
            || !boundary_before(bytes, index)
            || !bytes[index + 1].is_ascii_uppercase()
            || !bytes[index + 2].is_ascii_digit()
            || !bytes[index + 3].is_ascii_digit()
        {
            index += 1;
            continue;
        }
        let mut end = index + 4;
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() && end - index < 34 {
            end += 1;
        }
        let length = end - index;
        if length < 15
            || !boundary_after(bytes, end)
            || !iban_checksum_ok(&bytes[index..end])
        {
            index += 1;
            continue;
        }
        spans.push(Span { start: index, end });
        index = end;
    }
    spans
}

/// Whether a card number satisfies the Luhn check.
///
/// Separators are skipped, so `4111 1111 1111 1111` and `4111111111111111` agree.
fn luhn_ok(candidate: &[u8]) -> bool {
    let mut sum: u32 = 0;
    let mut double = false;
    for byte in candidate.iter().rev() {
        if !byte.is_ascii_digit() {
            continue;
        }
        let mut value = u32::from(byte - b'0');
        if double {
            value *= 2;
            if value > 9 {
                value -= 9;
            }
        }
        sum += value;
        double = !double;
    }
    sum > 0 && sum % 10 == 0
}

/// 13–19 digits, optionally grouped, that pass the Luhn check.
fn scan_cards(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() || !boundary_before(bytes, index) {
            index += 1;
            continue;
        }
        let start = index;
        let mut position = index;
        let mut digits = 0usize;
        let mut last_digit_end = index;
        let mut previous_was_digit = false;
        while position < bytes.len() {
            let byte = bytes[position];
            if byte.is_ascii_digit() {
                digits += 1;
                position += 1;
                last_digit_end = position;
                previous_was_digit = true;
                if digits > 19 {
                    break;
                }
            } else if matches!(byte, b' ' | b'-')
                && previous_was_digit
                && position + 1 < bytes.len()
                && bytes[position + 1].is_ascii_digit()
            {
                position += 1;
                previous_was_digit = false;
            } else {
                break;
            }
        }
        let end = last_digit_end;
        if (13..=19).contains(&digits)
            && boundary_after(bytes, end)
            && luhn_ok(&bytes[start..end])
        {
            spans.push(Span { start, end });
            index = end;
        } else {
            index = start + 1;
        }
    }
    spans
}

/// A phone number: 7–15 digits, and a shape a bare number does not have.
///
/// Either a leading `+` or at least one separator is required. A plain ten-digit
/// run is how an account number, a build id and a timestamp look too, and a
/// detector that flags all of those is turned off within a week.
fn scan_phones(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let starts = (first.is_ascii_digit() || first == b'+' || first == b'(')
            && boundary_before(bytes, index);
        if !starts {
            index += 1;
            continue;
        }
        let start = index;
        let mut position = index;
        let mut digits = 0usize;
        let mut separators = 0usize;
        let mut last_digit_end = index;
        let leading_plus = first == b'+';
        if leading_plus {
            position += 1;
            if position >= bytes.len()
                || !(bytes[position].is_ascii_digit() || bytes[position] == b'(')
            {
                index += 1;
                continue;
            }
        }
        while position < bytes.len() {
            let byte = bytes[position];
            if byte.is_ascii_digit() {
                digits += 1;
                position += 1;
                last_digit_end = position;
                if digits > 15 {
                    break;
                }
            } else if matches!(byte, b' ' | b'-' | b'.')
                && position + 1 < bytes.len()
                && bytes[position + 1].is_ascii_digit()
            {
                separators += 1;
                position += 1;
            } else if matches!(byte, b'(' | b')') {
                separators += 1;
                position += 1;
            } else {
                break;
            }
        }
        let end = last_digit_end;
        if (7..=15).contains(&digits)
            && (leading_plus || separators > 0)
            && boundary_after(bytes, end)
        {
            spans.push(Span { start, end });
            index = end;
        } else {
            index = start + 1;
        }
    }
    spans
}

/// File extensions that look like a TLD and are not one.
///
/// Without this list every `report.txt` and `main.rs` in a pasted log becomes a
/// hostname. The list is a heuristic with a cost: an environment that really
/// talks to `.io`-style new TLDs is unaffected, one that talks to `.local`
/// internal names gets a match, and one that pastes `notes.md` does not.
const FILE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "json", "txt", "md", "yaml", "yml", "xml",
    "html", "htm", "pdf", "zip", "tar", "gz", "csv", "tsv", "log", "rs", "py", "js", "ts", "sh",
    "so", "dll", "exe", "bin", "img", "iso", "mp4", "webm", "wav", "mp3", "toml", "lock", "sql",
    "env", "cfg", "conf", "ini", "bak", "tmp", "dat",
];

fn is_host_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

/// Whether a dotted run is shaped like a hostname: two to eight labels, an
/// alphabetic last label, and not a dotted quad or a file name.
fn hostname_shape_ok(candidate: &[u8]) -> bool {
    let labels: Vec<&[u8]> = candidate.split(|byte| *byte == b'.').collect();
    if labels.len() < 2 || labels.len() > 8 {
        return false;
    }
    if labels
        .iter()
        .any(|label| label.is_empty() || label.len() > 63)
    {
        return false;
    }
    if labels
        .iter()
        .all(|label| label.iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    if labels
        .iter()
        .any(|label| label[0] == b'-' || label[label.len() - 1] == b'-')
    {
        return false;
    }
    let Some(tld) = labels.last() else {
        return false;
    };
    if tld.len() < 2 || tld.len() > 24 || !tld.iter().all(u8::is_ascii_alphabetic) {
        return false;
    }
    let tld = String::from_utf8_lossy(tld).to_ascii_lowercase();
    !FILE_EXTENSIONS.contains(&tld.as_str())
}

/// A dotted name with a plausible last label.
fn scan_hostnames(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !is_host_byte(bytes[index]) || !boundary_before(bytes, index) {
            index += 1;
            continue;
        }
        let start = index;
        let mut end = index;
        while end < bytes.len() && is_host_byte(bytes[end]) {
            end += 1;
        }
        while end > start && matches!(bytes[end - 1], b'.' | b'-') {
            end -= 1;
        }
        if end > start && hostname_shape_ok(&bytes[start..end]) && boundary_after(bytes, end) {
            spans.push(Span { start, end });
            index = end;
        } else {
            index = start + 1;
        }
    }
    spans
}

/// Words that are capitalised for grammar rather than because they are a name.
const NAME_STOPWORDS: &[&str] = &[
    "The", "This", "That", "These", "Those", "There", "Then", "Dear", "Hello", "Hi", "Hey",
    "Regards", "Thanks", "Thank", "Note", "Warning", "Error", "Summary", "Chapter", "Section",
    "Step", "Example", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
    "Sunday", "January", "February", "March", "April", "May", "June", "July", "August",
    "September", "October", "November", "December",
];

/// Every run of alphabetic characters, with its span.
fn words_of(bytes: &[u8]) -> Vec<Span> {
    let mut words = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphabetic() || bytes[index] == b'\'')
        {
            index += 1;
        }
        words.push(Span { start, end: index });
    }
    words
}

/// `Marcus`, not `MARCUS` and not `marcus`.
fn is_capitalised_word(word: &[u8]) -> bool {
    if word.len() < 2 || word.len() > 24 {
        return false;
    }
    if !word[0].is_ascii_uppercase() {
        return false;
    }
    word[1..].iter().all(u8::is_ascii_lowercase)
}

/// Two capitalised words separated by one space.
///
/// This is the weakest detector and it says so: names have no checksum, and the
/// alternative — a model call — is a dependency this layer does not have. The
/// stopwords and the strict capitalisation rule are what keep it from flagging
/// every heading.
fn scan_person_names(bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let words = words_of(bytes);
    for pair in words.windows(2) {
        let first = pair[0];
        let second = pair[1];
        if &bytes[first.end..second.start] != b" " {
            continue;
        }
        let left = &bytes[first.start..first.end];
        let right = &bytes[second.start..second.end];
        if !is_capitalised_word(left) || !is_capitalised_word(right) {
            continue;
        }
        if NAME_STOPWORDS.iter().any(|word| word.as_bytes() == left)
            || NAME_STOPWORDS.iter().any(|word| word.as_bytes() == right)
        {
            continue;
        }
        spans.push(Span {
            start: first.start,
            end: second.end,
        });
    }
    spans
}

/// The currency symbols and codes an amount is written with.
const CURRENCY_SYMBOLS: [char; 4] = ['$', '€', '£', '¥'];
const CURRENCY_CODES: &[&str] = &["USD", "EUR", "GBP", "CHF", "JPY", "CAD", "AUD"];

/// The end of an amount: digits with thousands separators and a decimal part.
fn amount_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut position = start;
    let mut digits = 0usize;
    while position < bytes.len() {
        if bytes[position].is_ascii_digit() {
            digits += 1;
            position += 1;
        } else if matches!(bytes[position], b',' | b'.')
            && position + 1 < bytes.len()
            && bytes[position + 1].is_ascii_digit()
        {
            position += 1;
        } else {
            break;
        }
    }
    if digits == 0 {
        None
    } else {
        Some(position)
    }
}

/// A money amount: `$1,240.00`, `€900`, `1200 USD`.
fn scan_financial(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let offsets: Vec<(usize, char)> = text.char_indices().collect();
    for (index, (position, character)) in offsets.iter().enumerate() {
        if !CURRENCY_SYMBOLS.contains(character) {
            continue;
        }
        let Some((next, _)) = offsets.get(index + 1) else {
            continue;
        };
        if !bytes[*next].is_ascii_digit() {
            continue;
        }
        if let Some(end) = amount_end(bytes, *next) {
            spans.push(Span {
                start: *position,
                end,
            });
        }
    }
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() || !boundary_before(bytes, index) {
            index += 1;
            continue;
        }
        let Some(end) = amount_end(bytes, index) else {
            index += 1;
            continue;
        };
        let mut position = end;
        if position < bytes.len() && bytes[position] == b' ' {
            position += 1;
        }
        if position + 3 <= bytes.len() {
            let code = &bytes[position..position + 3];
            if CURRENCY_CODES.iter().any(|known| known.as_bytes() == code)
                && boundary_after(bytes, position + 3)
            {
                spans.push(Span {
                    start: index,
                    end: position + 3,
                });
            }
        }
        index = end;
    }
    spans
}

/// The keywords that introduce an assigned secret.
const CREDENTIAL_KEYWORDS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "client_secret",
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "private_key",
    "token",
];

/// The characters a secret value may contain. Quotes, `&`, `;` and whitespace
/// end it, so a query string or a JSON body stops at its own delimiter.
fn is_secret_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'_' | b'-' | b'.' | b'!' | b'@' | b'#' | b'$' | b'%' | b'^' | b'*' | b'+' | b'/'
                | b'~' | b'?'
        )
}

/// The value of a `key = value` / `key: value` assignment, if there is one.
fn assignment_value(bytes: &[u8], mut position: usize) -> Option<Span> {
    while position < bytes.len() && bytes[position] == b' ' {
        position += 1;
    }
    if position >= bytes.len() || !matches!(bytes[position], b'=' | b':') {
        return None;
    }
    position += 1;
    while position < bytes.len()
        && matches!(bytes[position], b' ' | b'"' | b'\'' | b'`')
    {
        position += 1;
    }
    let start = position;
    while position < bytes.len() && is_secret_byte(bytes[position]) {
        position += 1;
    }
    if position - start < 4 {
        return None;
    }
    Some(Span { start, end: position })
}

/// A PEM private-key block, bounded so a huge paste cannot become one finding.
fn pem_block(text: &str) -> Option<Span> {
    let mut search = 0;
    while search < text.len() {
        let found = text[search..].find("-----BEGIN ")?;
        let start = search + found;
        let header_end = text[start..]
            .find('\n')
            .map(|offset| start + offset)
            .unwrap_or(text.len());
        let header = &text[start..header_end];
        if header.contains("PRIVATE KEY") {
            let body_start = (header_end + 1).min(text.len());
            if let Some(offset) = text[body_start..].find("-----END ") {
                let marker = body_start + offset;
                let line_end = text[marker..]
                    .find('\n')
                    .map(|rest| marker + rest)
                    .unwrap_or(text.len());
                let end = line_end.min(start + 4096);
                if end > start {
                    return Some(Span { start, end });
                }
            }
        }
        search = header_end.max(start + 1);
    }
    None
}

/// An assigned password, token or key, and a PEM private-key block.
///
/// A blocker by default: these are the values that end an incident report.
fn scan_credentials(text: &str, bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    let lowered = text.to_ascii_lowercase();
    for keyword in CREDENTIAL_KEYWORDS {
        for (position, _) in lowered.match_indices(keyword) {
            if !boundary_before(bytes, position) {
                continue;
            }
            let after = position + keyword.len();
            // `passwords=`, `tokens=` and `secretive` are not assignments.
            if after < bytes.len() && is_alnum(bytes[after]) {
                continue;
            }
            if let Some(span) = assignment_value(bytes, after) {
                spans.push(span);
            }
        }
    }
    if let Some(span) = pem_block(text) {
        spans.push(span);
    }
    spans
}

/// The prefixes providers give their keys, so a key is recognised by its shape.
const API_KEY_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "AKIA",
    "ASIA",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "AIza",
    "glpat-",
];

fn is_key_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn is_base64url(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

/// The end of a JSON Web Token's three segments, starting after its `eyJ`.
fn jwt_end(bytes: &[u8], mut position: usize) -> Option<usize> {
    for segment in 0..3 {
        let start = position;
        while position < bytes.len() && is_base64url(bytes[position]) {
            position += 1;
        }
        if position - start < 6 {
            return None;
        }
        if segment == 2 {
            return Some(position);
        }
        if position >= bytes.len() || bytes[position] != b'.' {
            return None;
        }
        position += 1;
    }
    None
}

/// A prefixed provider key, or a JWT.
fn scan_api_keys(text: &str, bytes: &[u8]) -> Vec<Span> {
    let mut spans = Vec::new();
    for prefix in API_KEY_PREFIXES {
        for (position, matched) in text.match_indices(prefix) {
            if !boundary_before(bytes, position) {
                continue;
            }
            let mut end = position + matched.len();
            while end < bytes.len() && is_key_byte(bytes[end]) {
                end += 1;
            }
            if end - (position + matched.len()) < 12 {
                continue;
            }
            spans.push(Span { start: position, end });
        }
    }
    for (position, matched) in text.match_indices("eyJ") {
        if !boundary_before(bytes, position) {
            continue;
        }
        if let Some(end) = jwt_end(bytes, position + matched.len()) {
            spans.push(Span { start: position, end });
        }
    }
    spans
}

/// One incident the evaluation wants written.
struct IncidentDraft {
    detector: Detector,
    action: IncidentAction,
    /// What the policy asked for. Equal to `action` except in `observe`, where
    /// the policy asks for a rewrite and the request goes out unchanged.
    requested_action: IncidentAction,
    masked_value: String,
    value_fingerprint: String,
    /// The substitute written into the body (`<IBAN_faf7e1c0>`, `[REDACTED]`).
    replacement: String,
    cloud_delivery: bool,
    /// The exception that allowed this finding, when one did.
    exception_id: Option<Uuid>,
}

/// The whole answer of one inspection, before anything is written.
struct Evaluation {
    outcome: Outcome,
    incidents: Vec<IncidentDraft>,
    /// The mode in force, carried so the answer can tell the proxy what to do
    /// when it cannot ask again.
    mode: PrivacyMode,
    /// Placeholders to turn back into their values in the answer, for
    /// `pseudonymize`. Only the internal endpoint's answer carries them.
    restore: Vec<RestorePair>,
}

/// One placeholder and the value it stands for.
///
/// Lives only for the duration of one request: the proxy keeps it in memory and
/// never writes it down. `Debug` is written by hand for that reason.
#[derive(Clone, Serialize, ToSchema)]
pub struct RestorePair {
    /// The placeholder the model saw.
    pub placeholder: String,
    /// The value it stands for.
    pub value: String,
}

impl std::fmt::Debug for RestorePair {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RestorePair")
            .field("placeholder", &self.placeholder)
            .field("value", &"[redacted]")
            .finish()
    }
}

/// Whether the content counts as a delivery to a cloud destination.
///
/// Three cases, and each is a decision:
///
/// * a blocked request delivered nothing, so `false`;
/// * `observe` delivered the content unchanged, so `true` — the incident exists
///   precisely to record that the payload left as it was;
/// * `review` and `strict` deliver a rewritten payload, so the destination and
///   the direction decide.
fn cloud_delivery(
    mode: PrivacyMode,
    direction: Direction,
    destination: Option<&str>,
    blocked: bool,
) -> bool {
    if blocked {
        return false;
    }
    match mode {
        PrivacyMode::Off => false,
        PrivacyMode::Observe => true,
        PrivacyMode::Review | PrivacyMode::Strict => {
            direction == Direction::Outbound && destination.is_some()
        }
    }
}

/// The findings of one text, each with its salted token and any exception that
/// covers it.
///
/// An exception sees only what it was granted for: the detector, the fingerprint
/// of the value and the destination. Its action rides along on the [`Marked`]
/// entry so `apply` does not second-guess it.
fn mark_findings(
    content: &str,
    detectors: &[Detector],
    key: Option<&[u8]>,
    exceptions: &[crate::domain::privacy_exception::PrivacyException],
    sandbox_id: Uuid,
    destination: Option<&str>,
) -> Vec<Marked> {
    let now = Utc::now();
    detect(content, detectors)
        .into_iter()
        .map(|finding| {
            let token = fingerprint_with(key, &finding.value);
            let mut marked = Marked::new(finding, token);
            if !exceptions.is_empty() {
                let match_key = crate::domain::privacy_exception::FindingKey {
                    sandbox_id,
                    detector: marked.finding.detector,
                    fingerprint: &marked.token,
                    destination,
                };
                if let Some(exception) =
                    crate::domain::privacy_exception::best_match(exceptions, &match_key, now)
                {
                    marked = marked.with_exception(exception.action, exception.id);
                }
            }
            marked
        })
        .collect()
}

/// Scan and decide, without touching the database.
///
/// Pure so the whole mode matrix is a unit test, and so a caller can preview
/// what a policy would do to a document before saving it. `exceptions` are the
/// rows the caller already loaded ([`crate::services::privacy_exceptions::active_for`]);
/// the function only matches them, it does not read them.
#[allow(clippy::too_many_arguments)]
fn evaluate(
    settings: &PrivacySettings,
    direction: Direction,
    destination: Option<&str>,
    content: &str,
    key: Option<&[u8]>,
    exceptions: &[crate::domain::privacy_exception::PrivacyException],
    sandbox_id: Uuid,
) -> Evaluation {
    if settings.mode == PrivacyMode::Off {
        return Evaluation {
            outcome: Outcome {
                action: Action::Allow,
                permitted: true,
                transformed: content.to_string(),
                decisions: Vec::new(),
            },
            incidents: Vec::new(),
            mode: settings.mode,
            restore: Vec::new(),
        };
    }

    let detectors = settings.enabled_detectors();
    let marked = mark_findings(content, &detectors, key, exceptions, sandbox_id, destination);
    let outcome = apply(settings, &marked, content);
    let delivered = cloud_delivery(
        settings.mode,
        direction,
        destination,
        outcome.action == Action::Block,
    );
    let incidents = outcome
        .decisions
        .iter()
        .map(|decision| IncidentDraft {
            detector: decision.detector,
            action: decision.incident_action(),
            requested_action: decision.action.incident_action(),
            masked_value: decision.masked_value.clone(),
            value_fingerprint: decision.token.clone(),
            replacement: decision.replacement(),
            cloud_delivery: delivered,
            exception_id: decision.exception_id,
        })
        .collect();

    // The values behind the placeholders, for the hop that has to restore them.
    // Built from `marked`, which still holds the value: nothing else does.
    let restore = outcome
        .decisions
        .iter()
        .zip(marked.iter())
        .filter(|(decision, _)| decision.action == Action::Pseudonymize)
        .map(|(decision, entry)| RestorePair {
            placeholder: decision.replacement(),
            value: entry.finding.value.clone(),
        })
        .collect();

    Evaluation {
        outcome,
        incidents,
        mode: settings.mode,
        restore,
    }
}

/// The settings that apply to a sandbox: its own row, else the environment
/// default, else the built-in strict policy.
///
/// The order is the product promise, and the last step is why an environment
/// that never opened the screen is still protected: absence of a row is the
/// strict policy, not "no policy".
async fn resolve_settings(
    scope: &mut TenantScope<'_>,
    sandbox_id: Option<Uuid>,
) -> Result<PrivacySettings, ApiError> {
    if let Some(sandbox_id) = sandbox_id {
        if let Some(settings) = store::settings_for(scope, Some(sandbox_id))
            .await
            .map_err(ApiError::from)?
        {
            return Ok(settings);
        }
    }
    if let Some(settings) = store::settings_for(scope, None)
        .await
        .map_err(ApiError::from)?
    {
        return Ok(settings);
    }
    Ok(PrivacySettings::strict_default())
}

/// The settings a sandbox would use, and whether they are its own.
///
/// A returned `sandbox_id: None` means the row is the environment default the
/// sandbox inherits; `updated_at: None` means nothing was ever saved and the
/// built-in strict policy applies.
pub async fn effective_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
) -> Result<PrivacySettings, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if let Some(sandbox_id) = sandbox_id {
        require_owned_sandbox(&mut scope, principal, sandbox_id).await?;
    }
    let settings = resolve_settings(&mut scope, sandbox_id).await?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(settings)
}

/// Save the policy for the environment (`sandbox_id: None`) or one sandbox.
///
/// `PolicyWrite`, not a sandbox permission: the environment row is what every
/// sandbox inherits, so writing it is a policy act. The body cannot name the
/// scope or the actor — both come from the path and the principal.
pub async fn set_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
    update: PrivacyUpdate,
) -> Result<PrivacySettings, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    update.validate().map_err(ApiError::validation)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if let Some(sandbox_id) = sandbox_id {
        if !store::sandbox_exists(&mut scope, sandbox_id)
            .await
            .map_err(ApiError::from)?
        {
            return Err(ApiError::not_found(
                ErrorCode::SANDBOX_NOT_FOUND,
                "No such sandbox in this environment.",
            ));
        }
    }
    // What the row said before the write. The history screen shows old and new
    // side by side, and "which detector did I turn off" is unanswerable from the
    // new values alone. `None` means nothing was ever saved, so the built-in
    // strict policy was in force; that is a value, not a gap.
    let previous = store::settings_for(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    let mut settings = update.into_settings(sandbox_id);
    settings.updated_by = Some(principal.user_id);
    store::save_settings(&mut scope, &settings)
        .await
        .map_err(ApiError::from)?;
    let summarize = |value: &PrivacySettings| {
        json!({
            "mode": value.mode.as_str(),
            "detectors": value
                .detectors
                .iter()
                .map(|detector| detector.as_str())
                .collect::<Vec<_>>(),
            "block_cloud_egress": value.block_cloud_egress,
            "retention_days": value.retention_days,
            "overrides": value
                .actions
                .iter()
                .map(|(detector, action)| format!("{}={}", detector.as_str(), action.as_str()))
                .collect::<Vec<_>>(),
        })
    };
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "privacy.settings",
            subject_type: Some(if sandbox_id.is_some() { "sandbox" } else { "environment" }),
            subject_id: Some(sandbox_id.unwrap_or(principal.environment_id)),
            detail: &json!({
                "mode": settings.mode.as_str(),
                "detectors": settings
                .detectors
                .iter()
                .map(|detector| detector.as_str())
                .collect::<Vec<_>>(),
                "block_cloud_egress": settings.block_cloud_egress,
                "retention_days": settings.retention_days,
                "overrides": settings.actions.len(),
                "after": summarize(&settings),
                "before": previous.as_ref().map(summarize),
            }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let stored = store::settings_for(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    stored.ok_or_else(|| ApiError::internal("The privacy settings were not written."))
}

/// Drop a sandbox's override so it inherits the environment default again.
///
/// Returns whether an override existed: "inherit again" is idempotent, but the
/// audit trail should be able to say whether anything changed.
pub async fn clear_settings(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<bool, ApiError> {
    require(principal, Permission::PolicyWrite)?;
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if !store::sandbox_exists(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "No such sandbox in this environment.",
        ));
    }
    let removed = store::delete_override(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "privacy.inherit",
            subject_type: Some("sandbox"),
            subject_id: Some(sandbox_id),
            detail: &json!({ "removed": removed }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(removed)
}

/// Inspect one piece of content and record what it found.
///
/// `TelemetryRead` rather than a sandbox permission: this is the path a gateway
/// calls on behalf of a sandbox's own traffic, and what it returns about the
/// content is evidence, not configuration. The content itself is the caller's —
/// it is inspected and handed back rewritten, never stored.
/// What the request was, for the incident row.
///
/// Everything but `source` is optional because the test button on the settings
/// screen knows none of it: it inspects a typed string, not a request.
#[derive(Debug, Clone, Default)]
pub struct InspectContext {
    /// The host or URL the content was heading to.
    pub destination: Option<String>,
    /// Which hop saw the content.
    pub source: IncidentSource,
    pub method: Option<String>,
    pub path: Option<String>,
    pub model: Option<String>,
    pub mcp_tool: Option<String>,
    /// The conversation the request belonged to, when known.
    pub conversation_id: Option<String>,
    /// The proxy's `x-request-id`, when it sent one. Joins the finding to the
    /// turn and to the live stream.
    pub request_ref: Option<String>,
}

impl InspectContext {
    /// The destination, with a blank string read as "not named".
    fn destination(&self) -> Option<&str> {
        self.destination
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    }
}

/// Run the policy over one piece of text and record what it found.
pub async fn inspect(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    direction: Direction,
    content: &str,
    destination: Option<&str>,
) -> Result<InspectResult, ApiError> {
    let context = InspectContext {
        destination: destination.map(str::to_string),
        ..InspectContext::default()
    };
    inspect_with(ctx, principal, sandbox_id, direction, content, &context).await
}

/// The one implementation: evaluate, record, answer.
///
/// The context is a borrow and does not enter the answer, so the caller — the
/// test button, the accounting proxy, the egress broker — only has to say where
/// the content was going and which hop caught it.
pub async fn inspect_with(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    direction: Direction,
    content: &str,
    context: &InspectContext,
) -> Result<InspectResult, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let limit = if context.source == IncidentSource::Test {
        MAX_CONTENT
    } else {
        MAX_PROXY_CONTENT
    };
    if content.len() > limit {
        return Err(ApiError::validation(format!(
            "Content must be at most {limit} bytes."
        )));
    }

    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if !store::sandbox_exists(&mut scope, sandbox_id)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::not_found(
            ErrorCode::SANDBOX_NOT_FOUND,
            "No such sandbox in this environment.",
        ));
    }
    let settings = resolve_settings(&mut scope, Some(sandbox_id)).await?;
    // The exceptions in force for this sandbox, read once per request: they are
    // a policy input like the settings themselves, and matching them per finding
    // would put a query behind every detector hit.
    let exceptions = crate::services::privacy_exceptions::active_for(&mut scope, sandbox_id).await?;
    let key = privacy_key(&ctx.secret_dir);
    let environment_id = scope.environment_id();
    let destination = context.destination();
    // What this request is allowed to cost: a body check, a record, or nothing.
    // The login paths are exempt before any detector runs, because a refresh
    // token in a credential detector's hands signs the user out.
    let enforcement = crate::domain::privacy::enforcement_for(
        context.source,
        destination,
        context.path.as_deref(),
        &settings.enforce_hosts,
    );
    if enforcement == crate::domain::privacy::Enforcement::Exempt {
        scope.commit().await.map_err(ApiError::from)?;
        return Ok(InspectResult {
            permitted: true,
            action: "skipped".to_string(),
            transformed_content: content.to_string(),
            findings: Vec::new(),
            restore: Vec::new(),
            mode: settings.mode.as_str().to_string(),
            enforcement: "exempt".to_string(),
        });
    }
    // A host that is not a model or an operator's entry is recorded as if the
    // mode were `observe`, whatever the mode says.
    let settings = if enforcement == crate::domain::privacy::Enforcement::RecordOnly {
        PrivacySettings {
            mode: PrivacyMode::Observe,
            ..settings
        }
    } else {
        settings
    };
    // The proxy sends a document, not a sentence: inspect the string values and
    // splice back exactly those, so the request stays byte-identical everywhere a
    // detector found nothing. The test button sends free text and keeps the
    // plain path.
    let evaluation = match context.source {
        IncidentSource::Test | IncidentSource::AccountingProxy => evaluate(
            &settings,
            direction,
            destination,
            content,
            key,
            &exceptions,
            sandbox_id,
        ),
        _ => evaluate_json(
            &settings,
            direction,
            destination,
            content,
            key,
            &exceptions,
            sandbox_id,
        )
        .unwrap_or_else(|| {
            evaluate(
                &settings,
                direction,
                destination,
                content,
                key,
                &exceptions,
                sandbox_id,
            )
        }),
    };
    let mode = settings.mode.as_str();
    let request_ref = context
        .request_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    for incident in &evaluation.incidents {
        store::insert_incident(
            &mut scope,
            store::NewIncident {
                sandbox_id,
                detector: incident.detector,
                action: incident.action,
                severity: incident.action.severity(),
                masked_value: Some(incident.masked_value.as_str()),
                value_fingerprint: Some(incident.value_fingerprint.as_str()),
                destination,
                cloud_delivery: incident.cloud_delivery,
                direction: direction.as_str(),
                requested_action: incident.requested_action,
                mode,
                replacement: Some(incident.replacement.as_str()),
                source: context.source,
                method: context.method.as_deref(),
                path: context.path.as_deref(),
                model: context.model.as_deref(),
                mcp_tool: context.mcp_tool.as_deref(),
                conversation_id: context.conversation_id.as_deref(),
                request_ref,
                exception_id: incident.exception_id,
            },
        )
        .await
        .map_err(ApiError::from)?;
    }
    scope.commit().await.map_err(ApiError::from)?;

    // "A chat started" is a live event, not a stored one: it gives the console
    // something to draw while the answer is still streaming, and it is emitted
    // only for the hops that see a model call (a browser's page load is not a
    // chat). The text is masked the same way a stored turn is.
    if let Some(request_ref) = request_ref {
        let method = context.method.as_deref().unwrap_or_default();
        let path = context.path.as_deref().unwrap_or_default();
        if matches!(context.source, IncidentSource::Egress | IncidentSource::Mcp)
            && crate::domain::chat_turn::is_model_call(method, path)
        {
            let user_text = crate::domain::chat_turn::turn_from(
                &evaluation.outcome.transformed,
                "",
            )
            .user_text;
            let user_text = crate::domain::chat_turn::truncate_chars(
                &scrub_text(&settings, &user_text, key, sandbox_id),
                2048,
            );
            let dlp = dlp_label(&evaluation);
            let host = destination.map(str::to_string);
            ctx.bus.publish(crate::domain::telemetry::LiveEvent::ChatStarted {
                environment_id,
                sandbox_id,
                request_ref: request_ref.to_string(),
                ts: Utc::now(),
                model: context.model.clone(),
                provider: crate::domain::chat_turn::provider_for(
                    host.as_deref().unwrap_or_default(),
                )
                .to_string(),
                host,
                account_kind: None,
                dlp,
                delivered: evaluation.outcome.permitted,
                user_text,
            });
        }
    }

    Ok(view(evaluation, enforcement))
}

/// What the live event says happened to the content: `blocked`, `pseudonymized`,
/// `replaced`, or nothing when the request went out unchanged.
fn dlp_label(evaluation: &Evaluation) -> Option<String> {
    if !evaluation.outcome.permitted {
        return Some("blocked".to_string());
    }
    if !evaluation.restore.is_empty() {
        return Some("pseudonymized".to_string());
    }
    if evaluation.outcome.action != Action::Allow {
        return Some("replaced".to_string());
    }
    None
}

/// Record a refusal the proxy reported, and tell the live stream about it.
///
/// A refusal is a decision about a person's request: the content check could not
/// answer, or the body was larger than it reads, and the proxy stopped the
/// request. Without this row the chat says "sent" and nothing explains the 503.
///
/// It is a tenant-scoped write with no caller principal — the appliance speaks,
/// exactly as on the inspection path — so the environment comes from the caller
/// (the handler looked it up from the sandbox) and the row carries no value and
/// no fingerprint: there was no finding.
#[allow(clippy::too_many_arguments)]
pub async fn record_refusal(
    ctx: &ServiceContext,
    environment_id: Uuid,
    sandbox_id: Uuid,
    kind: &str,
    destination: Option<&str>,
    method: Option<&str>,
    path: Option<&str>,
    request_ref: Option<&str>,
    model: Option<&str>,
) -> Result<(), ApiError> {
    let detector = match kind {
        "unavailable" => Detector::CheckUnavailable,
        "too_large" => Detector::Oversize,
        other => {
            return Err(ApiError::validation(format!(
                "Unknown refusal kind '{other}': expected 'unavailable' or 'too_large'."
            )));
        }
    };
    let request_ref = request_ref.map(str::trim).filter(|value| !value.is_empty());

    let mut scope = crate::store::TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let settings = resolve_settings(&mut scope, Some(sandbox_id)).await?;
    store::insert_incident(
        &mut scope,
        store::NewIncident {
            sandbox_id,
            detector,
            action: IncidentAction::Blocked,
            severity: IncidentAction::Blocked.severity(),
            masked_value: None,
            value_fingerprint: None,
            destination,
            cloud_delivery: false,
            direction: Direction::Inbound.as_str(),
            requested_action: IncidentAction::Blocked,
            mode: settings.mode.as_str(),
            replacement: None,
            source: IncidentSource::Egress,
            method,
            path,
            model,
            mcp_tool: None,
            conversation_id: None,
            request_ref,
            exception_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    if let Some(request_ref) = request_ref {
        let host = destination.map(str::to_string);
        ctx.bus.publish(crate::domain::telemetry::LiveEvent::ChatStarted {
            environment_id,
            sandbox_id,
            request_ref: request_ref.to_string(),
            ts: Utc::now(),
            model: model.map(str::to_string),
            provider: crate::domain::chat_turn::provider_for(host.as_deref().unwrap_or_default())
                .to_string(),
            host,
            account_kind: None,
            dlp: Some(kind.to_string()),
            delivered: false,
            user_text: String::new(),
        });
    }
    Ok(())
}

/// Evaluate every string value of a JSON document and splice back only the ones
/// a detector changed.
///
/// `None` when the text is not JSON, so the caller falls back to the plain path.
/// The first system block is never rewritten: with a subscription, Anthropic
/// accepts an OAuth request only while that block says exactly what Claude Code
/// wrote (finding B6 / section 5.2 of the privacy module).
#[allow(clippy::too_many_arguments)]
fn evaluate_json(
    settings: &PrivacySettings,
    direction: Direction,
    destination: Option<&str>,
    raw: &str,
    key: Option<&[u8]>,
    exceptions: &[crate::domain::privacy_exception::PrivacyException],
    sandbox_id: Uuid,
) -> Option<Evaluation> {
    let values = crate::domain::privacy_json::string_values(raw)?;
    let mut action = Action::Allow;
    let mut decisions = Vec::new();
    let mut incidents = Vec::new();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut restore = Vec::new();
    let mut system_head_seen = false;

    for value in &values {
        let is_system_head = value.path == ["system"] || value.path == ["system", "text"];
        if is_system_head && !system_head_seen {
            system_head_seen = true;
            continue;
        }
        if crate::domain::privacy_json::is_skipped(&value.path) {
            continue;
        }
        let one = evaluate(
            settings,
            direction,
            destination,
            &value.value,
            key,
            exceptions,
            sandbox_id,
        );
        action = stronger(action, one.outcome.action);
        if one.outcome.transformed != value.value {
            edits.push((value.start, value.end, one.outcome.transformed.clone()));
        }
        decisions.extend(one.outcome.decisions);
        incidents.extend(one.incidents);
        restore.extend(one.restore);
    }

    Some(Evaluation {
        outcome: Outcome {
            action,
            permitted: action != Action::Block,
            transformed: crate::domain::privacy_json::replace_spans(raw, &edits),
            decisions,
        },
        incidents,
        mode: settings.mode,
        restore,
    })
}

/// The more severe of two actions, for aggregating one document.
fn stronger(current: Action, next: Action) -> Action {
    match (current, next) {
        (Action::Block, _) | (_, Action::Block) => Action::Block,
        (
            Action::Redact | Action::Replace | Action::Pseudonymize,
            _,
        )
        | (
            _,
            Action::Redact | Action::Replace | Action::Pseudonymize,
        ) => Action::Replace,
        _ => Action::Allow,
    }
}

/// One text as the store may keep it: every finding replaced, no exception
/// applied, the strict policy always.
///
/// The stored copy must not depend on the mode that happened to be set when the
/// body arrived: a reader is looking for what was in the text, so a value that
/// the current mode merely records is still masked here.
fn scrub_text(
    settings: &PrivacySettings,
    text: &str,
    key: Option<&[u8]>,
    sandbox_id: Uuid,
) -> String {
    let strict = PrivacySettings {
        mode: PrivacyMode::Strict,
        block_cloud_egress: true,
        ..settings.clone()
    };
    evaluate_json(&strict, Direction::Inbound, None, text, key, &[], sandbox_id)
        .unwrap_or_else(|| evaluate(&strict, Direction::Inbound, None, text, key, &[], sandbox_id))
        .outcome
        .transformed
}

/// The text as it may be stored: every finding replaced by its placeholder.
///
/// The proxy captures a body before the content check rewrites it, so the stored
/// copy would otherwise hold the value the policy promised never to keep. The
/// masked text is the one that left (or, in `observe`, a masked copy of the one
/// that left unchanged).
///
/// Settings are read once per sandbox in the batch; a batch carries up to 500
/// bodies and a body is not worth a settings query each.
pub async fn scrub_batch(
    ctx: &ServiceContext,
    environment_id: Uuid,
    items: &[(Uuid, String)],
) -> Result<Vec<String>, ApiError> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let key = privacy_key(&ctx.secret_dir);
    let mut scope = crate::store::TenantScope::begin(&ctx.pool, environment_id)
        .await
        .map_err(ApiError::from)?;
    let mut cache: std::collections::HashMap<Uuid, PrivacySettings> =
        std::collections::HashMap::new();
    let mut out = Vec::with_capacity(items.len());
    for (sandbox_id, text) in items {
        if !cache.contains_key(sandbox_id) {
            let settings = resolve_settings(&mut scope, Some(*sandbox_id)).await?;
            cache.insert(*sandbox_id, settings);
        }
        let settings = cache.get(sandbox_id).ok_or_else(|| {
            ApiError::internal("The settings for a sandbox are missing from the batch cache.")
        })?;
        out.push(scrub_text(settings, text, key, *sandbox_id));
    }
    scope.commit().await.map_err(ApiError::from)?;
    Ok(out)
}

/// A non-admin may read the DLP data of a sandbox they own and of no other.
///
/// Answers "not found" rather than "forbidden" so that the existence of someone
/// else's sandbox is not confirmed by the error.
async fn require_owned_sandbox(
    scope: &mut TenantScope<'_>,
    principal: &Principal,
    sandbox_id: Uuid,
) -> Result<(), ApiError> {
    if principal.role == crate::domain::auth::Role::Admin {
        return Ok(());
    }
    let owner: Option<Uuid> = sqlx::query_scalar(
        "SELECT owner_user_id FROM sandboxes WHERE environment_id = $1 AND id = $2",
    )
    .bind(scope.environment_id())
    .bind(sandbox_id)
    .fetch_optional(scope.connection())
    .await
    .map_err(|error| ApiError::from(crate::store::map_sqlx_error(error)))?
    .flatten();
    if owner == Some(principal.user_id) {
        return Ok(());
    }
    Err(ApiError::not_found(
        ErrorCode::SANDBOX_NOT_FOUND,
        "No such sandbox in this environment.",
    ))
}

/// The policy history of one sandbox and the environment default it inherits.
pub async fn changes(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<incident_store::DlpChange>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    // A sandbox's history belongs to its owner; without one, only an admin may
    // read the environment's history, because it is the same act as reading the
    // default policy.
    if sandbox_id.is_none() && principal.role != crate::domain::auth::Role::Admin {
        return Err(ApiError::forbidden("policy.read"));
    }
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    if let Some(sandbox_id) = sandbox_id {
        require_owned_sandbox(&mut scope, principal, sandbox_id).await?;
    }
    let environment_id = scope.environment_id();
    let items = incident_store::dlp_changes(&mut scope, sandbox_id, environment_id, limit)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(items)
}

/// The wire shape of one evaluation.
///
/// Split out of [`inspect`] so the mapping from a decision to a finding view —
/// which is what the console reads — is testable without a database.
fn view(evaluation: Evaluation, enforcement: crate::domain::privacy::Enforcement) -> InspectResult {
    let findings: Vec<FindingView> = evaluation
        .outcome
        .decisions
        .iter()
        .map(|decision| FindingView {
            detector: decision.detector.as_str().to_string(),
            masked_value: decision.masked_value.clone(),
            action: decision.incident_action().as_str().to_string(),
            requested: decision.action.incident_action().as_str().to_string(),
        })
        .collect();
    InspectResult {
        permitted: evaluation.outcome.permitted,
        action: evaluation.outcome.action_str().to_string(),
        transformed_content: evaluation.outcome.transformed,
        findings,
        restore: evaluation.restore,
        mode: evaluation.mode.as_str().to_string(),
        enforcement: enforcement_str(enforcement).to_string(),
    }
}

/// The wire spelling of [`crate::domain::privacy::Enforcement`].
fn enforcement_str(enforcement: crate::domain::privacy::Enforcement) -> &'static str {
    match enforcement {
        crate::domain::privacy::Enforcement::Enforce => "enforce",
        crate::domain::privacy::Enforcement::RecordOnly => "record_only",
        crate::domain::privacy::Enforcement::Exempt => "exempt",
    }
}

/// The most recent incidents of one sandbox, newest first.
pub async fn list_incidents(
    ctx: &ServiceContext,
    principal: &Principal,
    sandbox_id: Uuid,
    limit: i64,
) -> Result<Vec<PrivacyIncident>, ApiError> {
    require(principal, Permission::TelemetryRead)?;
    let limit = if limit <= 0 {
        DEFAULT_INCIDENTS
    } else {
        limit.min(MAX_INCIDENTS)
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    require_owned_sandbox(&mut scope, principal, sandbox_id).await?;
    let incidents = store::list_incidents(&mut scope, sandbox_id, limit)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(incidents)
}

/// Delete incidents older than `days` and report how many were removed.
///
/// `PolicyGlobal` because this is a destructive act over the environment's
/// evidence: an operator may tune the policy, an admin decides how long it is
/// kept. `purge` is the scheduled caller of the same window.
pub async fn purge(
    ctx: &ServiceContext,
    principal: &Principal,
    days: i64,
) -> Result<u64, ApiError> {
    require(principal, Permission::PolicyGlobal)?;
    if !(1..=3650).contains(&days) {
        return Err(ApiError::validation("days must be between 1 and 3650."));
    }
    let cutoff: DateTime<Utc> = Utc::now() - chrono::Duration::days(days);
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let removed = store::purge_before(&mut scope, cutoff)
        .await
        .map_err(ApiError::from)?;
    crate::store::postgres::audit::record(
        &mut scope,
        crate::store::postgres::audit::NewAuditEvent {
            actor_id: Some(principal.user_id),
            actor_label: &principal.display_name,
            kind: "privacy.purge",
            subject_type: Some("environment"),
            subject_id: Some(principal.environment_id),
            detail: &json!({ "days": days, "incidents": removed }),
            request_id: None,
        },
    )
    .await
    .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(removed)
}

/// [`evaluate`] with no salt, no exceptions and no sandbox: the shape the mode
/// matrix and the storage tests want. The scope only matters to exception
/// matching, which those tests do not exercise.
#[cfg(test)]
fn evaluate_plain(
    settings: &PrivacySettings,
    direction: Direction,
    destination: Option<&str>,
    content: &str,
) -> Evaluation {
    evaluate(
        settings,
        direction,
        destination,
        content,
        None,
        &[],
        Uuid::nil(),
    )
}

/// [`evaluate_json`] with no salt, no exceptions and no sandbox, for tests.
#[cfg(test)]
fn evaluate_json_plain(
    settings: &PrivacySettings,
    direction: Direction,
    destination: Option<&str>,
    raw: &str,
) -> Option<Evaluation> {
    evaluate_json(settings, direction, destination, raw, None, &[], Uuid::nil())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: PrivacyMode) -> PrivacySettings {
        PrivacySettings {
            mode,
            ..PrivacySettings::strict_default()
        }
    }

    #[test]
    fn the_mode_matrix_does_what_each_mode_promises() {
        let content = "password=hunter2 and ada@example.com";

        let off = evaluate_plain(&policy(PrivacyMode::Off), Direction::Outbound, Some("api"), content);
        assert_eq!(off.outcome.action_str(), "allowed");
        assert!(off.outcome.permitted);
        assert_eq!(off.outcome.transformed, content);
        assert!(off.incidents.is_empty());
        assert!(off.outcome.decisions.is_empty());

        let observe = evaluate_plain(
            &policy(PrivacyMode::Observe),
            Direction::Outbound,
            Some("api"),
            content,
        );
        assert_eq!(observe.outcome.action_str(), "allowed");
        assert!(observe.outcome.permitted);
        assert_eq!(observe.outcome.transformed, content);
        assert_eq!(observe.incidents.len(), 2);
        for incident in &observe.incidents {
            assert_eq!(incident.action, IncidentAction::Allowed);
            assert!(incident.cloud_delivery, "observe records the pass-through");
        }

        let review = evaluate_plain(
            &policy(PrivacyMode::Review),
            Direction::Outbound,
            Some("api"),
            content,
        );
        assert_eq!(review.outcome.action_str(), "replaced");
        assert!(review.outcome.permitted, "review never refuses");
        assert!(!review.outcome.transformed.contains("hunter2"));
        assert!(!review.outcome.transformed.contains("ada@example.com"));
        assert_eq!(review.incidents.len(), 2);
        for incident in &review.incidents {
            assert_ne!(incident.action, IncidentAction::Blocked);
            assert!(incident.cloud_delivery);
        }

        let strict = evaluate_plain(
            &policy(PrivacyMode::Strict),
            Direction::Outbound,
            Some("api"),
            content,
        );
        assert_eq!(strict.outcome.action_str(), "blocked");
        assert!(!strict.outcome.permitted);
        assert!(!strict.outcome.transformed.contains("hunter2"));
        assert!(!strict.outcome.transformed.contains("ada@example.com"));
        assert!(
            strict
                .incidents
                .iter()
                .any(|incident| incident.action == IncidentAction::Blocked),
            "a credential blocks in strict mode"
        );
        for incident in &strict.incidents {
            assert!(!incident.cloud_delivery, "a blocked request delivered nothing");
        }
    }

    #[test]
    fn strict_without_the_egress_gate_still_redacts_but_does_not_refuse() {
        let mut settings = policy(PrivacyMode::Strict);
        settings.block_cloud_egress = false;
        let evaluation = evaluate_plain(&settings, Direction::Outbound, Some("api"), "password=hunter2");
        assert!(evaluation.outcome.permitted);
        assert_eq!(evaluation.outcome.action_str(), "replaced");
        assert!(!evaluation.outcome.transformed.contains("hunter2"));
        assert_eq!(evaluation.incidents.len(), 1);
        assert_eq!(evaluation.incidents[0].action, IncidentAction::Replaced);
    }

    #[test]
    fn a_detector_set_that_is_empty_runs_every_detector() {
        let settings = PrivacySettings {
            detectors: Vec::new(),
            ..PrivacySettings::strict_default()
        };
        let evaluation = evaluate_plain(&settings, Direction::Inbound, None, "ada@example.com");
        assert_eq!(evaluation.incidents.len(), 1);
    }

    #[test]
    fn only_the_configured_detectors_run() {
        let settings = PrivacySettings {
            detectors: vec![Detector::IpAddresses],
            ..PrivacySettings::strict_default()
        };
        let evaluation = evaluate_plain(
            &settings,
            Direction::Outbound,
            None,
            "ada@example.com from 10.20.30.40",
        );
        assert_eq!(evaluation.incidents.len(), 1);
        assert_eq!(evaluation.incidents[0].detector, Detector::IpAddresses);
    }

    #[test]
    fn each_detector_finds_its_own_format() {
        let cases: [(&str, Detector, &str); 10] = [
            ("mail ada@example.com now", Detector::EmailAddresses, "ada@example.com"),
            ("host 10.20.30.40", Detector::IpAddresses, "10.20.30.40"),
            ("iban DE89370400440532013000", Detector::Iban, "DE89370400440532013000"),
            ("card 4111 1111 1111 1111", Detector::CreditCards, "4111 1111 1111 1111"),
            (
                "key sk-live-0123456789abcdef",
                Detector::ApiKeys,
                "sk-live-0123456789abcdef",
            ),
            ("password=hunter2", Detector::Credentials, "hunter2"),
            ("reach api.example.com", Detector::Hostnames, "api.example.com"),
            ("call +49 30 1234567", Detector::PhoneNumbers, "+49 30 1234567"),
            ("from Ada Lovelace", Detector::PersonNames, "Ada Lovelace"),
            ("total $1,240.00", Detector::FinancialData, "$1,240.00"),
        ];

        for (text, detector, expected) in cases {
            let findings = detect(text, &Detector::ALL);
            assert!(
                findings
                    .iter()
                    .any(|finding| finding.detector == detector && finding.value == expected),
                "{text} should yield {expected} as {}: {findings:?}",
                detector.as_str()
            );
        }
    }

    #[test]
    fn a_checksum_keeps_the_near_misses_out() {
        let iban = detect("iban DE00370400440532013000", &[Detector::Iban]);
        assert!(iban.is_empty(), "a wrong IBAN check digit is not an IBAN");

        let card = detect("card 4111111111111112", &[Detector::CreditCards]);
        assert!(card.is_empty(), "a number that fails Luhn is not a card");

        let card = detect("card 4111 1111 1111 1111", &[Detector::CreditCards]);
        assert_eq!(card.len(), 1);
    }

    #[test]
    fn a_bare_digit_run_is_not_a_phone_number() {
        assert!(detect("order 1234567890 shipped", &[Detector::PhoneNumbers]).is_empty());
        assert_eq!(detect("call 030-1234567", &[Detector::PhoneNumbers]).len(), 1);
    }

    #[test]
    fn a_file_name_is_not_a_hostname() {
        assert!(detect("see notes.md and report.txt", &[Detector::Hostnames]).is_empty());
        assert_eq!(detect("see build.example.net now", &[Detector::Hostnames]).len(), 1);
    }

    #[test]
    fn one_value_is_reported_once() {
        let findings = detect("ada@example.com and ada@example.com", &[Detector::EmailAddresses]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].value, "ada@example.com");
    }

    #[test]
    fn a_fingerprint_is_sha256_hex_of_the_value() {
        let first = fingerprint("hunter2");
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_eq!(first, fingerprint("hunter2"));
        assert_ne!(first, fingerprint("hunter3"));
        // A known digest, so the function cannot silently change its input.
        assert_eq!(
            fingerprint("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_finding_view_never_carries_the_value() {
        let secret = "sk-live-0123456789abcdef";
        let evaluation = evaluate_plain(
            &policy(PrivacyMode::Strict),
            Direction::Outbound,
            Some("api.provider.example"),
            &format!("key {secret}"),
        );
        let result = view(evaluation, crate::domain::privacy::Enforcement::Enforce);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].detector, "api_keys");
        assert_eq!(result.findings[0].action, "blocked");
        assert!(!result.findings[0].masked_value.contains(secret));
        assert!(!result.transformed_content.contains(secret));
        assert_eq!(result.action, "blocked");
        assert!(!result.permitted);
    }

    #[test]
    fn cloud_delivery_follows_the_mode_and_the_direction() {
        assert!(!cloud_delivery(
            PrivacyMode::Strict,
            Direction::Outbound,
            Some("api"),
            true
        ));
        assert!(cloud_delivery(
            PrivacyMode::Observe,
            Direction::Inbound,
            None,
            false
        ));
        assert!(cloud_delivery(
            PrivacyMode::Review,
            Direction::Outbound,
            Some("api"),
            false
        ));
        assert!(!cloud_delivery(
            PrivacyMode::Review,
            Direction::Inbound,
            Some("api"),
            false
        ));
        assert!(!cloud_delivery(
            PrivacyMode::Strict,
            Direction::Outbound,
            None,
            false
        ));
        assert!(!cloud_delivery(
            PrivacyMode::Off,
            Direction::Outbound,
            Some("api"),
            false
        ));
    }

    #[test]
    fn a_direction_parses_its_own_spelling() {
        assert_eq!(Direction::parse("inbound"), Some(Direction::Inbound));
        assert_eq!(Direction::parse("outbound"), Some(Direction::Outbound));
        assert_eq!(Direction::parse("sideways"), None);
        assert_eq!(Direction::Outbound.as_str(), "outbound");
    }
}



// ---------------------------------------------------------------------------
// DLP across sandboxes: a dashboard and a log
//
// Administrators see the whole environment; anyone else sees the incidents of
// their own sandboxes, because a finding names where content was going and which
// detector fired, and that is the sandbox owner's business first.
// ---------------------------------------------------------------------------

use crate::store::postgres::privacy as incident_store;

/// The dashboard: totals and one breakdown.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct DlpReport {
    pub group_by: String,
    pub since: DateTime<Utc>,
    pub rows: Vec<incident_store::DlpRow>,
    pub totals: incident_store::DlpRow,
}

/// Incidents over a window, grouped by detector, action, severity, sandbox, day
/// or destination.
pub async fn dlp_overview(
    ctx: &ServiceContext,
    principal: &Principal,
    since: DateTime<Utc>,
    group_by: &str,
    sandbox_id: Option<Uuid>,
) -> Result<DlpReport, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let group = incident_store::DlpGroup::parse(group_by).ok_or_else(|| {
        ApiError::validation("group_by is detector, action, severity, sandbox, day or destination.")
    })?;
    let owner = if principal.role == crate::domain::auth::Role::Admin {
        None
    } else {
        Some(principal.user_id)
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let rows = incident_store::dlp_breakdown(&mut scope, group, sandbox_id, owner, since)
        .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;

    let mut totals = incident_store::DlpRow {
        key: "total".into(),
        incidents: 0,
        seen: 0,
        blocked: 0,
        redacted: 0,
        replaced: 0,
        allowed: 0,
        cloud: 0,
    };
    for row in &rows {
        totals.incidents += row.incidents;
        totals.seen += row.seen;
        totals.blocked += row.blocked;
        totals.redacted += row.redacted;
        totals.replaced += row.replaced;
        totals.allowed += row.allowed;
        totals.cloud += row.cloud;
    }
    Ok(DlpReport { group_by: group_by.to_string(), since, rows, totals })
}

/// The log: the newest incidents, across sandboxes, with their masked excerpts.
pub async fn dlp_log(
    ctx: &ServiceContext,
    principal: &Principal,
    since: DateTime<Utc>,
    sandbox_id: Option<Uuid>,
    action: Option<&str>,
    detector: Option<&str>,
    direction: Option<&str>,
    source: Option<&str>,
    q: Option<&str>,
    limit: i64,
) -> Result<Vec<incident_store::DlpEvent>, ApiError> {
    require(principal, Permission::SandboxRead)?;
    let owner = if principal.role == crate::domain::auth::Role::Admin {
        None
    } else {
        Some(principal.user_id)
    };
    let mut scope = TenantScope::begin_for(&ctx.pool, principal)
        .await
        .map_err(ApiError::from)?;
    let items = incident_store::dlp_events(
        &mut scope,
        incident_store::DlpLogQuery {
            sandbox_id,
            owner,
            since,
            action,
            detector,
            direction,
            source,
            q,
            limit,
        },
    )
    .await
        .map_err(ApiError::from)?;
    scope.commit().await.map_err(ApiError::from)?;
    Ok(items)
}

#[cfg(test)]
mod json_tests {
    use super::*;

    fn policy(mode: PrivacyMode) -> PrivacySettings {
        PrivacySettings {
            mode,
            ..PrivacySettings::strict_default()
        }
    }

    fn request_with_tools() -> String {
        let tools: Vec<String> = (0..40)
            .map(|index| {
                format!(
                    r#"{{"name":"tool{index}","description":"see https://github.com/org/repo{index} and mail ops{index}@example.org","input_schema":{{"type":"object"}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"model":"claude-x","tools":[{}],"system":[{{"type":"text","text":"You are Claude Code, Anthropic's official CLI for Claude."}},{{"type":"text","text":"Contact Max Mustermann"}}],"messages":[{{"role":"user","content":[{{"type":"text","text":"IBAN DE89370400440532013000 and ada@example.com"}}]}}]}}"#,
            tools.join(",")
        )
    }

    #[test]
    fn a_json_request_keeps_everything_else_byte_identical() {
        let raw = request_with_tools();
        let evaluation = evaluate_json_plain(
            &policy(PrivacyMode::Strict),
            Direction::Inbound,
            Some("api.anthropic.com"),
            &raw,
        )
        .expect("a valid document");
        let out = &evaluation.outcome.transformed;

        // Tool definitions are not a person's text and must survive untouched.
        assert!(out.contains("https://github.com/org/repo39"));
        assert!(out.contains("ops39@example.org"));
        // The block a subscription is checked against is never rewritten.
        assert!(out.contains("You are Claude Code, Anthropic's official CLI for Claude."));
        // A name in the second system block is a finding.
        assert!(!out.contains("Max Mustermann"));
        // The user's text is rewritten, and the document is still valid JSON.
        assert!(!out.contains("ada@example.com"));
        assert!(out.contains("<EMAIL_ADDRESSES_"));
        serde_json::from_str::<serde_json::Value>(out).expect("still json");
        // Strict refuses the IBAN.
        assert!(!evaluation.outcome.permitted);
        assert_eq!(evaluation.outcome.action, Action::Block);
    }

    #[test]
    fn a_large_document_is_inspected() {
        let padding = "x".repeat(300 * 1024);
        let raw = format!(
            r#"{{"model":"claude-x","system":"{padding}","messages":[{{"role":"user","content":"IBAN DE89370400440532013000"}}]}}"#
        );
        let evaluation = evaluate_json_plain(
            &policy(PrivacyMode::Review),
            Direction::Inbound,
            Some("api.anthropic.com"),
            &raw,
        )
        .expect("a valid document");
        assert!(evaluation.outcome.permitted);
        assert!(evaluation.outcome.transformed.contains("<IBAN_"));
        serde_json::from_str::<serde_json::Value>(&evaluation.outcome.transformed)
            .expect("still json");
    }

    #[test]
    fn free_text_still_takes_the_plain_path() {
        assert!(evaluate_json_plain(&policy(PrivacyMode::Strict), Direction::Inbound, None, "not json").is_none());
    }

    /// RFC 4231, test case 2: the construction the salted fingerprints use.
    #[test]
    fn hmac_sha256_matches_the_rfc_vector() {
        let digest = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// One value with one key is always the same placeholder, and another key is
    /// a different one: that is what makes the stored fingerprint unusable
    /// without the salt.
    #[test]
    fn the_fingerprint_is_salted_when_a_key_is_given() {
        let first = fingerprint_with(Some(b"key-one"), "ada@example.com");
        assert_eq!(
            first,
            fingerprint_with(Some(b"key-one"), "ada@example.com")
        );
        assert_ne!(
            first,
            fingerprint_with(Some(b"key-two"), "ada@example.com")
        );
        assert_ne!(first, fingerprint("ada@example.com"));
        assert_eq!(fingerprint_with(Some(b"key-one"), "ada@example.com").len(), 64);
    }

    /// A key file is 64 hex characters; anything else is not a key.
    #[test]
    fn a_key_is_thirty_two_bytes_of_hex() {
        assert_eq!(hex_bytes(&"ab".repeat(32)), Some(vec![0xab; 32]));
        assert_eq!(hex_bytes("abc"), None);
        assert_eq!(hex_bytes(&"a".repeat(63)), None);
        assert_eq!(hex_bytes(&"zz".repeat(32)), None);
        assert_eq!(hex_bytes(""), None);
    }

    /// An exception for one fingerprint lets exactly that value through; the
    /// next value of the same detector is still refused.
    #[test]
    fn an_exception_allows_one_value_and_leaves_the_others_blocked() {
        let key = b"0123456789abcdef0123456789abcdef";
        let allowed = "DE89370400440532013000";
        let other = "GB82WEST12345698765432";
        let sandbox_id = Uuid::new_v4();
        let exception = crate::domain::privacy_exception::PrivacyException {
            id: Uuid::new_v4(),
            environment_id: Uuid::nil(),
            sandbox_id: Some(sandbox_id),
            detector: Detector::Iban,
            value_fingerprint: Some(fingerprint_with(Some(key), allowed)),
            destination: None,
            action: Action::Allow,
            expires_at: None,
            reason: "the finance team's own account".into(),
            created_by: None,
            created_at: Utc::now(),
        };

        let evaluation = evaluate(
            &policy(PrivacyMode::Strict),
            Direction::Inbound,
            None,
            allowed,
            Some(key),
            std::slice::from_ref(&exception),
            sandbox_id,
        );
        assert_eq!(evaluation.outcome.action, Action::Allow);
        assert!(evaluation.outcome.permitted);
        assert_eq!(evaluation.outcome.transformed, allowed);
        assert_eq!(evaluation.incidents.len(), 1);
        assert_eq!(evaluation.incidents[0].action, IncidentAction::Allowed);
        assert_eq!(evaluation.incidents[0].exception_id, Some(exception.id));

        let evaluation = evaluate(
            &policy(PrivacyMode::Strict),
            Direction::Inbound,
            None,
            other,
            Some(key),
            std::slice::from_ref(&exception),
            sandbox_id,
        );
        assert_eq!(evaluation.outcome.action, Action::Block);
        assert!(!evaluation.outcome.permitted);
        assert_eq!(evaluation.incidents[0].action, IncidentAction::Blocked);
        assert_eq!(evaluation.incidents[0].exception_id, None);
    }
}

#[cfg(test)]
mod scrub_tests {
    use super::*;

    /// What is stored must not depend on the mode that happened to be set: the
    /// proxy captures the body before the check rewrites it, so the masked form
    /// is the only safe thing to keep.
    #[test]
    fn the_stored_form_holds_no_value_whatever_the_mode() {
        let body = r#"{"model":"claude-x","system":"hi","messages":[{"role":"user","content":"IBAN DE89370400440532013000, mail ada@example.com"}]}"#;
        for mode in [PrivacyMode::Strict, PrivacyMode::Observe, PrivacyMode::Review] {
            let settings = PrivacySettings {
                mode,
                ..PrivacySettings::strict_default()
            };
            let stored = PrivacySettings {
                mode: PrivacyMode::Strict,
                block_cloud_egress: true,
                ..settings
            };
            let evaluation = evaluate_json_plain(&stored, Direction::Inbound, None, body)
                .unwrap_or_else(|| evaluate_plain(&stored, Direction::Inbound, None, body));
            assert!(!evaluation.outcome.transformed.contains("DE89370400440532013000"));
            assert!(!evaluation.outcome.transformed.contains("ada@example.com"));
            serde_json::from_str::<serde_json::Value>(&evaluation.outcome.transformed)
                .expect("still json");
        }
    }
}
