# features/tenants

Customers, data centers and sites (ADR 0044). One page, `TenantsPage.jsx`:

* the organisation tree the caller reaches, with each data center's counts;
* **Add a customer** - only shown to a platform or partner administrator; the API refuses anyone else;
* **Add a data center** in the caller's organisation;
* the **sites** of the active data center, with *Add site* and *Remove* (an empty site only).

`ContextBar.jsx` is the switcher in the topbar: it reads like the mockup's breadcrumb
(`PLATFORM ▸ Berlin`), with the organisation in the eyebrow face and the data center as a
borderless select whose label "Data center" is the accessible name only (V01). It reads the
same tree, so it never offers a data center the caller cannot open.

Nothing here decides who may do what: the buttons are shown by role, and the API repeats the check.
| `TenantsPage.jsx` | also embeds `PermissionsTab` (read-only) per organisation (F22) |

Restyle (F25): `TenantsPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
