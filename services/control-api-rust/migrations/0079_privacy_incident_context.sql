-- 0079_privacy_incident_context — one incident says more than "something fired".
--
-- The first version answered three questions: which detector, what the policy
-- did, and a masked excerpt. That is enough for "there was a finding" and not
-- enough for the two questions an operator actually asks: **in which
-- direction** it was going, and **what it was replaced with**. The columns here
-- answer those, and they add the address of the request so a line in the DLP log
-- can be opened next to the traffic entry and the chat it belongs to.
--
-- Still no column holds the value. `replacement` holds the *substitute* the
-- detector wrote — `<IBAN_faf7e1c0>` or `[REDACTED]` — which is derived from the
-- fingerprint and reveals as little as the mask. `direction` and `source` are
-- closed vocabularies with a CHECK, like `action` and `severity`: a new source
-- is a migration, not a typo in a row.
--
-- `requested_action` is separate from `action` on purpose. In `observe` mode the
-- policy asks for a rewrite and does nothing, so `action` is `allowed` while
-- `requested_action` is `replaced`. Storing only the effective action made the
-- log claim the content went out untouched — true, but without saying what the
-- policy thought of it. Both are kept; the difference is the point.

ALTER TABLE privacy_incidents
    ADD COLUMN direction        text NOT NULL DEFAULT 'inbound'
        CHECK (direction IN ('inbound', 'outbound')),
    -- NULL for a row written before this migration: "not recorded", not "allowed".
    ADD COLUMN requested_action text
        CHECK (requested_action IS NULL OR requested_action IN ('blocked', 'redacted', 'replaced', 'allowed')),
    -- The mode in force at the moment of the finding. NULL for older rows.
    ADD COLUMN mode             text
        CHECK (mode IS NULL OR mode IN ('off', 'observe', 'review', 'strict')),
    -- What the detector wrote instead of the value. Never the value.
    ADD COLUMN replacement      text
        CONSTRAINT privacy_incidents_replacement_short
        CHECK (replacement IS NULL OR length(replacement) <= 300),
    -- Where the finding was seen. `egress` is the transparent proxy, the rest
    -- say which hop it was, and `test` is the button on the settings screen.
    ADD COLUMN source           text NOT NULL DEFAULT 'test'
        CHECK (source IN ('egress', 'accounting_proxy', 'mcp', 'test')),
    ADD COLUMN method           text
        CONSTRAINT privacy_incidents_method_short
        CHECK (method IS NULL OR length(method) <= 20),
    ADD COLUMN path             text
        CONSTRAINT privacy_incidents_path_short
        CHECK (path IS NULL OR length(path) <= 500),
    ADD COLUMN model            text
        CONSTRAINT privacy_incidents_model_short
        CHECK (model IS NULL OR length(model) <= 120),
    ADD COLUMN mcp_tool         text
        CONSTRAINT privacy_incidents_mcp_tool_short
        CHECK (mcp_tool IS NULL OR length(mcp_tool) <= 200),
    -- The conversation the request belonged to, when the proxy recognised one.
    -- A reference, not content: it is the id the chat list already uses.
    ADD COLUMN conversation_id  text
        CONSTRAINT privacy_incidents_conversation_short
        CHECK (conversation_id IS NULL OR length(conversation_id) <= 200);

-- The DLP log filters by direction and source as well as by action and
-- detector; the existing (sandbox_id, created_at DESC) index still serves the
-- sandbox view, this one serves "everything in this window" cheaply.
CREATE INDEX privacy_incidents_window_idx
    ON privacy_incidents (environment_id, created_at DESC);
