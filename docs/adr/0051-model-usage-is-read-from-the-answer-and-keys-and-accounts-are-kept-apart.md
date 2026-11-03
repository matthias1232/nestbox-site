# 0051 — Model usage is read from the answer, and keys and accounts are kept apart

## Context

The accounting proxy only sees traffic an agent was pointed at it. An agent signed
in with its own account (Claude, Codex) talks to its provider directly, so its
tokens, its cache behaviour and its cost never reached the accounting table.
Operators want one view per sandbox, model, provider, owner, data center and day.

## Decision

* **The proxy that already reads the answer reads its counters.** In an inspecting
  egress mode (`transparent`, `hybrid`) `mcp.lua` recognises a model call by its
  path, extracts the usage counters from the JSON or the last 16 KiB of the stream,
  and writes them into the access log as one compact JSON field in one vocabulary:
  fresh input, output, cache reads, cache writes, the one-hour share of the writes,
  reasoning. Anthropic, OpenAI chat and responses (Codex), and Gemini are mapped;
  OpenAI's `prompt_tokens` includes the cache and Anthropic's `input_tokens` does
  not, so the proxy normalises before it ships.
* **No content, no credential.** The proxy also records how many `cache_control`
  markers the request carried and whether it was made with a key or an account,
  decided from the shape of the credential. The credential itself never leaves the
  Lua function.
* **Same table, new columns.** Rows go into `accounting_requests` (migration 0077)
  with the cache split, `auth`, `source` and the conversation. The id of the row is
  the traffic event's id, so a retried telemetry batch inserts nothing twice.
* **A key has a cost, an account has an estimate.** `cost_micros` is only for calls
  made with an API key (and for what the accounting proxy wrote). A call made through
  a signed-in account carries `estimated_cost_micros` at list price, shown in its
  own column and never added to the spend: the subscription is what is paid.
* **Missing is n/a, not zero.** A model with no price has no figure; setting a price
  prices the calls already stored for that model, and only those that had none.
* **Cache writes follow the input rate** (1.25x for five minutes, 2x for an hour),
  as Anthropic publishes them; cache reads use the price's cached rate.

## Consequences

Usage of a sandbox in `forward_proxy` mode is invisible (the proxy sees a tunnel).
Customer is the organization of the environment and the data center is the node's
site; both are only meaningful across environments once the API grows a
cross-environment read. A non-admin sees their own sandboxes only.
