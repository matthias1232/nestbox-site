-- 0077_usage_observed — usage read off the transparent proxy, with the cache split.
--
-- The accounting proxy only sees traffic an agent was pointed at it. An agent
-- that signed in with its own account (Claude, Codex) talks to its provider
-- directly, and the egress proxy — which reads that traffic when the sandbox is in
-- an inspecting mode — is the only place that sees the answer's usage counters.
-- Those rows go into the same table so one query describes a sandbox.
--
-- Token semantics are unchanged: `prompt_tokens` is everything the model read
-- (fresh input + cache reads + cache writes) and `cached_tokens` is the part served
-- from cache, so `cached_tokens <= prompt_tokens` keeps holding. What is new is the
-- cache *writes* (billed above the input rate), their one-hour share (billed above
-- the five-minute rate), and how many `cache_control` markers the request carried.
--
-- `auth` says whose bill this is. A request made with an API key has a cost;
-- a request made through a signed-in account has none — the subscription is the
-- price — so it carries an *estimate* at list price in its own column and never
-- in `cost_micros`. Mixing the two would make "what did we spend" unanswerable.

ALTER TABLE accounting_requests
    ADD COLUMN cache_write_tokens    int NOT NULL DEFAULT 0 CHECK (cache_write_tokens >= 0),
    ADD COLUMN cache_write_1h_tokens int NOT NULL DEFAULT 0 CHECK (cache_write_1h_tokens >= 0),
    ADD COLUMN cache_markers         int NOT NULL DEFAULT 0 CHECK (cache_markers >= 0),
    ADD COLUMN auth                  text NOT NULL DEFAULT 'unknown'
                                     CHECK (auth IN ('key', 'account', 'unknown')),
    ADD COLUMN source                text NOT NULL DEFAULT 'accounting-proxy'
                                     CHECK (source IN ('accounting-proxy', 'egress')),
    ADD COLUMN conversation          text,
    ADD COLUMN estimated_cost_micros bigint CHECK (estimated_cost_micros IS NULL OR estimated_cost_micros >= 0);

ALTER TABLE accounting_requests
    ADD CONSTRAINT accounting_requests_cache_within_prompt
        CHECK (cached_tokens + cache_write_tokens <= prompt_tokens),
    ADD CONSTRAINT accounting_requests_write_1h_within_write
        CHECK (cache_write_1h_tokens <= cache_write_tokens);

CREATE INDEX accounting_requests_model_idx
    ON accounting_requests (environment_id, model, created_at DESC);

-- The rollup keeps what survives the purge: the cache split and the estimate.
ALTER TABLE accounting_daily
    ADD COLUMN cache_write_tokens    bigint NOT NULL DEFAULT 0,
    ADD COLUMN estimated_cost_micros bigint;
