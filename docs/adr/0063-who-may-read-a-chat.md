# 0063 — Who may read a chat, and what the reader leaves behind

## Context

Chats are the most useful thing a sandbox produces and the most sensitive: a
conversation with a model carries what the agent was asked, what it found, and
often the customer record that caused the question. Until now the console showed
them to anyone with read access to the sandbox, without a word about it. That is
a defensible default for an operator debugging their own sandbox and a problem
the moment a colleague, an administrator, or a works council looks at it.

Reading someone else's conversation is processing personal data. Under the GDPR
it needs a legal basis, it needs to be limited to what the purpose requires, and
— in Germany and Austria — it can be subject to co-determination. A feature that
quietly shows text to whoever opens a page fails the second and the third test
without anyone deciding to fail them.

ADR 0056 already established the pattern for terminals: sharing is visible, not
silent. This ADR applies it to content.

## Decision

Three rules, together. Any one of them alone would be worse than none.

**1. A setting, not a default right.** `admin_chat_view` on the environment
decides whether administrators see text at all. With it off, the chat API omits
`user_text` and `assistant_text` — the omission happens in the service, not in a
component, so a new screen cannot show what the API never sent. Administrators
still see metadata and findings: which sandbox, which model, which route, what
the content check did, what it cost. Findings are not text; a masked excerpt and
a fingerprint are what the detector already decided is shareable.

**2. A permanent notice for the end user.** While the setting is on, the workbox
shows the owner of the sandbox a line that stays: *chats of this sandbox can be
read by administrators*. Not a dialog to dismiss, and not a footnote on a
settings page — a fact next to the conversation, because that is where the
expectation is formed. Turning the setting off removes the notice, because it is
then no longer true.

**3. An audit trail for every reader.** Opening a thread records `chat.view`
with the sandbox and the session; opening *all chats* or the live request stream
records `live.watch`, with the end written on `beforeunload` on a best-effort
basis. Best effort is the honest word: a closed tab can lose its ending, and a
record that starts is worth more than a record that only completes. The audit
answers "who looked at this conversation", which is the question a data subject
asks and the one an operator must be able to answer.

**Live deltas are never stored.** The stream that makes an answer grow token by
token carries deltas from the broker to the browser and stops there: no table,
no ring buffer, no log line. Only the finished turn is written, and only masked
(ADR 0027's boundary, applied to content). A live row that disappears on reload
is the intended behaviour and the console says so, because the alternative — a
"live" view backed by a store of half-finished answers — is a second corpus of
content nobody asked for.

**What an owner sees.** Their own sandbox, always, with the notice as context.
`SandboxRead` without ownership is what `admin_chat_view` gates; there is no
third rule, because inventing one would grant a right to read content that the
person who typed it never agreed to.

## Consequences

* An environment where nobody should read chats is configured, not trusted. The
  setting off is a real off: no text through the API, so no text in an export, a
  screenshot or a copied row.
* Findings stay useful with the setting off. Blocked, rewritten and
  pseudonymised rows are still visible as states, with masks, which is what
  debugging a policy actually needs.
* The audit log grows with reads, not with messages. A busy dashboard opening
  *all chats* once per shift is a handful of rows a week.
* A `chat.view` row is written even when the reader is the owner. Their read is
  not the interesting one, but a rule that special-cases the owner is a rule that
  can be got wrong in the direction that matters.
* Retention applies to both: turns and the audit rows are cleaned up by the same
  policy, so "we looked at it" does not outlive "it existed" indefinitely.
* Live viewers of a sandbox where the setting is off see metadata — host, bytes,
  decision, route — and no text, which is the same limitation the console states
  for `forward_proxy` sandboxes.

## Rejected alternatives

* **Admin-only, always on.** Simple, and it makes the notice a lie by omission:
  the person typing would have no way to know.
* **A consent dialog before an admin opens a thread.** It turns a legitimate
  support case into a negotiation with someone who may be asleep, and the
  precedent in ADR 0056 is right: visibility, not permission.
* **Hiding text in the console only.** The browser is not the boundary. The API
  omits the field.
* **Storing live deltas "for completeness".** Half-finished answers are content
  with no context, no end and no owner; they are the exact corpus nobody asked
  to keep.
* **Masking only the findings and showing the rest.** The value is in the
  conversation; masking the incident while showing the sentence around it
  protects nothing.

## What would invalidate this

A tenant that requires co-determination would need per-environment retention and
a documented purpose per read, and the audit becomes a report rather than a log.
If a regulator requires that an administrator cannot read text under any
setting, `admin_chat_view` loses its `on` state and the metadata view becomes the
only view.
