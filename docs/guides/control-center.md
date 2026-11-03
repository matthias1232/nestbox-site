# Control Center guide

The Control Center is the Nestbox console organised around where things run. It does not
repeat the in-console help; each task below links to the page under `/docs/<slug>` in the console
(source: `services/web-console-solid/public/help/<slug>.md`).

## The tree

The left side shows one tree of the whole estate: organisation, data centers, sites, nodes and
the sandboxes on them. Selecting a branch opens its page (`/dc/<id>`, `/site/<id>`, `/node/<id>`).
The search box (Ctrl+K) finds nodes, sandboxes, models and policies by name.
See `/docs/tenant-tree`.

## The three views

The tree can be shown three ways; the choice is remembered per browser:

| View | Groups by | Use it to |
|---|---|---|
| Infrastructure | data center, site, node | see capacity and health of machines |
| Workloads | environment, sandbox | find and act on sandboxes |
| Models | model, model cluster, install | see where each model runs |

Rows can be selected for a bulk action (stop, start, policy, deploy a model).

## Roles

| Role | Can |
|---|---|
| viewer | read everything permitted to them |
| member | use and change their own sandboxes (and ones granted to them) |
| operator | manage nodes, providers, rented machines, system containers |
| admin | additionally manage users and permissions |

Hiding a menu entry is a convenience; the API enforces the same rules.

## Where to find each task

| Task | Page | Doc slug |
|---|---|---|
| Sign in and look around | Home | `getting-started` |
| Understand a term | Docs | `glossary` |
| Open a terminal, desktop or files | Sandbox workbox | `workbox` |
| Deploy a model on one node | Models | `deploy-a-model` |
| Deploy a model on many nodes | Models, bulk deploy | `deploy-on-many-nodes` |
| Share a model across nodes round-robin | Model clusters | `model-cluster-round-robin` |
| Add, drain or quick-deploy a node | Nodes | `nodes` |
| Move a sandbox to another node | Moves | `moves` |
| See what a sandbox's folders map to | Persistent data, mounts | `mounts` |
| Decide what a sandbox may reach | Policies | `policies`, `egress-modes` |
| Watch traffic, logs and alerts | Live traffic, Live logs, Alerts | `live-logs-and-alerts` |
| Organise data centers and sites | Data centers & sites | `tenant-tree` |
| Learn the keyboard | Shortcuts | `shortcuts` |

## How to add a document

1. Create `services/web-console-solid/public/help/<slug>.md` with a `# Title` line, a one-sentence
   summary paragraph and `##` sections. Use English only.
2. Add tags in the front matter as the existing files do.
3. Run `node scripts/check-help.mjs` in `services/web-console-solid`; it rebuilds the checks for
   `index.json` (title, tags, headings) and fails on broken links.
4. Link it from related pages in a "Related" section.

## How to render assets

Illustrations (login, empty states, doc headers) are generated, not drawn. Every asset has its exact
prompt next to it in `services/web-console-solid/public/assets/prompts/<name>.txt`; the table in
`services/web-console-solid/public/illustrations/AGENTS.md` lists asset, prompt, size and tool.
To render again: use the `render_image` tool with that prompt (1600x900 for wide images, 1024x1024
for empty states), keep every WebP under 300 KB and inspect the result for stray text or logos
before copying it to `public/illustrations/<name>.webp`. The optional login loop uses
`render_video` (3 s, 16:9, 720p, under 2 MB).
