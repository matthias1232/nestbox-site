-- 0082 — link an incident to the request it came from, and two environment switches.
--
-- `request_ref` is the proxy's `x-request-id`. It is what lets a DLP finding, a
-- captured body and a stored chat turn be opened as one story instead of three
-- tables with no join. `exception_id` records that a finding was allowed on
-- purpose, so a reader can tell "the policy said allow" from "someone allowed
-- this one".
--
-- The two switches: whether a sandbox owner may grant exceptions for their own
-- sandbox, and whether an administrator may read chat content at all. The second
-- one is a decision, not a default nobody had to choose — see ADR 0063.

ALTER TABLE privacy_incidents
    ADD COLUMN request_ref  text
        CONSTRAINT privacy_incidents_request_ref_short
        CHECK (request_ref IS NULL OR length(request_ref) <= 100),
    ADD COLUMN exception_id uuid;

CREATE INDEX privacy_incidents_request_ref_idx
    ON privacy_incidents (environment_id, request_ref) WHERE request_ref IS NOT NULL;

ALTER TABLE privacy_settings
    ADD COLUMN owners_may_grant_exceptions boolean NOT NULL DEFAULT false,
    ADD COLUMN admin_chat_view             boolean NOT NULL DEFAULT true;
