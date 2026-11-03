-- 0072_root_snapshots - what was snapshotted, when, by whom (ADR 0049).
--
-- A sandbox working away from home is snapshotted on a timer, so the machine can be lost without
-- losing the work. This is the list of those snapshots, per folder, for the console and for the
-- "is there anything newer than the last snapshot" question.
CREATE TABLE root_snapshots (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    root_id        uuid NOT NULL REFERENCES persistent_roots(id) ON DELETE CASCADE,
    node_id        uuid REFERENCES nodes(id) ON DELETE SET NULL,
    snapshot       text NOT NULL,
    kind           text NOT NULL DEFAULT 'scheduled',
    bytes_added    bigint NOT NULL DEFAULT 0,
    taken_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT root_snapshots_kind_valid CHECK (kind IN ('scheduled', 'move', 'manual'))
);
CREATE INDEX root_snapshots_root_idx ON root_snapshots (root_id, taken_at DESC);
-- Not row-level secured: the scheduler writes it for every data center; reads filter by environment.
