# Privacy Module (DLP)

Before content leaves a sandbox for a language model, the module checks it for
data worth protecting: IBANs, card numbers, credentials, API keys, e-mail
addresses, phone numbers, person names, hostnames, IP addresses and financial
figures. What happens then is the policy's decision: the request is **recorded**
and leaves unchanged, **rewritten** so a placeholder goes out instead of the
value, or **blocked** so it never leaves the appliance. What is stored is a
masked excerpt and a fingerprint, never the value. Administrators and the
sandbox's owner see what happened.

The decisions behind this are in ADR 0059 (the check runs on the decrypted hop),
0060 (a rule may expire), 0061 (pseudonymise and restore on the way back), 0062
(a finding may be allowed on purpose) and 0063 (who may read a chat).

## Terms

| Term | Meaning |
|---|---|
| **Detector** | Recognises one kind of data: `credentials`, `api_keys`, `iban`, `credit_cards`, `email_addresses`, `phone_numbers`, `person_names`, `hostnames`, `ip_addresses`, `financial_data` |
| **Action per detector** | `block`, `replace` (placeholder `<IBAN_faf7e1c0>`), `redact` (`[REDACTED]`), `pseudonymize` (placeholder out, value back on the return path), `allow` (record only) |
| **Mode** (per sandbox, or the environment default) | `off` (nothing), `observe` (record everything, "would have"), `review` (rewrite instead of blocking), `strict` (the configured actions apply) |
| **`block_cloud_egress`** | Without this option a `block` is downgraded to a rewrite |
| **Finding** | One hit: detector, actual and requested action, direction, source, destination, path, model, masked value, replacement, fingerprint |
| **Anonymise** | Replace the value outbound. The model never sees it, and the agent works with the placeholder |
| **Pseudonymise** | Anonymise, and turn the model's answer back into the real value on the return path |

## Where the check runs

| Request path | Can be checked? | State |
|---|---|---|
| Egress mode `transparent` (Docker) | yes — the broker sees the decrypted body | built |
| Egress mode `hybrid` (Docker) | yes, for model hosts only | built |
| Agent SDK with its base URL on the accounting proxy | yes | built (older path) |
| MCP calls on the transparent path | yes (JSON-RPC is recognised as `mcp`) | built |
| `forward_proxy` (OpenShell), `direct` | **no** — a TLS tunnel with no inspection point | impossible by construction |
| The model's answer (return path, SSE) | restore only when pseudonymising; otherwise not recorded | partial |

The transparent path, end to end:

```
sandbox ──TLS──> Envoy CONNECT ──> broker :10001 (terminates TLS)
        ──> Envoy :10000 (policy, Lua: decision and telemetry)
        ──> broker :10002 (content check: block / rewrite / pass, re-encrypts) ──> provider
```

The check runs in `egress-mitm` (`services/egress-proxy/mitm/dlp.go`), the Go
broker that re-encrypts the decrypted hop. It holds the plaintext, can replace
it, and can answer `403` before a byte leaves. The rule itself is the control
API's: the broker sends body, direction and destination, and does what the
answer says.

## How a request is checked

**Only what can be rewritten is read.** `POST`, `PUT` and `PATCH` with a JSON
content type, up to 8 MiB — the limit the control API accepts
(`MAX_PROXY_CONTENT`). A larger body is refused, not truncated and not passed:
half a JSON document is not a smaller document, and a body that was not read
cannot be claimed as clean. The refusal is a finding (`oversize`), so the
console shows it.

**The scan is a span scanner, not a re-serialiser.** Each detector walks the
bytes, validates its checksum where the format has one (Luhn for cards, mod-97
for IBANs) and requires a word boundary. A detector claims its span, and a later
one cannot take it: a card number is not also a phone number. Replacements are
applied to the spans in place, so every byte outside a hit stays exactly where
it was — same key order, same whitespace, same escaping. That is what keeps the
provider's prompt cache working: the same conversation prefix is byte-identical
on every request.

**Two places are never rewritten.**

* The **first text block of `system`**. Claude Code with an OAuth subscription
  is accepted only while that block still starts with its own preamble, and
  `person_names` would rewrite "Claude Code" in it.
* **Login and token paths.** OAuth and sign-in requests are exempt: paths under
  `/oauth/`, a path ending in `/oauth/token`, and the sign-in hosts
  `claude.ai`, `console.anthropic.com`, `platform.claude.com`,
  `auth.openai.com` and `accounts.google.com`. The JSON keys `refresh_token`,
  `access_token`, `code`, `code_verifier`, `client_id` and `state` are not
  checked either. A `credentials` detector would otherwise block token renewal
  in `strict` and sign the sandbox out. The token itself is never read, stored
  or changed.

**Only the right destinations are enforced.** Three kinds of request are
enforced: hosts from the model catalogue (the same list `hybrid` uses, plus the
router address), MCP calls, and hosts an administrator adds in `enforce_hosts`.
Every other destination on the transparent path is only **recorded** — a finding
with the requested action and the action `allowed` — because a browser form post
to a web shop is not a model call, and rewriting it breaks the page. The console
marks these rows `record_only`.

**Failing to check is a decision, not an accident.** When the control API cannot
answer, the broker follows the sandbox's DLP mode: `observe` carries on, every
other mode refuses (`unavailable`). A filter that fails open is a filter an
outage turns off. A refusal without a finding still produces one, with the
detector `check_unavailable` or `oversize`, so a blocked request is never a row
with no explanation.

**Every answer says what happened.** The broker sets `x-nestbox-dlp` on every
response it influenced: `blocked`, `replaced`, `pseudonymized`, `unavailable`,
`too_large` or `skipped`. Envoy copies it into the access log, telemetry stores
it, and the console shows it as a badge per request: *blocked — not delivered*,
*rewritten*, *pseudonymised*, *check failed — not delivered*, *too large — not
delivered*, *exempt*, *clean*.

## What happens to the content

The mode decides how hard the policy works; the detector action says what to do
with one hit.

| Mode | `block` | `replace` / `redact` | `pseudonymize` |
|---|---|---|---|
| `off` | nothing is checked | nothing | nothing |
| `observe` | recorded, request leaves unchanged | recorded, unchanged | recorded, unchanged |
| `review` | rewritten, request leaves | rewritten, request leaves | rewritten, restored on the way back |
| `strict` | refused with `403` | rewritten, request leaves | rewritten, restored on the way back |

`strict` keeps a `block` only while `block_cloud_egress` is on; without it the
block is downgraded to a rewrite, because an installation that has not opted
into refusing traffic must not lose it silently. A `403` is answered in the
provider's own error format, so the agent shows a message instead of a protocol
error.

## Pseudonymisation

`pseudonymize` behaves like `replace` outbound and turns the answer back on the
return path (ADR 0061):

* The placeholder is `<DETECTOR_xxxxxxxx>`, derived from the fingerprint, so the
  same value gets the same placeholder in every message.
* Only the internal inspect endpoint returns the placeholder/value pairs, and
  only for `pseudonymize` findings. The proxy holds them in memory for the
  lifetime of one request; they are never logged, stored or written to the
  database, and `Debug` prints `[redacted]`.
* Inbound, a whole JSON answer has the placeholder in a string value replaced;
  server-sent events hold back a tail that could still become a placeholder and
  prefix it to the next delta of the same block. Tool calls are covered too: a
  placeholder in `partial_json` is almost always split across deltas, so the tail
  is held per `index` and flushed as its own `content_block_delta` line before
  the stop line — otherwise the agent writes `<PERSON_NAMES_…>` into files and
  commands.

## Exceptions

A blocked request is a decision the user may disagree with. An exception says
"this value, or every value of this detector, may leave this sandbox or this
environment until a given moment" (ADR 0062). It is stored in
`privacy_exceptions`, matched by the salted fingerprint and never by the value;
the table has no column that could hold content. An exception is reachable from
a blocked row ("allow for 60 minutes"), so the fix does not require finding the
settings screen first.

## What is stored

**Masked, always.** `privacy_incidents` has a masked excerpt, the replacement
and the fingerprint — no column for the value, so no bug can store one. The same
holds for the body excerpt in `traffic_bodies` and the last message in
telemetry: what is written is what the provider received (the rewritten text),
not what the agent typed. Old rows are cleaned up by retention, not kept
forever.

**Fingerprints are salted.** The fingerprint is an HMAC under a key derived from
`privacy-hmac`, and the placeholder is built from it. An unsalted SHA-256 of a
name or a phone number is reversible with a word list: the provider could
recover the value from the placeholder, and a reader of the database could
recover it from `value_fingerprint`. With a salt, a placeholder is comparable
and not invertible. Losing the key costs the comparison of old findings, not the
data — which is the correct failure.

**Chat turns are the story, not the bytes.** `chat_turns` holds one row per
model call: what was asked and answered (masked), the route (sandbox → egress →
content check → upstream address → router → model), the served model, the
provider, the account kind, the DLP state, and the cost. Rows are written by the
telemetry ingest. Copy and export produce the masked text, because that is the
only text that exists.

**Live events are not stored.** The stream that makes an answer grow word by
word carries deltas to the console and nothing else; only the finished turn is
written. A live row that vanished on reload is the intended behaviour, and the
console says so.

## Who may read a chat

Chat text is personal data, and an administrator reading someone else's
conversation is processing it (ADR 0063). Three rules make that visible instead
of quiet:

* **A setting, not a default right.** `admin_chat_view` on the environment
  decides whether administrators see text at all. With it off they see metadata
  and findings, and the chat API returns no `user_text` or `assistant_text`.
* **A notice for the end user.** While it is on, the workbox shows the owner of
  the sandbox a permanent line: chats of this sandbox can be read by
  administrators.
* **An audit trail.** Opening a thread records `chat.view` (sandbox, session);
  opening *all chats* or the request stream records `live.watch`, with the end
  recorded on `beforeunload` on a best-effort basis.

## Known limits

* **`forward_proxy` and `direct` cannot be checked.** OpenShell sandboxes in
  `forward_proxy` mode send their bytes through a tunnel the appliance does not
  terminate, and `direct` bypasses the proxy. The console says "this sandbox
  cannot be checked for content" instead of showing an empty findings table. For
  those sandboxes the live stream shows metadata rows — host, bytes, decision —
  with the note that the content is not visible.
* **Answers are only changed when pseudonymising.** Every other action works on
  the outbound direction. An answer that streams back token by token is recorded
  by the accounting proxy when the agent routes through it, and otherwise not at
  all.
* **Invented placeholders stay.** A model that writes `<IBAN_1a2b3c4d>` on its
  own — or a value the agent copied from somewhere the check did not see — is not
  turned back. Restore only knows the pairs of one request.
* **The accounting proxy reports live events from a sandbox's second request.**
  The first request of a sandbox establishes the route the stream needs; its turn
  is stored, but the live row appears one request later. The transparent path
  does not have this gap.
* **`person_names` is the weakest detector.** Two capitalised words in a row is
  a heuristic, and "Pull Request" or "Visual Studio" look like names. The first
  system block is protected, and an exception is the intended answer for a
  false positive that keeps coming back.

## Acceptance

The subscription protocol below is run against a sandbox the user signed in to
themselves (default: a dedicated sandbox `nbt-sub`, pack `claude-code`,
`egress_mode: transparent`). Each test is one short prompt. Tools: findings
(`GET /privacy/log?sandbox_id=`), `traffic_bodies` (masked only), the broker log
and the console.

| # | Test | Expected | Result |
|---|---|---|---|
| T0 | Plain prompt, clean text | `200`, no finding, badge *clean* | |
| T1 | IBAN in `strict` | `403`, finding `iban` / `block`, request never reaches the provider | |
| T2 | Same IBAN in `review` | `200`, placeholder `<IBAN_…>` in the body the provider received, finding shows the mask only | |
| T3 | The same IBAN twice in one session | same placeholder both times, prompt cache keeps the prefix | |
| T4 | Card number with a wrong check digit | no finding (checksum rejected) | |
| T5 | Sign-in and token renewal with the subscription | untouched, no finding, badge *exempt*; the session stays signed in | |
| T6 | Tool call carrying a person name, `pseudonymize` | the agent writes the real name to the file, not the placeholder | |
| T7 | Body above 8 MiB | `403`, finding `oversize`, badge *too large* | |
| T8 | Control API stopped, mode `strict` | `403`, finding `check_unavailable` | |
| T9 | Control API stopped, mode `observe` | request leaves, finding recorded | |
