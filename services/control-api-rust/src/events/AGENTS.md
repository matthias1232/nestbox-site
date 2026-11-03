# events/ — the live event bus and SSE

## Purpose

Deliver state changes and telemetry to browsers immediately, not after a polling
interval. The transport is Server-Sent Events because it is one HTTP connection,
works through the existing proxy, reconnects on its own, and needs no WebSocket
upgrade path.

## Dependencies

- May use: `domain` (the event types), `tokio::sync::broadcast`
- Must not use: `sqlx`, `axum` routing (the SSE response is built at the edge)

## Security rules

1. A stream is opened only after authentication, and only for the principal's
   own environment. An event for another environment is dropped, not forwarded.
2. A subscriber that cannot keep up is disconnected with a typed reason rather
   than being allowed to grow memory without bound. Degrading the slow client is
   the correct choice; degrading the server is not.
3. Event payloads are redacted before publishing, at the same boundary as the
   database write.

## The trait, and why it exists

`EventBus` is a trait with one M1 implementation (`LocalBus`, a broadcast
channel). A broadcast channel is per process, so a second API instance would not
see events published on the first. That is a stated M1 limit (ADR 0009), and the
trait is what makes moving to `LISTEN/NOTIFY` or Redis a new implementation
rather than a refactor of every caller.

## Files

| File | Responsibility |
|---|---|
| `bus.rs` | `EventBus` trait, `LocalBus`, subscription |
| `stream.rs` | SSE framing and the per-principal filter |

## Extending this module

Add a variant to `domain::telemetry::LiveEvent` first. Every consumer matches on
the enum, so the compiler will list the places that need a case — which is the
reason to keep it an enum rather than an open string tag.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `mod.rs` | Module root: declares and re-exports the submodules. |
