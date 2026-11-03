-- 0067_sandbox_moves_visibility - the move runner writes progress without a tenant scope.
--
-- The background task of a move is not a request: it has no session, and its updates to
-- `sandbox_moves` ran under a policy that needs `app.environment_id`, so they matched no rows and
-- the console saw a move stuck at "queued" while it was in fact working. Reads still filter by
-- environment explicitly and by the visibility of the sandbox (see move_service).
ALTER TABLE sandbox_moves DISABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON sandbox_moves;
