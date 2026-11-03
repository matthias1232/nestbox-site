-- 0068_hosting - machines rented from providers, with a lifetime and a budget (ADR 0050).
--
--   * `provider_accounts`: a provider the data center can create machines at. The API token lives
--     in the vault (`secret_name`), never here; `config` holds the non-secret settings (a Proxmox
--     node, a template, an address pool). `max_hourly_milli` caps what running machines may cost
--     per hour in total (thousandths of a currency unit), 0 meaning no cap;
--   * `provisioned_hosts`: each machine created through one, with its price, its lifetime and the
--     node it became.

CREATE TABLE provider_accounts (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id   uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    kind             text NOT NULL,
    name             text NOT NULL,
    config           jsonb NOT NULL DEFAULT '{}'::jsonb,
    secret_name      text NOT NULL,
    max_hourly_milli bigint NOT NULL DEFAULT 0,
    created_by       uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT provider_accounts_kind_valid CHECK (kind IN ('hetzner', 'proxmox')),
    CONSTRAINT provider_accounts_name_unique UNIQUE (environment_id, name),
    CONSTRAINT provider_accounts_budget_sane CHECK (max_hourly_milli >= 0)
);

CREATE TABLE provisioned_hosts (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    account_id     uuid NOT NULL REFERENCES provider_accounts(id) ON DELETE RESTRICT,
    node_name      text NOT NULL,
    external_id    text NOT NULL,
    host_type      text NOT NULL,
    location       text,
    address        text,
    hourly_milli   bigint NOT NULL DEFAULT 0,
    state          text NOT NULL DEFAULT 'creating',
    -- When the machine is deleted whatever happens to be on it (unless data is still there:
    -- then the deadline moves and the console says why).
    expires_at     timestamptz NOT NULL,
    -- Deleted after this long with nothing on it. 0 disables.
    idle_minutes   integer NOT NULL DEFAULT 60,
    idle_since     timestamptz,
    note           text,
    created_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    destroyed_at   timestamptz,
    CONSTRAINT provisioned_hosts_state_valid CHECK (state IN ('creating', 'running', 'destroyed', 'failed')),
    CONSTRAINT provisioned_hosts_idle_sane CHECK (idle_minutes >= 0)
);
CREATE INDEX provisioned_hosts_active_idx ON provisioned_hosts (environment_id) WHERE destroyed_at IS NULL;

ALTER TABLE provider_accounts ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON provider_accounts
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

-- Not row-level secured: the reaper walks every data center's machines. Reads filter by
-- environment explicitly.
