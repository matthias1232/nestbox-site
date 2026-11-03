-- 0063_node_join - a machine joins a data center over the WireGuard overlay (ADR 0047).
--
--   * `node_join_tokens` are one-time, short-lived and stored as a hash, like a session;
--   * a node carries the address it has inside the overlay and its WireGuard public key (never
--     the private key, which is made on the node and stays there);
--   * `runner_token_ref` is where the control plane finds the token it presents to that node's
--     runner (a `db://` locator into the vault); `node_token_ref` for a joined node is the
--     `digest://` (a SHA-256) of the same token, which is what the node presents back.

CREATE TABLE node_join_tokens (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash     text NOT NULL UNIQUE,
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    site_id        uuid NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    node_name      text NOT NULL,
    created_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    expires_at     timestamptz NOT NULL,
    used_at        timestamptz,
    node_id        uuid REFERENCES nodes(id) ON DELETE SET NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT node_join_name_format CHECK (node_name ~ '^[a-z0-9]([a-z0-9-]{0,40}[a-z0-9])?$')
);
-- Not row-level secured: a machine joins before any tenant is known. Listing filters by
-- environment explicitly.

ALTER TABLE nodes
    ADD COLUMN overlay_ip       text,
    ADD COLUMN wg_public_key    text,
    ADD COLUMN runner_token_ref text,
    ADD COLUMN joined_at        timestamptz,
    ADD CONSTRAINT nodes_overlay_ip_unique UNIQUE (overlay_ip);

-- The overlay network of each site: 10.77.<n>.0/24, handed out in order.
CREATE SEQUENCE overlay_site_seq START 1;
