# media — rendering model choices

## Purpose

One screen for the question "which model draws our images, clips and tracks?".
The top half is the environment default and the allow-list; the bottom half is a
sandbox's own choice, which may only narrow what the environment allows.

## Files

| File | Responsibility |
|---|---|
| `MediaPage.jsx` | The screen: defaults, allow-list, per-sandbox override, recent renders |
| `useMedia.js` | Loading, saving, error text; no JSX |

## Rules

1. A blank model means "inherit", never "empty model". The server writes NULL and
   the console shows the inherited value next to the field.
2. The provider list comes from the server; the console does not hard-code a
   vendor. A provider missing from the list cannot be chosen, which is why the
   allow-list is the same list with checkboxes.
3. Saving is explicit. A changed select is not applied until "Save", because the
   environment default changes what every sandbox inherits.
4. Rendering itself is not offered here: it is an agent tool. This screen decides
   what that tool may use.

Restyle (F25): `MediaPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
