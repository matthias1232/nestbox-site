-- 0081_privacy_repeats_and_scope — one row per finding, and which hosts a body
-- check applies to.
--
-- An agent resends its whole history on every turn, so the same value is found
-- again and again. Without this, one conversation with one IBAN wrote a row per
-- turn and the dashboard counted turns, not secrets. A repeat within a day now
-- increments the row it already has (`occurrences`, `last_seen_at`).
--
-- `enforce_hosts` answers the second question the first version got wrong: the
-- transparent hop sees every request a sandbox makes, including the browser's.
-- Rewriting a form post to a web shop because an address looked like an e-mail is
-- not protection, it is breakage. The check is enforced for the model hosts and
-- for whatever an operator adds here; everything else is only recorded.

ALTER TABLE privacy_incidents
    ADD COLUMN occurrences  integer NOT NULL DEFAULT 1 CHECK (occurrences >= 1),
    ADD COLUMN last_seen_at timestamptz;

UPDATE privacy_incidents SET last_seen_at = created_at WHERE last_seen_at IS NULL;

ALTER TABLE privacy_incidents
    ALTER COLUMN last_seen_at SET NOT NULL,
    ALTER COLUMN last_seen_at SET DEFAULT now();

-- Serves "is this the same finding within a day": sandbox, value, detector.
CREATE INDEX privacy_incidents_repeat_idx
    ON privacy_incidents (sandbox_id, value_fingerprint, detector, created_at DESC);

-- 0079 added this one; 0056's (environment_id, created_at) already serves it.
DROP INDEX IF EXISTS privacy_incidents_window_idx;

-- `pseudonymize` is an action the incident can now record.
ALTER TABLE privacy_incidents DROP CONSTRAINT IF EXISTS privacy_incidents_action_check;
ALTER TABLE privacy_incidents
    ADD CONSTRAINT privacy_incidents_action_check
    CHECK (action IN ('blocked', 'redacted', 'replaced', 'pseudonymized', 'allowed'));

ALTER TABLE privacy_incidents DROP CONSTRAINT IF EXISTS privacy_incidents_requested_action_check;
ALTER TABLE privacy_incidents
    ADD CONSTRAINT privacy_incidents_requested_action_check
    CHECK (requested_action IS NULL
           OR requested_action IN ('blocked', 'redacted', 'replaced', 'pseudonymized', 'allowed'));

ALTER TABLE privacy_settings
    ADD COLUMN enforce_hosts text[] NOT NULL DEFAULT '{}';
