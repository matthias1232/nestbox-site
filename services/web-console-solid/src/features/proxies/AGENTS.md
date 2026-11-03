# features/proxies/ — the exit proxy catalogue

One screen, one job: keep the list of places traffic can leave from, and say
which one is enabled.

- A row is data. The firewall's exits (DE, CH, US, AT, Tor) are seeded by
  migration `0059_egress_proxies.sql`; an operator edits them here.
- A password is never typed here. An exit that needs one gets a locator
  (`file:///…`) and the value stays on the host (ADR 0005).
- The screen does not claim a country is verified. `country` is what the
  operator typed; the only proof of an exit is a request that comes back
  through it.
- Choosing an exit for a sandbox is in that sandbox's Policies tab, and for a
  provider profile in Providers. This page only owns the catalogue.

The API is `/api/v1/proxies` (`src/api/proxies.js`); the service is
`services/proxy_service.rs`.

Restyle (F25): `ProxiesPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
