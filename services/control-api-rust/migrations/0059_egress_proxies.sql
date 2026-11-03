-- 0059 — the exit proxy catalogue, and a fourth egress mode.
--
-- Two things the operator asked for, and one thing they must be able to see:
--
--   1. `direct`: a sandbox whose traffic leaves the node without any proxy at
--      all. The three existing modes all go through the node's egress proxy; a
--      sandbox that must not be observed had no mode to express that.
--   2. A catalogue of exit proxies (the firewall's DE/CH/US/AT exits and Tor),
--      selectable per sandbox and per provider profile. The exit is a fact
--      about *where* traffic leaves, not about whether it is observed, so it is
--      a separate column rather than a fourth meaning of `mode`.
--
-- The password of an exit proxy is never stored here: `password_ref` is a
-- locator (ADR 0005), exactly like `provider_profiles.secret_ref`.

ALTER TABLE egress_modes DROP CONSTRAINT egress_modes_mode_valid;
ALTER TABLE egress_modes
    ADD CONSTRAINT egress_modes_mode_valid
    CHECK (mode IN ('direct', 'forward_proxy', 'transparent', 'hybrid'));

CREATE TABLE proxies (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name           text NOT NULL,
    -- `http` covers an HTTP/HTTPS proxy that speaks CONNECT; `socks5` is the
    -- SOCKS5 port some exits expose next to it.
    kind           text NOT NULL DEFAULT 'http',
    host           text NOT NULL,
    port           integer NOT NULL,
    -- Where the exit is, as the operator thinks of it: DE, CH, US, AT, or
    -- `tor` for an exit that is not a country. Shown in the selector.
    country        text,
    -- Where the proxy comes from (mullvad, tor, custom), so a list of twenty
    -- ports is still readable.
    provider       text,
    username       text,
    -- Locator only, never a value.
    password_ref   text,
    enabled        boolean NOT NULL DEFAULT true,
    version        bigint NOT NULL DEFAULT 1,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT proxies_name_unique UNIQUE (environment_id, name),
    CONSTRAINT proxies_kind_valid CHECK (kind IN ('http', 'socks5')),
    CONSTRAINT proxies_port_valid CHECK (port BETWEEN 1 AND 65535),
    CONSTRAINT proxies_host_not_empty CHECK (length(host) > 0)
);

CREATE INDEX proxies_env_idx ON proxies (environment_id);

COMMENT ON COLUMN proxies.password_ref IS
  'A locator such as file:///etc/nestbox/secrets/exit-de. Never a credential.';

ALTER TABLE proxies ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON proxies;
CREATE POLICY tenant_isolation ON proxies
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

-- Which exit a sandbox leaves through. NULL is the node's own route, which is
-- what every sandbox had before this column existed.
ALTER TABLE sandboxes
    ADD COLUMN proxy_id uuid REFERENCES proxies(id) ON DELETE SET NULL;

-- Which exit the control plane uses when it calls a provider. A provider that
-- must be reached from a particular country is the same problem as a sandbox
-- that must, and the answer is the same row.
ALTER TABLE provider_profiles
    ADD COLUMN proxy_id uuid REFERENCES proxies(id) ON DELETE SET NULL;

-- Seed the exits the appliance's firewall publishes, for every environment that
-- already exists. Ports and countries are the documented set on the OpenWrt
-- cluster; an operator can edit or delete any row. `ON CONFLICT DO NOTHING`
-- keeps a re-run and a renamed row harmless.
INSERT INTO proxies (environment_id, name, kind, host, port, country, provider)
SELECT e.id, seed.name, seed.kind, seed.host, seed.port, seed.country, seed.provider
FROM environments e
CROSS JOIN (VALUES
    ('DE — exit 1 (HTTP)',  'http',   '192.168.55.1', 22541, 'DE', 'mullvad'),
    ('DE — exit 1 (SOCKS)', 'socks5', '192.168.55.1', 22542, 'DE', 'mullvad'),
    ('CH — exit (HTTP)',    'http',   '192.168.55.1', 22540, 'CH', 'mullvad'),
    ('AT — exit (HTTP)',    'http',   '192.168.55.1', 22543, 'AT', 'mullvad'),
    ('US — exit 1 (HTTP)',  'http',   '192.168.55.1', 22544, 'US', 'mullvad'),
    ('US — exit 2 (HTTP)',  'http',   '192.168.55.1', 22545, 'US', 'mullvad'),
    ('No VPN (HTTP)',       'http',   '192.168.55.1', 22547, NULL, 'mullvad'),
    ('Tor (HTTP)',          'http',   '192.168.55.1', 22561, 'tor', 'tor'),
    ('Tor (SOCKS)',         'socks5', '192.168.55.1', 22562, 'tor', 'tor'),
    ('Tor (control)',       'socks5', '192.168.55.1', 22563, 'tor', 'tor')
) AS seed(name, kind, host, port, country, provider)
ON CONFLICT (environment_id, name) DO NOTHING;
