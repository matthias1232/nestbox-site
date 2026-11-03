//! Session and CSRF tokens.
//!
//! Tokens are random, and only their SHA-256 hash is stored. A database dump
//! therefore yields no usable session, which is the property that makes the
//! "session table" acceptable to keep at all.
//!
//! Comparison is constant-time. A byte-by-byte compare leaks enough timing to
//! forge a token one byte at a time, and the fix costs nothing.

use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Bytes of entropy per token. 32 bytes is 256 bits: far beyond guessable, and
/// small enough to keep a cookie short.
const TOKEN_BYTES: usize = 32;

/// A freshly generated token pair.
#[derive(Debug, Clone)]
pub struct IssuedSession {
    /// Goes to the browser in a cookie. Never stored.
    pub token: String,
    /// Stored in the database.
    pub token_hash: String,
    /// Sent to the client for the double-submit check. Never stored.
    pub csrf_token: String,
    /// Stored in the database.
    pub csrf_hash: String,
}

/// Generate a session and its CSRF token.
pub fn issue() -> IssuedSession {
    let token = random_token();
    let csrf_token = random_token();
    IssuedSession {
        token_hash: hash_token(&token),
        csrf_hash: hash_token(&csrf_token),
        token,
        csrf_token,
    }
}

/// Hash a token for storage or comparison.
///
/// SHA-256 rather than Argon2 because a token is already 256 bits of entropy:
/// there is nothing to brute-force, so the memory-hard property buys nothing and
/// costs latency on every request.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    // Lowercase hex, so the value is safe in a URL and easy to compare in SQL.
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Constant-time comparison of two token hashes.
pub fn hashes_equal(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

/// 32 random bytes as lowercase hex.
pub fn random_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The cookie name for the session.
pub const SESSION_COOKIE: &str = "nestbox_session";
/// A new CSRF token, for a session that lost its own.
pub fn issue_csrf_token() -> String {
    random_token()
}

/// The header carrying the CSRF token on a mutating request.
pub const CSRF_HEADER: &str = "x-csrf-token";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_sessions_never_share_a_token() {
        let a = issue();
        let b = issue();
        assert_ne!(a.token, b.token);
        assert_ne!(a.csrf_token, b.csrf_token);
    }

    #[test]
    fn the_stored_value_is_a_hash_not_the_token() {
        let session = issue();
        assert_ne!(session.token_hash, session.token);
        assert_eq!(session.token_hash.len(), 64); // hex of 32 bytes
        assert_eq!(session.token_hash, hash_token(&session.token));
    }

    #[test]
    fn comparison_is_correct_for_equal_and_unequal_values() {
        let a = hash_token("one");
        let b = hash_token("one");
        let c = hash_token("two");
        assert!(hashes_equal(&a, &b));
        assert!(!hashes_equal(&a, &c));
        assert!(!hashes_equal(&a, "short"));
    }

    #[test]
    fn a_token_is_long_enough_to_be_unguessable() {
        let session = issue();
        assert_eq!(session.token.len(), 64);
        // Hex of 32 bytes: 256 bits of entropy.
        assert_eq!(session.token.chars().count(), 64);
    }
}
