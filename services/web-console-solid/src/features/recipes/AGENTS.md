# recipes/ — the templates an image is built from

## Purpose

The list, the drawer, and the two forms that change a recipe: the editor and the
build. A recipe is the desired state of a sandbox image; building it produces an
image tagged by the content hash, and a sandbox created from it runs that image.

## Files

| File | Responsibility |
|---|---|
| `RecipesPage.jsx` | The table: name, baseline, revision, state, image, node |
| `RecipeDetail.jsx` | The drawer: what the recipe asks for, the build, run it |
| `RecipeEditor.jsx` | The PATCH form, over the whole recipe |
| `useRecipes.js` | List, selection, build polling, the four actions |

Styles live in `src/styles/recipes-persist.css`, imported by the page.

## Decisions that must not be quietly undone

1. **The image reference is immutable, and the drawer says so next to it.** The
   tag is the content hash, so an edit produces a new reference rather than
   rewriting the image behind the old one. A UI that shows an edit "updating" the
   tag would be claiming the runtime does something it does not.
2. **`build_error` is shown verbatim** in a `<pre>`. It is the runtime's or the
   generator's own words, and a summary would hide the line that matters.
3. **A build is a job, not a progress bar.** The row is the only place the
   outcome lands, so the drawer polls the recipe while it is `building`, stops on
   the first other state, and stops after two minutes. Closing the drawer stops
   the polling, not the job.
4. **The editor sends the whole recipe and omits the node.** The API takes a full
   recipe on PATCH and keeps the existing node when none is sent. A picker that
   offered a move would be offering something the screen has no reason to do.
5. **A picker never silently drops a value.** A tool, pack, baseline, or root the
   catalog no longer lists stays visible and selected, marked as unknown, so
   saving an unrelated field cannot delete it.
6. **Creating a recipe is not here.** That form belongs to the builder, which
   validates every field against the baseline and the catalogs before a row
   exists. This screen edits, builds, and runs.

## Wiring

`App.jsx` is not part of this package: the integrator adds the route and the
`features/AGENTS.md` row. The page exports a default component with no props.

Restyle (F25): `RecipesPage.jsx` uses the shared `PageHeader` (title, subtitle, actions) and a `HelpTip`; data flow is unchanged.
