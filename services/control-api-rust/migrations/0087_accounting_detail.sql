-- 0087_accounting_detail — the cluster member a proxied request went through.
--
-- The accounting proxy already reports which cluster it routed for and which member answered
-- (`detail: {cluster_id, install_id}` in its record body), but the ingest body had no field for it
-- and the table had no column, so serde dropped the value before it was stored.
--
-- `jsonb` because the shape belongs to the proxy that wrote it: the next caller with extra
-- metadata adds a key instead of a migration. It carries identifiers, never request content — the
-- same rule 0058 states for the whole row (only a digest of the body is kept).
ALTER TABLE accounting_requests
    ADD COLUMN IF NOT EXISTS detail jsonb;
