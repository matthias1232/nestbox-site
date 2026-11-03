-- 0062_node_ownership_and_names - whose node is it, and who already uses this name (ADR 0044).
--
--   * a node with an environment belongs to that environment's organisation: only that
--     organisation's data centers see it and place sandboxes on it;
--   * the node that existed before belongs to the first environment, so every data center of
--     the provider's own organisation keeps seeing it and a customer's does not;
--   * a sandbox name is the runtime's name for it (container, agent home, SSH alias), so it must
--     be unique across the whole installation, not only inside one data center. Row-level
--     security hides other data centers' sandboxes, hence a function that may look.

UPDATE nodes SET environment_id = '00000000-0000-0000-0000-000000000002'
 WHERE is_local AND environment_id IS NULL
   AND EXISTS (SELECT 1 FROM environments WHERE id = '00000000-0000-0000-0000-000000000002');

-- True when a sandbox of ANOTHER data center already uses the name. SECURITY DEFINER because
-- the caller cannot see those rows; it reveals one bit and no row.
CREATE FUNCTION nb_sandbox_name_taken(p_name text, p_environment uuid) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$ SELECT EXISTS (SELECT 1 FROM sandboxes WHERE name = p_name AND environment_id <> p_environment) $$;

-- The node a sandbox runs on, by its runtime name, for the dispatcher that has to pick a
-- runner before any tenant is known.
CREATE FUNCTION nb_node_of_sandbox(p_name text) RETURNS uuid
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$ SELECT node_id FROM sandboxes WHERE name = p_name ORDER BY created_at DESC LIMIT 1 $$;

-- Whether an environment may use a node: it is the node's own, belongs to the same
-- organisation, or the node is shared (no owner).
CREATE FUNCTION nb_node_usable(p_node_environment uuid, p_environment uuid) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$
    SELECT p_node_environment IS NULL
        OR p_node_environment = p_environment
        OR (SELECT organization_id FROM environments WHERE id = p_node_environment)
         = (SELECT organization_id FROM environments WHERE id = p_environment)
$$;
