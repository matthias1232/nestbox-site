# features/hosting

Rented machines (ADR 0050). `HostingPage.jsx` shows

* the **provider accounts** (Hetzner, Proxmox) with what their running machines cost per hour
  against the account's budget, and a form to add one (the API token is sent once and stored
  encrypted);
* **Rent a machine**: account, machine type with its hourly price, site, a lifetime and an idle
  limit. The machine joins the data center by itself and appears under Nodes;
* the **machines**: state, cost so far, when each is handed back, and *Delete now* (refused while
  data is still on its node).

Prices are the provider's, shown per hour; a type without a published price is marked as such.

Restyle (F25): `HostingPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
