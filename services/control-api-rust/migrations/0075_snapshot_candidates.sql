-- 0075_snapshot_candidates - which folders should be snapshotted now? (ADR 0049)
--
-- The scheduler acts for no tenant and the folder and mount tables are row-level secured, so it
-- asks one function that sees across data centers and returns only what it needs: the folders that
-- are mounted in a running sandbox on a joined, online node, with the time of their last snapshot.
CREATE FUNCTION nb_snapshot_candidates()
    RETURNS TABLE (root_id uuid, environment_id uuid, node_id uuid, host_path text, last_snapshot timestamptz)
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$
    SELECT DISTINCT r.id, r.environment_id, r.node_id, r.host_path,
           (SELECT max(s.taken_at) FROM root_snapshots s WHERE s.root_id = r.id)
      FROM persistent_roots r
      JOIN nodes n ON n.id = r.node_id AND n.wg_public_key IS NOT NULL AND n.status = 'online'
      JOIN mounts m ON m.root_id = r.id AND m.status = 'mounted'
      JOIN sandboxes b ON b.id = m.sandbox_id AND b.status = 'running'
     WHERE r.archived_at IS NULL
$$;
