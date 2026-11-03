-- 0084 — one model call as a chat turn.
--
-- `traffic_bodies` holds the bytes; this holds the story. The texts are the
-- masked ones: the same body the provider received (or, in `observe`, a masked
-- copy of it). `route` is the chain the console draws: sandbox → egress → content
-- check → upstream address → router → model.
--
-- Rows are written by the telemetry ingest, one per model call. The live stream
-- is not stored: only the finished turn is.
CREATE TABLE chat_turns (
    id                uuid PRIMARY KEY,
    environment_id    uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    sandbox_id        uuid REFERENCES sandboxes(id) ON DELETE CASCADE,
    owner_user_id     uuid,
    ts                timestamptz NOT NULL,
    ended_at          timestamptz,
    request_ref       text CHECK (request_ref IS NULL OR length(request_ref) <= 100),
    session_id        text CHECK (session_id IS NULL OR length(session_id) <= 200),
    agent             text CHECK (agent IS NULL OR length(agent) <= 60),
    provider          text NOT NULL DEFAULT 'other' CHECK (length(provider) <= 40),
    host              text CHECK (host IS NULL OR length(host) <= 253),
    upstream_ip       text CHECK (upstream_ip IS NULL OR length(upstream_ip) <= 64),
    upstream_ip_would text CHECK (upstream_ip_would IS NULL OR length(upstream_ip_would) <= 64),
    account_kind      text CHECK (account_kind IS NULL OR account_kind IN ('account', 'key', 'unknown')),
    requested_model   text CHECK (requested_model IS NULL OR length(requested_model) <= 200),
    served_model      text CHECK (served_model IS NULL OR length(served_model) <= 200),
    router_provider   text CHECK (router_provider IS NULL OR length(router_provider) <= 200),
    router_model      text CHECK (router_model IS NULL OR length(router_model) <= 200),
    user_text         text CHECK (user_text IS NULL OR length(user_text) <= 4096),
    assistant_text    text CHECK (assistant_text IS NULL OR length(assistant_text) <= 16384),
    tool_calls        jsonb NOT NULL DEFAULT '[]',
    input_tokens      bigint,
    output_tokens     bigint,
    cache_read        bigint,
    cache_write       bigint,
    status            integer,
    dlp               text CHECK (dlp IS NULL OR dlp IN ('blocked', 'replaced', 'pseudonymized', 'unavailable', 'too_large', 'skipped')),
    delivered         boolean NOT NULL DEFAULT true,
    route             jsonb NOT NULL DEFAULT '[]'
);

CREATE INDEX chat_turns_env_ts_idx      ON chat_turns (environment_id, ts DESC);
CREATE INDEX chat_turns_sandbox_ts_idx  ON chat_turns (environment_id, sandbox_id, ts DESC);
CREATE INDEX chat_turns_session_idx     ON chat_turns (environment_id, session_id);
CREATE INDEX chat_turns_request_ref_idx ON chat_turns (environment_id, request_ref) WHERE request_ref IS NOT NULL;

ALTER TABLE chat_turns ENABLE ROW LEVEL SECURITY;
ALTER TABLE chat_turns FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON chat_turns
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON chat_turns TO nestbox_app;
