# providers/ — model endpoints

## Purpose

Show and edit the places a sandbox can send inference requests: the local router,
a cloud API, a vLLM on the overlay, and the server an installed model will run
as. It is the same row shape for all of them, so "local versus cloud" is a
property of the profile, not a separate screen.

## The rules that shape this screen

**No credential value ever crosses the API.** A profile carries a locator such as
`file:///secrets/omniroute_api_key`; the screen shows `configured` or `missing`
and nothing else. A form field for an API key would turn the control plane into a
secret store, which it is not (ADR 0005).

**A status is the server's verdict, never the screen's guess.** `pending`,
`reachable`, and `unreachable` come from `provider_profiles.status`, which only a
probe writes. `pending` means *an install created this profile and its model
server is not started yet* (ADR 0018): the row says that and never calls it
"offline", because nothing has failed. The badge carries the full meaning in its
tooltip and a short version as a second line.

**A removal that is refused is an answer.** `DELETE /providers/{id}` answers
409 `provider.in_use` with `details.sandboxes` when a sandbox still selects the
profile. The screen lists those names instead of showing "request failed", and
the confirm dialog says the removal can be refused before it is attempted.

## What the screen must make obvious

- Which profile a new agent sandbox will use (the first OpenAI-compatible one),
  because that is the endpoint the agent actually talks to.
- Whether the credential resolves: the probe answers "reachable, key accepted"
  or names the specific failure.
- Where an install-created profile came from: the model and the node, under the
  name. A cloud profile keeps the plain row it had.
- That the endpoint is editable data, so pointing at a different provider is an
  edit, not a redeploy.

## Files

| File | Responsibility |
|---|---|
| `ProvidersPage.jsx` | List, filter by status, create, edit, probe, remove |
| `../../styles/providers-status.css` | The status, origin, and refusal blocks |

## Known limit

The console routes by signal (`App.jsx`), not by URL, so a model and a node have
no address to link to. The install origin is therefore a reference with the ids
in its tooltip, not an anchor that would lead nowhere. Turning it into a link
needs a route or a callback from `App.jsx`; neither belongs in this feature.

Restyle (F25): `ProvidersPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
