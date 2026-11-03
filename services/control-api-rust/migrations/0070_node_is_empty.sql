-- 0070_node_is_empty - may the reaper delete this machine's node? (ADR 0050)
--
-- The reaper acts for no tenant, and the tables that say what lives on a node are row-level
-- secured, so it asked a question the database could not answer without a scope. The function
-- answers one bit across every data center and returns no row.
CREATE FUNCTION nb_node_is_empty(p_node uuid) RETURNS boolean
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$
    SELECT NOT EXISTS (SELECT 1 FROM sandboxes WHERE node_id = p_node AND status <> 'deleted')
       AND NOT EXISTS (SELECT 1 FROM model_installs WHERE node_id = p_node)
       AND NOT EXISTS (SELECT 1 FROM persistent_roots WHERE node_id = p_node AND archived_at IS NULL)
$$;
