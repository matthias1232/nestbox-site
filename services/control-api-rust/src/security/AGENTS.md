# security/ — identity, redaction, headers

## Purpose

The cross-cutting controls that would be forgotten if each module implemented
them separately: password hashing, session and CSRF tokens, secret redaction,
and the response headers that make a browser treat the console safely.

This module exists as one folder so there is exactly one implementation of each
control. A second implementation is how a control gets weakened in one place and
not another.

## Dependencies

- May use: `domain`, `argon2`, `sha2`, `rand`, `base64`, `axum` (for headers and
  response types)
- Must not use: `sqlx` directly. Persistence goes through `store/`.

## Security rules

1. A password is hashed with Argon2id. The plaintext is never stored, logged, or
   returned, and never compared with `==`.
2. A session token and a CSRF token are random, and only their SHA-256 hash is
   stored. A database dump therefore yields no usable session.
3. Redaction is applied at the boundary (log, error, event), never at the source.
   A value that is secret must be masked on the way out, so forgetting one call
   site cannot leak it.
4. Comparison of a hash is constant-time. A byte-by-byte compare leaks timing
   that is enough to forge a token one byte at a time.
5. Headers are set on every response, including errors and streams. A CSP that
   is present only on success is not a CSP.
6. A secret locator is resolved by `secrets::resolve` only, which refuses a
   path outside the secret directory and a file that group or world can read.
   The value it returns goes into a container environment or an outbound
   request, never into a response, a log, or the database (ADR 0016).
   `secrets::resolve_verbatim` is the same check without the trailing trim: a
   multi-line secret (a private key) needs its final newline, which OpenSSH
   refuses to parse without. The one response that may carry such a value is the
   one-time SSH key download (ADR 0024), and it is built for exactly that.
7. A node token is verified by `node_token::verify` and nowhere else. Every
   failure — no locator, an unreadable file, an empty token, a wrong token —
   is the same refusal, because a caller must not learn whether a node is
   configured. There is no "no token configured, so allow" branch.

## Conventions

- Every function here is small and total: it either returns a value or a typed
  error, and it never panics on input.
- The list of sensitive key names is one constant. Adding a credential type
  means adding it there, once.
- Redaction over free text runs three passes: key/value pairs, prefixed token
  shapes (`sk-`, `ghp_`, `Bearer `, `-----BEGIN`, and a JWT from `eyJ`), and long
  opaque runs (40+ characters with two character classes). The last is a
  heuristic and stays above the UUID length on purpose: a 36-character id must
  remain readable. It is the only defence for a secret pasted into a body with no
  key name, which is what a prompt looks like.

## Files

| File | Responsibility |
|---|---|
| `password.rs` | Argon2id hashing and verification |
| `session.rs` | Session and CSRF token generation, hashing, comparison |
| `node_token.rs` | A node's bearer token: read through `secrets`, compared in constant time, fail-closed |
| `secrets.rs` | Resolve a `file://` locator to a value, with path and permission guards |
| `redaction.rs` | Masking secrets in strings and JSON values, including the free-text token scan for bodies |
| `headers.rs` | CSP, HSTS, frame options, CORS policy |

## Extending this module

If you are adding a new secret-shaped field, add its key name to
`redaction::SENSITIVE_KEYS` **first**, then write the code that handles it. That
order is deliberate: the mask exists before the value does.

`node_token.rs` also verifies a joined node's `digest://<sha256>` locator, in constant time, next to
the file locators of the appliance's own node.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `mod.rs` | Module root: declares and re-exports the submodules. |
| `vault.rs` | Encryption of stored secrets. |
