-- 0071_moves_outlive_nodes - a node can be deleted after its history was written.
--
-- The history of a move names its two nodes. A node that was rented for a day and handed back is
-- deleted, and its moves must stay readable: the names are kept on the row and the ids may go.
ALTER TABLE sandbox_moves
    ADD COLUMN from_node_name text,
    ADD COLUMN to_node_name text;
UPDATE sandbox_moves m SET
    from_node_name = (SELECT name FROM nodes WHERE id = m.from_node_id),
    to_node_name = (SELECT name FROM nodes WHERE id = m.to_node_id);
ALTER TABLE sandbox_moves
    DROP CONSTRAINT sandbox_moves_from_node_id_fkey,
    DROP CONSTRAINT sandbox_moves_to_node_id_fkey,
    DROP CONSTRAINT sandbox_moves_distinct,
    ALTER COLUMN from_node_id DROP NOT NULL,
    ALTER COLUMN to_node_id DROP NOT NULL,
    ADD CONSTRAINT sandbox_moves_from_node_id_fkey FOREIGN KEY (from_node_id) REFERENCES nodes(id) ON DELETE SET NULL,
    ADD CONSTRAINT sandbox_moves_to_node_id_fkey FOREIGN KEY (to_node_id) REFERENCES nodes(id) ON DELETE SET NULL,
    ADD CONSTRAINT sandbox_moves_distinct CHECK (from_node_id IS NULL OR to_node_id IS NULL OR from_node_id <> to_node_id);
