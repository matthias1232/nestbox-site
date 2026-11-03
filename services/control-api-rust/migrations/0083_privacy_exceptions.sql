-- 0083 — a finding allowed on purpose.
--
-- A blocked request is a decision the user may disagree with. An exception says
-- "this value (or every value of this detector), for this sandbox or the whole
-- environment, may leave until a given moment". It is matched by the salted
-- fingerprint, never by the value: the table cannot hold content even if a caller
-- wanted it to.
CREATE TABLE privacy_exceptions (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id    uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    -- NULL is the environment's exception, not a bug. A sandbox row is narrower.
    sandbox_id        uuid REFERENCES sandboxes(id) ON DELETE CASCADE,
    detector          text NOT NULL,
    -- NULL means "any value of this detector". A set value is one exact match.
    value_fingerprint text CHECK (value_fingerprint IS NULL OR value_fingerprint ~ '^[0-9a-f]{64}$'),
    -- NULL means "any destination".
    destination       text CHECK (destination IS NULL OR length(destination) <= 253),
    action            text NOT NULL CHECK (action IN ('allow', 'replace', 'pseudonymize')),
    expires_at        timestamptz,
    reason            text NOT NULL DEFAULT '' CHECK (length(reason) <= 500),
    created_by        uuid,
    created_at        timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX privacy_exceptions_scope_idx ON privacy_exceptions (environment_id, sandbox_id);

ALTER TABLE privacy_exceptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE privacy_exceptions FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON privacy_exceptions
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

GRANT SELECT, INSERT, UPDATE, DELETE ON privacy_exceptions TO nestbox_app;

ALTER TABLE privacy_incidents
    ADD CONSTRAINT privacy_incidents_exception_fk
    FOREIGN KEY (exception_id) REFERENCES privacy_exceptions(id) ON DELETE SET NULL;
