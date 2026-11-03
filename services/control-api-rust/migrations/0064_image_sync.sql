-- 0064_image_sync - which images belong on which machines, and what each machine holds (ADR 0048).
--
--   * `image_assignments` say that an image should be on every node of a data center, of a site,
--     or on one node;
--   * `node_images` is what each node reported last, with the state of the last pull, so the
--     console can draw images x nodes without asking every machine on every page load.

CREATE TABLE image_assignments (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    image          text NOT NULL,
    scope_type     text NOT NULL,
    scope_id       uuid,
    created_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT image_assignments_scope_valid CHECK (scope_type IN ('environment', 'site', 'node')),
    CONSTRAINT image_assignments_scope_id CHECK ((scope_type = 'environment') = (scope_id IS NULL)),
    CONSTRAINT image_assignments_image_shape CHECK (image ~ '^[A-Za-z0-9][A-Za-z0-9._/:@-]{0,299}$')
);
CREATE UNIQUE INDEX image_assignments_unique
    ON image_assignments (environment_id, image, scope_type, coalesce(scope_id, '00000000-0000-0000-0000-000000000000'::uuid));

ALTER TABLE image_assignments ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON image_assignments
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

CREATE TABLE node_images (
    node_id    uuid NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    image      text NOT NULL,
    state      text NOT NULL,
    size_bytes bigint,
    error      text,
    checked_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (node_id, image),
    CONSTRAINT node_images_state_valid CHECK (state IN ('present', 'pulling', 'missing', 'failed'))
);
-- Not row-level secured: the sync loop writes it for every node. Reads go through a service
-- that filters by the caller's data center.
