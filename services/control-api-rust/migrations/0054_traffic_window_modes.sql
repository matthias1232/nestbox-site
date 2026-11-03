-- A traffic window says *how* it bypasses, and for which channel.
--
-- The existing window allowed everything for a sandbox or for the environment.
-- Two things were missing for the master switches an operator asked for:
--
-- 1. a **channel**, so a DNS window does not open the web and an MCP window does
--    not open arbitrary hosts. "Allow everything" per firewall is the useful
--    unit, not "allow everything everywhere".
-- 2. a **mode**, because observation is not bypass: the request is allowed and
--    the decision is recorded as a block, which is how a policy is developed
--    against real traffic without breaking the workload.
--
-- Both columns are additive with defaults that reproduce the previous behaviour
-- exactly: an existing row is a bypass for every channel.

ALTER TABLE traffic_windows
    ADD COLUMN IF NOT EXISTS mode text NOT NULL DEFAULT 'bypass',
    ADD COLUMN IF NOT EXISTS channel text NOT NULL DEFAULT 'all';

ALTER TABLE traffic_windows
    ADD CONSTRAINT traffic_windows_mode_valid CHECK (mode IN ('bypass', 'audit'));

ALTER TABLE traffic_windows
    ADD CONSTRAINT traffic_windows_channel_valid
    CHECK (channel IN ('all', 'proxy', 'dns', 'mcp', 'gateway', 'openshell'));

-- The decider loads every live window of one environment on each check; this
-- index serves exactly that read.
CREATE INDEX IF NOT EXISTS traffic_windows_channel_idx
    ON traffic_windows (environment_id, channel, expires_at)
    WHERE revoked_at IS NULL;
