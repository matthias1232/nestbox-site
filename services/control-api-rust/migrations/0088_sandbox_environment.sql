-- 0088_sandbox_environment — let the appliance find a sandbox's data center by name.
--
-- The accounting proxy names a sandbox and nothing else: it asks which models the sandbox may use
-- before it routes a request, and it writes the usage row for that request. Neither path carries a
-- principal, and `sandboxes` is under row-level security, so the caller could only ask "which
-- environments exist?" — a question with exactly one right answer while the appliance held one
-- data center. From the second one on, that question answers 422 and every sandbox loses both its
-- granted models and its usage record (review finding F0, X06 fixed only the cluster lookup).
--
-- The sandbox is the thing that knows its tenant, so it answers instead of a guess. A name is
-- unique per data center (`sandboxes_name_unique`), not per appliance, so the function returns a
-- *set*: one row means the sandbox lives in exactly one data center and the caller proceeds under
-- that scope, two rows mean the name is ambiguous and the caller answers 409 rather than routing
-- a request into the wrong tenant, and no row means there is no such sandbox.
--
-- This overloads `nb_sandbox_environment(uuid)` from 0080 — same narrow question, other key. As
-- there, the function runs as its owner (the schema owner, which owns the table) and can only
-- answer an environment id: no listing, no filtering, no other column. Callers must cast the
-- parameter (`$1::text`), otherwise an untyped parameter makes the two overloads ambiguous.
CREATE FUNCTION nb_sandbox_environment(p_name text) RETURNS SETOF uuid
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$
    SELECT environment_id
      FROM sandboxes
     WHERE name = p_name
       AND status <> 'deleted'
$$;

REVOKE ALL ON FUNCTION nb_sandbox_environment(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION nb_sandbox_environment(text) TO nestbox_app;
