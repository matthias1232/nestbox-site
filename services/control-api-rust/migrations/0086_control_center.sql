-- 0086_control_center — model clusters, recording mutes, first-run tours.
--
-- 1. A model cluster is several installs of one model behind one provider profile, spread
--    by the accounting proxy (round robin, least loaded or failover).
-- 2. Two mute columns on sandboxes let a member stop telemetry storage for a while.
-- 3. The `jobs.kind` column has no CHECK constraint (see 0006), so the three new job kinds
--    (`model_bulk_install`, `node_drain`, `node_quick_deploy`) need no schema change.
-- 4. `ui_tours_seen` remembers which guided tour a user has finished.

-- 1. Model clusters ------------------------------------------------------------------------
CREATE TABLE model_clusters (
    id                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id      uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name                text NOT NULL,
    model_key           text NOT NULL REFERENCES models(key) ON DELETE CASCADE,
    strategy            text NOT NULL DEFAULT 'round_robin'
        CHECK (strategy IN ('round_robin', 'least_loaded', 'failover')),
    provider_profile_id uuid REFERENCES provider_profiles(id) ON DELETE SET NULL,
    created_by          uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now(),
    UNIQUE (environment_id, name)
);

CREATE INDEX model_clusters_environment_idx ON model_clusters (environment_id);
CREATE INDEX model_clusters_model_idx ON model_clusters (model_key);

CREATE TABLE model_cluster_members (
    cluster_id uuid NOT NULL REFERENCES model_clusters(id) ON DELETE CASCADE,
    install_id uuid NOT NULL REFERENCES model_installs(id) ON DELETE CASCADE,
    weight     integer NOT NULL DEFAULT 1 CHECK (weight BETWEEN 1 AND 100),
    added_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (cluster_id, install_id)
);

CREATE INDEX model_cluster_members_install_idx ON model_cluster_members (install_id);

ALTER TABLE model_clusters ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON model_clusters;
CREATE POLICY tenant_isolation ON model_clusters
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

-- Members have no environment column; they are visible through their cluster.
ALTER TABLE model_cluster_members ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation ON model_cluster_members;
CREATE POLICY tenant_isolation ON model_cluster_members
    USING (EXISTS (
        SELECT 1 FROM model_clusters c
         WHERE c.id = model_cluster_members.cluster_id
           AND c.environment_id = current_setting('app.environment_id', true)::uuid))
    WITH CHECK (EXISTS (
        SELECT 1 FROM model_clusters c
         WHERE c.id = model_cluster_members.cluster_id
           AND c.environment_id = current_setting('app.environment_id', true)::uuid));

COMMENT ON TABLE model_clusters IS
    'Installs of one model behind one provider profile, spread by the accounting proxy.';

-- 2. Recording mutes ----------------------------------------------------------------------
ALTER TABLE sandboxes
    ADD COLUMN IF NOT EXISTS telemetry_mute_until        timestamptz NULL,
    ADD COLUMN IF NOT EXISTS telemetry_mute_bodies_until timestamptz NULL;

-- 4. First-run tours ----------------------------------------------------------------------
CREATE TABLE ui_tours_seen (
    user_id uuid REFERENCES users(id) ON DELETE CASCADE,
    tour    text,
    seen_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, tour)
);
