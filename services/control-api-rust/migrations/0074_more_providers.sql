-- 0074_more_providers - DigitalOcean and Vultr accounts (ADR 0050).
ALTER TABLE provider_accounts DROP CONSTRAINT provider_accounts_kind_valid;
ALTER TABLE provider_accounts ADD CONSTRAINT provider_accounts_kind_valid
    CHECK (kind IN ('hetzner', 'digitalocean', 'vultr', 'proxmox'));
