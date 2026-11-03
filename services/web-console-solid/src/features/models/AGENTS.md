# models/ — the model catalog and hardware fit

## Purpose

Answer three questions in order: **what does this machine have**, **what does a
model need**, and **where is the gap**. The screen exists so that nobody clicks
Install on a model the API will refuse.

## Rules

- The fit verdict, its reason, and the resource bars come from the server
  (`fit.fit`, `fit.reason`, `fit.bars`). This screen renders and filters them; it
  never recomputes a verdict. If it did, the badge and the install gate could
  disagree, and the user would see "can install" on a model the API refuses.
- The client may only do what the server allows: count verdicts, sort, and
  switch a filter on. `FITS` in `ModelsPage.jsx` mirrors `Fit::fits` for counting
  and must be changed with it, in the same commit.
- A cloud model is labelled as such. It is not installed here, and confusing the
  two makes the GPU story a lie.
- Install and uninstall are jobs. The screen shows the job, not a spinner that
  claims success.
- The download does not exist yet (Phase 2). The page says so in plain words
  instead of implying that an install job fetches the weights.
- "Probe hardware again" is the primary action, and a successful probe switches
  the filter to "fits this machine" — that is the flow the operator asked for:
  probe, see what changed, see what now fits.

## Layout

1. `CapacityOverview` — CPU model and cores, system RAM, GPU memory or the
   unified pool, each memory bar scaled against the largest requirement in the
   loaded catalog.
2. Filter chips (all / fits / does not fit / runs without a GPU / needs unified
   memory) with live counts, plus family, publisher, runtime, and text search.
3. The table: model and licence, size and format, minimum and recommended need,
   `ModelFitCell` (badge, two-track bars, shortfall), actions.

## Not here

No fetching of model weights, no progress bar for a download, no client-side
VRAM arithmetic.

## Files

| File | What it shows |
|---|---|
| `ModelsPage.jsx` | the catalogue, the install table, the filters |
| `ModelRuntimePanel.jsx` | one installed model's flags, logs and metrics |
| `ModelGrantsSection.jsx` | environment and user model grants (admins only), below the page |

`ModelPage.jsx` (`/models/<key>`) and `BulkDeployWizard.jsx` are placeholders created in Wave 0; package F08 builds them.
