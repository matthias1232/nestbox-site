# hosting/ - machines from providers

## Purpose

Turn "I need a node" into a running machine that joins the data center (ADR 0050). A driver knows
one provider's API: what it can create, what that costs by the hour, how to create it with the join
command inside, how to see whether it is there, and how to delete it. It knows nothing about data
centers, tokens or budgets: `services/provision_service.rs` decides those.

## Files

| File | Responsibility |
|---|---|
| `mod.rs` | The `HostDriver` trait, the types a driver speaks, the registry of drivers by kind |
| `hetzner.rs` | Hetzner Cloud: `user_data` carries the join command |
| `digitalocean.rs` | DigitalOcean: `user_data` carries the join command (unit tested, not run live) |
| `vultr.rs` | Vultr: base64 `user_data` (unit tested, not run live) |
| `proxmox.rs` | Proxmox VE: clone a template, give it an address, start it, run the join command through the QEMU guest agent |

## Rules

1. A driver never sees a secret except the one token it is constructed with.
2. Every network call has a timeout; a provider that does not answer is an error, not a hang.
3. Prices are what the provider publishes, in thousandths of a currency unit per hour, and a
   driver that cannot price a type says so (`None`) instead of guessing.
4. Not provided: AWS, GCP, NVIDIA Brev, and a generic SSH driver. A machine from any other host joins with the one-line command
   or the cloud-init text the console shows.
