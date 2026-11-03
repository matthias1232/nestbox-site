-- 0090_member_sees_own_sandboxes — an unowned sandbox is shared with the operators, not with the
-- members of the data center.
--
-- 0061 gave every non-admin role the same visibility: a sandbox they own, one they were granted,
-- or one nobody owns. The last clause is for what a removed user leaves behind (ADR 0038), and it
-- is right for the people who administer a data center. For a member it contradicts the definition
-- of the role — "everything inside the sandboxes the person owns or was granted, nothing outside
-- them" (`domain::auth::Role`) — and it is what made "My sandboxes" list every sandbox of the data
-- center: on an installation whose rows carry no owner (every sandbox whose creator was deleted),
-- a member granted one sandbox saw all of them (U06-05).
--
-- So the roles are now told apart: the control plane (`app.actor_role` unset) and the roles that
-- administer a data center see everything; a member and a viewer see what they own or were granted.
-- Telemetry, network rules and supervisors follow the sandbox (`EXISTS (SELECT 1 FROM sandboxes …)`
-- in 0045), so they narrow with it and need no change here.
--
-- The owner is now written in the INSERT itself rather than in a second statement
-- (`store/postgres/sandboxes.rs`), so a member creating a sandbox passes this WITH CHECK: the row
-- they insert is theirs at the moment it exists.

DROP POLICY IF EXISTS tenant_isolation ON sandboxes;
CREATE POLICY tenant_isolation ON sandboxes
    USING (
        environment_id = current_setting('app.environment_id', true)::uuid
        AND (
            coalesce(current_setting('app.actor_role', true), '') IN ('', 'admin', 'operator')
            OR owner_user_id::text = coalesce(current_setting('app.actor_id', true), '')
            OR EXISTS (SELECT 1 FROM sandbox_grants g
                        WHERE g.sandbox_id = sandboxes.id
                          AND g.user_id::text = coalesce(current_setting('app.actor_id', true), ''))
        )
    )
    WITH CHECK (
        environment_id = current_setting('app.environment_id', true)::uuid
        AND (
            coalesce(current_setting('app.actor_role', true), '') IN ('', 'admin', 'operator')
            OR owner_user_id::text = coalesce(current_setting('app.actor_id', true), '')
            OR EXISTS (SELECT 1 FROM sandbox_grants g
                        WHERE g.sandbox_id = sandboxes.id
                          AND g.user_id::text = coalesce(current_setting('app.actor_id', true), ''))
        )
    );

COMMENT ON POLICY tenant_isolation ON sandboxes IS
  'Second line of defence. The repository filter is the first. A member and a viewer see the '
  'sandboxes they own or were granted; an admin, an operator and the control plane see every one.';
