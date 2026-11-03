-- 0080_sandbox_environment — let the appliance find a sandbox's tenant by id.
--
-- The egress broker knows the sandbox id (Envoy copies it out of the policy
-- answer) and has to name the tenant to write an incident. It has no tenant of
-- its own, and `sandboxes` is under tenant isolation: a plain query without the
-- setting is refused by the policy, not answered with "no rows".
--
-- The function runs as its owner (the schema owner, which owns the table) and
-- exists for exactly one narrow question: "which environment does this sandbox
-- belong to?" It takes an id, returns an environment, and cannot list, filter or
-- return any other row. Writing that as a `SECURITY DEFINER` function is the
-- smallest hole that answers it; the alternative was opening the whole table to
-- the application role.
CREATE OR REPLACE FUNCTION nb_sandbox_environment(p_id uuid)
RETURNS uuid
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT environment_id
      FROM sandboxes
     WHERE id = p_id
       AND status <> 'deleted'
$$;

REVOKE ALL ON FUNCTION nb_sandbox_environment(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION nb_sandbox_environment(uuid) TO nestbox_app;
