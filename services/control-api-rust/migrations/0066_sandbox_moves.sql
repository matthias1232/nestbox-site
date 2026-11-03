-- 0066_sandbox_moves - a sandbox travels from one node to another and back (ADR 0049).
--
-- One row per move, with the step it is at, so a console can show progress, an operator can see
-- where a failed move stopped, and a restart of the control plane can tell an interrupted move
-- from a finished one. Nothing is deleted on the source: it stays as a cold copy until an
-- administrator removes it.

CREATE TABLE sandbox_moves (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    sandbox_id     uuid REFERENCES sandboxes(id) ON DELETE SET NULL,
    sandbox_name   text NOT NULL,
    from_node_id   uuid NOT NULL REFERENCES nodes(id) ON DELETE RESTRICT,
    to_node_id     uuid NOT NULL REFERENCES nodes(id) ON DELETE RESTRICT,
    state          text NOT NULL DEFAULT 'running',
    step           text NOT NULL DEFAULT 'queued',
    -- What each step did, in order, for the console: [{"step","at","detail"}].
    log            jsonb NOT NULL DEFAULT '[]'::jsonb,
    snapshots      jsonb NOT NULL DEFAULT '[]'::jsonb,
    bytes_moved    bigint NOT NULL DEFAULT 0,
    error          text,
    started_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    started_at     timestamptz NOT NULL DEFAULT now(),
    finished_at    timestamptz,
    CONSTRAINT sandbox_moves_state_valid CHECK (state IN ('running', 'done', 'failed')),
    CONSTRAINT sandbox_moves_distinct CHECK (from_node_id <> to_node_id)
);
CREATE INDEX sandbox_moves_env_idx ON sandbox_moves (environment_id, started_at DESC);
-- At most one move per sandbox at a time.
CREATE UNIQUE INDEX sandbox_moves_one_running ON sandbox_moves (sandbox_id) WHERE state = 'running';

ALTER TABLE sandbox_moves ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON sandbox_moves
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);
