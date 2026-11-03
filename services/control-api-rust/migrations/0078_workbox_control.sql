-- Workbox control (ADR 0052-0057): per-agent sign-in modes, live egress modes, open-ended
-- evaluation windows, presence for terminals and desktops, model grants, and the node gateway.

-- 1. Per-agent sign-in mode ---------------------------------------------------------------
CREATE TABLE sandbox_agent_auth (
    environment_id  uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    sandbox_id      uuid NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    agent           text NOT NULL,
    mode            text NOT NULL DEFAULT 'gateway',
    pending_restart boolean NOT NULL DEFAULT false,
    updated_by      uuid REFERENCES users(id) ON DELETE SET NULL,
    updated_at      timestamptz NOT NULL DEFAULT now(),
    version         bigint NOT NULL DEFAULT 1,
    PRIMARY KEY (sandbox_id, agent),
    CONSTRAINT sandbox_agent_auth_agent_valid CHECK (agent IN ('claude', 'codex', 'opencode')),
    CONSTRAINT sandbox_agent_auth_mode_valid CHECK (mode IN ('subscription', 'gateway'))
);

CREATE INDEX sandbox_agent_auth_pending_idx
    ON sandbox_agent_auth (environment_id) WHERE pending_restart;

ALTER TABLE sandbox_agent_auth ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON sandbox_agent_auth;
CREATE POLICY tenant_isolation ON sandbox_agent_auth
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

COMMENT ON TABLE sandbox_agent_auth IS
    'The sign-in mode chosen per agent of a sandbox: the own subscription or the gateway.';

-- Which agents a pack brings, so the console knows what can be switched.
ALTER TABLE agent_packs ADD COLUMN IF NOT EXISTS login_agents text[] NOT NULL DEFAULT '{}';
UPDATE agent_packs SET login_agents = ARRAY['claude', 'codex'] WHERE key = 'claude-code';
UPDATE agent_packs SET login_agents = ARRAY['opencode'] WHERE key IN ('opencode', 'openchamber');

-- 2. Live egress modes -------------------------------------------------------------------
-- The mode is decided per request by the proxy now; only `direct` still needs a rebuild.
DO $$
DECLARE constraint_name text;
BEGIN
    FOR constraint_name IN
        SELECT conname FROM pg_constraint
         WHERE conrelid = 'egress_modes'::regclass
           AND contype = 'c'
           AND pg_get_constraintdef(oid) LIKE '%forward_proxy%'
    LOOP
        EXECUTE format('ALTER TABLE egress_modes DROP CONSTRAINT %I', constraint_name);
    END LOOP;
END $$;

ALTER TABLE egress_modes ADD CONSTRAINT egress_modes_mode_valid
    CHECK (mode IN ('direct', 'forward_proxy', 'transparent', 'hybrid', 'off_allow_all'));

ALTER TABLE egress_modes
    ADD COLUMN IF NOT EXISTS changed_at timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN IF NOT EXISTS revert_at  timestamptz,
    ADD COLUMN IF NOT EXISTS revert_to  text;

CREATE INDEX IF NOT EXISTS egress_modes_revert_idx
    ON egress_modes (revert_at) WHERE revert_at IS NOT NULL;

-- 3. Open-ended evaluation windows -------------------------------------------------------
ALTER TABLE traffic_windows DROP CONSTRAINT IF EXISTS traffic_windows_expiry_future;
ALTER TABLE traffic_windows ALTER COLUMN expires_at DROP NOT NULL;
ALTER TABLE traffic_windows ADD CONSTRAINT traffic_windows_expiry_sane
    CHECK (expires_at IS NULL OR expires_at > created_at);

-- NULL means no limit; an administrator may bound it per environment (ADR 0053).
ALTER TABLE environments ADD COLUMN IF NOT EXISTS max_window_minutes integer
    CHECK (max_window_minutes IS NULL OR max_window_minutes BETWEEN 1 AND 525600);

-- 4. Presence ----------------------------------------------------------------------------
ALTER TABLE sandbox_sessions DROP CONSTRAINT IF EXISTS sandbox_sessions_kind_valid;
ALTER TABLE sandbox_sessions ADD CONSTRAINT sandbox_sessions_kind_valid
    CHECK (kind IN ('terminal', 'ssh', 'desktop'));
ALTER TABLE sandbox_sessions
    ADD COLUMN IF NOT EXISTS tab_id    text,
    ADD COLUMN IF NOT EXISTS view_only boolean NOT NULL DEFAULT false;

-- 5. Model grants ------------------------------------------------------------------------
CREATE TABLE model_grants (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id      uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    scope               text NOT NULL,
    user_id             uuid REFERENCES users(id) ON DELETE CASCADE,
    sandbox_id          uuid REFERENCES sandboxes(id) ON DELETE CASCADE,
    model_name          text NOT NULL,
    provider_profile_id uuid NOT NULL REFERENCES provider_profiles(id) ON DELETE CASCADE,
    upstream_model      text,
    is_default          boolean NOT NULL DEFAULT false,
    keep_warm           boolean NOT NULL DEFAULT false,
    created_by          uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at          timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT model_grants_scope_valid CHECK (
        (scope = 'environment' AND user_id IS NULL AND sandbox_id IS NULL)
     OR (scope = 'user' AND user_id IS NOT NULL AND sandbox_id IS NULL)
     OR (scope = 'sandbox' AND sandbox_id IS NOT NULL AND user_id IS NULL)),
    CONSTRAINT model_grants_name_shape CHECK (model_name ~ '^[A-Za-z0-9._:/@-]{1,120}$')
);

CREATE UNIQUE INDEX model_grants_unique ON model_grants (
    environment_id, scope,
    coalesce(user_id, '00000000-0000-0000-0000-000000000000'::uuid),
    coalesce(sandbox_id, '00000000-0000-0000-0000-000000000000'::uuid),
    model_name
);

ALTER TABLE model_grants ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON model_grants;
CREATE POLICY tenant_isolation ON model_grants
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

COMMENT ON TABLE model_grants IS
    'Which models a sandbox may use: granted to the environment, a user or one sandbox.';

-- 6. Node gateway ------------------------------------------------------------------------
ALTER TABLE nodes
    ADD COLUMN IF NOT EXISTS public_endpoint     text,
    ADD COLUMN IF NOT EXISTS wg_listen_port      integer,
    ADD COLUMN IF NOT EXISTS egress_lock         text NOT NULL DEFAULT 'strict',
    ADD COLUMN IF NOT EXISTS tunnel_state        text NOT NULL DEFAULT 'unknown',
    ADD COLUMN IF NOT EXISTS tunnel_handshake_at timestamptz,
    ADD COLUMN IF NOT EXISTS tunnel_rx_bytes     bigint NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS tunnel_tx_bytes     bigint NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'nodes_egress_lock_valid') THEN
        ALTER TABLE nodes ADD CONSTRAINT nodes_egress_lock_valid
            CHECK (egress_lock IN ('strict', 'exit_local'));
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'nodes_tunnel_state_valid') THEN
        ALTER TABLE nodes ADD CONSTRAINT nodes_tunnel_state_valid
            CHECK (tunnel_state IN ('up', 'stale', 'down', 'unknown'));
    END IF;
END $$;

-- Nodes that joined before the lock existed keep their behaviour until someone switches them.
UPDATE nodes SET egress_lock = 'exit_local' WHERE egress_lock = 'strict' AND joined_at IS NOT NULL;
