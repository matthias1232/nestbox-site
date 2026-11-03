-- 0069_host_join_retry - a machine that was created but never joined is helped again.
--
-- The join command of a Proxmox clone is run through the guest agent after the machine boots. If
-- the control plane restarts in between, that step is lost, so the reaper repeats it with a new
-- one-time token. It needs the site the machine was meant for and a count of tries.
ALTER TABLE provisioned_hosts
    ADD COLUMN site_id uuid REFERENCES sites(id) ON DELETE SET NULL,
    ADD COLUMN join_attempts integer NOT NULL DEFAULT 0;
