//! The node token: how a node proves it is the node it claims to be.
//!
//! The egress proxy and the runner call the control API without a browser
//! session, so they cannot present a cookie or a CSRF token. They present a
//! bearer token instead, one per node, kept in a 0600 file whose location the
//! database holds as a `file://` locator (`nodes.node_token_ref`, ADR 0005).
//!
//! Two properties are deliberate:
//!
//! * **Fail closed.** A missing file, an unreadable locator, an empty token, or
//!   a node with no locator is a refusal. There is no "no token configured, so
//!   allow" branch, because that branch is what turns a deployment mistake into
//!   an open policy endpoint.
//! * **Constant-time comparison.** The comparison hashes both sides with
//!   SHA-256 and compares the digests with `subtle`, so neither the content nor
//!   the length leaks through timing.

use std::path::Path;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::secrets;

/// Why a node token was refused.
///
/// Deliberately does not distinguish "wrong token" from "no token": the caller
/// answers one message, and a probe must not learn whether a node is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTokenError {
    /// The node has no locator, or the locator cannot be resolved to a value.
    NotConfigured,
    /// The presented token is empty.
    Missing,
    /// The presented token is not the node's token.
    Mismatch,
}

impl NodeTokenError {
    /// A message safe to return to a caller: it names no node and no file.
    pub fn message(self) -> &'static str {
        "The node token was refused."
    }

    /// Map to the one response status a refused credential gets.
    pub fn to_api(self) -> crate::error::ApiError {
        crate::error::ApiError::unauthenticated(self.message())
    }
}

/// Verify a token against the value its locator points at.
///
/// The expected value is read through `secrets::resolve`, which refuses a path
/// outside the secret directory and a file on which group or world has any
/// permission bit — the same guard every other credential gets.
pub fn verify(locator: &str, secret_dir: &Path, presented: &str) -> Result<(), NodeTokenError> {
    // A node that joined over the overlay (ADR 0047) has no file on the control plane: it
    // holds its token and the control plane holds the SHA-256 of it, written as
    // `digest://<hex>`. The hash of a 256-bit random token is not a way back to the token.
    if let Some(hex) = locator.strip_prefix("digest://") {
        let presented = presented.trim();
        if presented.is_empty() {
            return Err(NodeTokenError::Missing);
        }
        let digest = Sha256::digest(presented.as_bytes());
        let actual: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let expected = hex.trim().to_ascii_lowercase();
        return if bool::from(actual.as_bytes().ct_eq(expected.as_bytes())) {
            Ok(())
        } else {
            Err(NodeTokenError::Mismatch)
        };
    }
    let expected =
        secrets::resolve(locator, secret_dir).map_err(|_| NodeTokenError::NotConfigured)?;
    let presented = presented.trim();
    if presented.is_empty() {
        return Err(NodeTokenError::Missing);
    }
    if matches(&expected, presented) {
        Ok(())
    } else {
        Err(NodeTokenError::Mismatch)
    }
}

/// Whether two tokens are equal, in constant time.
///
/// Both sides are hashed first so the comparison always runs over 32 bytes: a
/// length-dependent early return would leak the length of the expected token.
pub fn matches(expected: &str, presented: &str) -> bool {
    let left = Sha256::digest(expected.trim().as_bytes());
    let right = Sha256::digest(presented.trim().as_bytes());
    left.ct_eq(&right).into()
}

/// The locator a joined node is stored under: `digest://` and the lower-case hex digest.
pub fn hash_locator(token: &str) -> String {
    let digest = Sha256::digest(token.trim().as_bytes());
    format!("digest://{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nestbox-node-token-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn write_token(dir: &Path, name: &str, value: &str, mode: u32) -> String {
        let file = dir.join(name);
        std::fs::write(&file, format!("{value}\n")).expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).expect("chmod");
        }
        format!("file://{}", file.display())
    }

    #[test]
    fn the_configured_token_is_accepted_and_a_different_one_is_not() {
        let dir = temp_dir("accept");
        let locator = write_token(&dir, "node", "abcdef0123456789", 0o600);
        assert_eq!(verify(&locator, &dir, "abcdef0123456789"), Ok(()));
        // The stored file is trimmed, so a trailing newline in the header is
        // not a mismatch.
        assert_eq!(verify(&locator, &dir, "abcdef0123456789\n"), Ok(()));
        assert_eq!(
            verify(&locator, &dir, "abcdef0123456780"),
            Err(NodeTokenError::Mismatch)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_presented_token_is_refused() {
        let dir = temp_dir("empty");
        let locator = write_token(&dir, "node", "secret-token", 0o600);
        assert_eq!(verify(&locator, &dir, "   "), Err(NodeTokenError::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_or_a_loose_file_fails_closed() {
        let dir = temp_dir("closed");
        let missing = format!("file://{}", dir.join("nope").display());
        assert_eq!(
            verify(&missing, &dir, "anything"),
            Err(NodeTokenError::NotConfigured)
        );

        let loose = write_token(&dir, "loose", "secret-token", 0o644);
        assert_eq!(
            verify(&loose, &dir, "secret-token"),
            Err(NodeTokenError::NotConfigured),
            "a world-readable token may have leaked and must not be trusted"
        );

        assert_eq!(
            verify("vault://team/node", &dir, "secret-token"),
            Err(NodeTokenError::NotConfigured)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn comparison_does_not_depend_on_length() {
        assert!(matches("abc", "abc"));
        assert!(!matches("abc", "abcd"));
        assert!(!matches("", "abc"));
        assert!(matches(" abc ", "abc"));
    }

    #[test]
    fn a_joined_node_is_verified_against_the_hash_of_its_token() {
        let locator = hash_locator("a-long-random-node-token");
        let dir = std::env::temp_dir();
        assert!(verify(&locator, &dir, "a-long-random-node-token").is_ok());
        assert!(verify(&locator, &dir, " a-long-random-node-token\n").is_ok());
        assert_eq!(verify(&locator, &dir, "another"), Err(NodeTokenError::Mismatch));
        assert_eq!(verify(&locator, &dir, ""), Err(NodeTokenError::Missing));
    }
}
