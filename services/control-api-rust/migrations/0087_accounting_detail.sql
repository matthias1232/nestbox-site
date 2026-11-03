-- 0087_accounting_detail — the cluster member a proxied request went through, and the data center
-- of a cluster for the caller that has no tenant.
--
-- 1. The accounting proxy already reports which cluster it routed for and which member answered
--    (`detail: {cluster_id, install_id}` in its record body), but the ingest body had no field for it
--    and the table had no column, so serde dropped the value before it was stored.
--
--    `jsonb` because the shape belongs to the proxy that wrote it: the next caller with extra
--    metadata adds a key instead of a migration. It carries identifiers, never request content — the
--    same rule 0058 states for the whole row (only a digest of the body is kept).
--
-- 2. `nb_cluster_environment` gives the accounting proxy the data center of one cluster without a
--    principal (X06). See the comment above the function.
ALTER TABLE accounting_requests
    ADD COLUMN IF NOT EXISTS detail jsonb;

-- The data center of one cluster, for the caller that has no tenant ---------------------------
--
-- The accounting proxy asks for a cluster's members before it has a principal, and `model_clusters`
-- is row-level secured, so the proxy could only find the data center by asking which environments
-- exist — a question with one right answer only while the appliance holds one data center. The
-- function answers the id from any data center and returns no row, so every other read still runs
-- under a scope.
CREATE FUNCTION nb_cluster_environment(p_cluster uuid) RETURNS uuid
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS
$$
    SELECT environment_id FROM model_clusters WHERE id = p_cluster
$$;
