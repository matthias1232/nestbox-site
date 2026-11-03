-- 0076_hosts_outlive_accounts - an account can be removed after its machines were handed back.
-- The history keeps the account's name; the link may go.
ALTER TABLE provisioned_hosts ADD COLUMN account_name text;
UPDATE provisioned_hosts h SET account_name = (SELECT name FROM provider_accounts a WHERE a.id = h.account_id);
ALTER TABLE provisioned_hosts
    DROP CONSTRAINT provisioned_hosts_account_id_fkey,
    ALTER COLUMN account_id DROP NOT NULL,
    ADD CONSTRAINT provisioned_hosts_account_id_fkey FOREIGN KEY (account_id) REFERENCES provider_accounts(id) ON DELETE SET NULL;
