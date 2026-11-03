-- 0085 — a rule may expire.
--
-- "Allow this host for an hour" is a rule with an end. Every query that reads
-- rules for a decision, a compilation or a list filters expired ones out, and the
-- reconcile pass deletes them (which invalidates the cache and pushes the gateway
-- policy, exactly as a manual delete does).
ALTER TABLE network_rules ADD COLUMN expires_at timestamptz;

CREATE INDEX network_rules_expiry_idx ON network_rules (expires_at) WHERE expires_at IS NOT NULL;
