# features/images

The image registry and how images reach nodes (ADR 0048). `ImagesPage.jsx` shows

* the **matrix**: images down, nodes across, each cell `present`, `pulling`, `failed` or empty;
* the **assignments**: which image should be on every node of the data center, of a site, or on one node;
* a form to assign an image from what the registry holds.

The matrix is what the nodes last reported, not a live query: the control plane's loop pulls
what is missing every 30 seconds and records the result.

Restyle (F25): `ImagesPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
