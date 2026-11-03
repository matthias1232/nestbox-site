# catalog_sync/ — building the catalogue from the repositories

## Purpose

Keep the model catalogue current without a deployment, and without ever writing
over a number a human checked (ADR 0013, ADR 0019).

Two kinds of row live in `models`:

| Kind | Who writes it | May the sync change it |
|---|---|---|
| `pinned = true` | the seed migrations, by hand | **never** — not the numbers, not the name, not `synced_at` |
| `pinned = false`, `source <> 'seed'` | this module | yes, on every run |

The one exception is `nim_available`: it records that NVIDIA hosts a model as a
NIM, which is a fact about a remote listing rather than a catalogue figure, and
the NIM step sets it once (`false → true`) on a matching row. Nothing else about
a pinned row is touched.

## Sources

| Module | States | Adds rows |
|---|---|---|
| `huggingface.rs` | parameters, weight files, license, context, experts | yes |
| `ollama.rs` | one GGUF quant, exact layer sizes, parameter class, file type | yes |
| `ngc.rs` | NVIDIA's packaged models, artefact size, precision, format | yes |
| `nim.rs` | ids only | no — sets `nim_available` |

Each source is a separate module because each states a different subset of the
truth. A new repository becomes a new module with one `pub async fn fetch`, and
a line in `run`; it does not change the rule.

Rules every source keeps:

1. Every request has a timeout (10 s, set on the shared client and the request).
2. A source stops starting new requests after its budget (60 s) rather than
   holding the job open.
3. One repository that cannot be read is a count in `failed`, never the end of
   the source; one source that cannot be read is one line in the job log and
   never the end of the sync.
4. Only stated figures are carried. No parameter count is multiplied by an
   assumed dtype, and no missing number is estimated.

## The gate rule

`derive.rs` is the ADR 0013 rule as code, unit tested:

- `min_vram = weights + overhead`: 10 % above 100 GiB, 6–16 GiB below it;
- `recommended_vram = 1.25 × min`, rounded up to the next GiB;
- `min_gpu_count = ceil(min_vram / 80 GiB)`; above one card the row is also
  `unified_memory_only`, because a single pool can hold it and a discrete card
  cannot;
- a GGUF row also states a RAM gate (`file + 2 GiB` working set, mapped not
  copied); a safetensors row states none, because it has no CPU path.

**With no stated size there is no gate — and the row is still written.** The
sync stores it with null `min_vram_bytes` / `min_ram_bytes`, and
`domain::model::classify` answers `Fit::Unknown` with a reason that says the
requirement is not published, never `installable` (ADR 0019). `unknown` is a
first-class verdict: it is not in `fits()`, and it disables install. The gate
figures are never estimated to avoid this state.

## Running it

`start` is `POST /models/sync`: it creates a `catalog_sync` job and runs it in the
request, so the caller receives the per-source counts. `run` is the function a
scheduler calls with the job the daily `catalog-sync` schedule created. There is
one implementation and two triggers.

Each run writes one job-log line per source, counts first and in a fixed order:

```
huggingface: added 12, updated 0, skipped 3, pinned 0, failed 1
```

`added` / `updated` / `pinned` / `skipped` / `failed` come from the upsert
outcome. A row is never deleted: a model that disappears upstream keeps its row
and its last `synced_at`.

## Extending a source

1. Read only endpoints that state a size, a license or an id; never scrape a web
   page for a number.
2. Return `Candidate` with `weights_bytes: None` when the artefact states no
   size. Do not compute a size from a parameter count.
3. Add a unit test for the parsing (no test may touch the network) and a filter
   test for whatever allowlist the source needs.
4. Update this file and the `catalog_sync/` row of `../AGENTS.md`.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `mod.rs` | Module root: declares and re-exports the submodules. |
