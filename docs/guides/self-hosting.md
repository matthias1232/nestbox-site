# Run Nestbox yourself

One machine, one command. The same code also runs as a hosted service (see
[multi-tenant.md](multi-tenant.md)); you can run either mode on your own hardware.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/matthias1232/nestbox-site/main/deploy/install.sh | sudo bash
```

It installs Docker if it is missing, fetches Nestbox into `/opt/nestbox`, writes `deploy/.env`,
starts the control plane and prints the address and the administrator's one-time password.

| Setting | Meaning | Default |
|---|---|---|
| `NESTBOX_VERSION` | release tag or branch to install | `main` |
| `NESTBOX_MODE` | `selfhosted` (invitations only) or `saas` (sign-up by identity provider) | `selfhosted` |
| `NESTBOX_PUBLIC_URL` | the address people and nodes use | `http://<ip>:8080` |
| `NESTBOX_IMAGES` | `ghcr` pulls the signed release images instead of building | build from source |

Needs: Linux with Docker, about 8 GB RAM for building from source (less with `NESTBOX_IMAGES=ghcr`),
and for nodes on other machines, UDP port 51821 reachable on this machine.

## Verify the released images

```bash
cosign verify ghcr.io/matthias1232/nestbox-control-api:v1.0.0 \
  --certificate-identity-regexp 'https://github.com/matthias1232/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## Sign-in

* **Password** (default): the administrator `admin@nestbox.local` with the password printed once at
  install. Add people under *Users*, or invite them with a link under *Users -> Invitations*.
* **Google, GitHub, Microsoft or any OpenID Connect provider**: set the provider in `deploy/.env`:

  ```
  NESTBOX_OIDC_PROVIDERS=google,github
  NESTBOX_OIDC_GOOGLE_CLIENT_ID=...
  NESTBOX_OIDC_GOOGLE_CLIENT_SECRET=...
  NESTBOX_PUBLIC_URL=https://nestbox.example.com
  ```

  Register this redirect URI at the provider: `https://nestbox.example.com/api/v1/auth/oidc/google/callback`.
  Any other issuer: `NESTBOX_OIDC_PROVIDERS=keycloak`, `NESTBOX_OIDC_KEYCLOAK_ISSUER=https://...`,
  `..._CLIENT_ID`, `..._CLIENT_SECRET`, `..._LABEL=Company login`.

In `selfhosted` mode a person the system does not know is turned away unless they follow an
invitation; in `saas` mode their first sign-in creates their own organisation and data center.

## Back up

```bash
/opt/nestbox/deploy/scripts/backup-control-plane.sh /somewhere/safe
```

writes the database dump and, separately, the vault key. Keep the key somewhere else: the database
holds stored API keys that only that key can open. Persistent data of sandboxes is backed up with
snapshots (see [nodes-and-moves.md](nodes-and-moves.md)).

## Update

```bash
NESTBOX_VERSION=v1.1.0 sudo -E bash /opt/nestbox/deploy/install.sh
```

Migrations run on start. Nodes follow by themselves within five minutes.
