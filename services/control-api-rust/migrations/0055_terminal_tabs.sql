-- 0055 — the terminal tabs a browser reopens after a reload.
--
-- The tab list used to live in `localStorage` only. That made the server the
-- wrong side of the truth: a reload restored the ids the browser remembered, and
-- nothing told the sandbox which shells were still alive, so an image without
-- tmux silently opened a fresh shell in place of the application the operator had
-- running. The row is a promise the control plane can keep and audit — this
-- sandbox, this user, this named tmux session — and it is bounded so a browser
-- cannot grow it without limit.

CREATE TABLE sandbox_terminal_tabs (
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    sandbox_id     uuid NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tab_id         text NOT NULL,
    title          text NOT NULL,
    position       integer NOT NULL DEFAULT 0,
    created_at     timestamptz NOT NULL DEFAULT now(),
    last_seen_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (sandbox_id, user_id, tab_id),
    -- The id becomes a tmux session name, so the shape is enforced here as well
    -- as in the API: one place that refuses is not enough for a value that is
    -- interpolated into a command.
    CONSTRAINT sandbox_terminal_tabs_id_shape CHECK (tab_id ~ '^[A-Za-z0-9_-]{1,64}$'),
    CONSTRAINT sandbox_terminal_tabs_title_len CHECK (length(title) BETWEEN 1 AND 120),
    CONSTRAINT sandbox_terminal_tabs_position_sane CHECK (position BETWEEN 0 AND 63)
);

CREATE INDEX sandbox_terminal_tabs_user_idx
    ON sandbox_terminal_tabs (environment_id, sandbox_id, user_id, position);

ALTER TABLE sandbox_terminal_tabs ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tenant_isolation ON sandbox_terminal_tabs;

CREATE POLICY tenant_isolation ON sandbox_terminal_tabs
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

COMMENT ON TABLE sandbox_terminal_tabs IS
    'The named terminal tabs one user keeps in one sandbox: the list a browser '
    'reopens after a reload. It records no keystroke and no output (ADR 0023).';
