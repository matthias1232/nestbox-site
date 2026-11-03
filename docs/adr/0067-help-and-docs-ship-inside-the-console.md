# ADR 0067 — Help and documentation ship inside the console

Status: Accepted
Date: 2026-11-02


## Context

The console now has a tree, bulk actions, clusters and wizards. People cannot find things
by guessing, and an appliance is often installed where the public website is not
reachable. Documentation that lives elsewhere is out of date the day the console changes,
and cannot be linked from the exact control that needs explaining.

## Decision

* **Documentation is markdown in the console's public folder** (`public/help/<slug>.md`),
  each file with a front matter block: `title`, `slug`, `tags`, `anchors`. It is served as
  static text by the same nginx as the console.
* **It is indexed at build time.** `scripts/check-help.mjs` generates `help/index.json`
  (slug, title, tags, summary, headings) and fails the build on a missing front matter
  field, a dangling `/docs/<slug>` link, or an image outside `/illustrations/`.
* **It is rendered by a small renderer of about 140 lines** that supports headings,
  lists, code, tables, links, images and callouts, and nothing else. No markdown library:
  the renderer is a size and safety decision (it never emits raw HTML).
* **It is anchored from the UI.** A `HelpTip` names `slug#heading` and opens a drawer at
  that place; the command palette searches the index client-side; `/docs` is the full
  reader. Illustrations are served from `/illustrations/`, not `/assets/`, which nginx
  caches for a year.
* **English only**, like the code and the rest of the documentation.
* **The first-run tour is one row per user** (`ui_tours_seen`, written through
  `PUT /users/me/tours/{tour}`), so it is shown once per person, not once per browser.

## Why not an external site

An air-gapped appliance cannot reach it; versions drift from the console they describe; a
link from a control to a heading cannot be checked at build time. Files in the same
repository change with the feature and fail the build when they rot.

## Alternatives rejected

* **Documentation only on the public website.** Fine for marketing, wrong for operating.
* **A markdown library.** More bytes and a wider attack surface than the subset needs.
* **Tour state in `localStorage`.** Lost on a new browser, and the rule is that
  `localStorage` holds only layout preferences.

## Consequences

A feature without a help page is visible as a missing anchor in review. Help files are
static, so they carry no user data and need no permission. The index is rebuilt with the
console and always matches the files shipped.

## Not built

Translations, versioned docs per release, and server-side full-text search.

## Invalidation conditions

If the documentation outgrows a client-side index (hundreds of pages), search moves to a
generated inverted index served as static JSON; the files and anchors stay.
