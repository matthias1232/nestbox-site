# features/help/ — docs, help drawer, shortcuts, tour (F18)

| File | Responsibility |
|---|---|
| `DocsPage.jsx` | `/docs` and `/docs/<slug>`: document list with search, rendered document, table of contents, previous/next, "Edit on GitHub" |
| `HelpDrawer.jsx` | Drawer opened by `ui.help.open('slug#heading')`; "Can't find it?" searches docs, menu and inventory; restart tour; mounted once by the Shell |
| `ShortcutsSheet.jsx` | Shortcut dialog (`?`); also registers `?`, `d` and the `g x` jumps; mounted once by the Shell |
| `Tour.jsx` | Five-step first-run tour over the `data-tour` regions; marks `shell-v1` seen on finish or skip; mounted once by the Shell |
| `useHelp.js` | Cached help index and documents, `useIndex`, `useDoc`, `scrollToHeading`, `interceptDocLinks`, `trapFocus`, `SHORTCUTS`, `registerGlobalShortcuts`, `restartTour` |
| `docsSearch.js` | Pure `search(index, q)`, `groupByFirstTag`, `splitAnchor` (tests in `docsSearch.test.js`) |
| `tourSteps.js` | `TOUR_ID` and the five `TOUR_STEPS` with their `data-tour` selectors |

Styles live in `styles/help.css`. Documents are static files under `public/help/` (see `scripts/check-help.mjs`).
`[` / `]` (previous/next tab) are listed on the sheet but registered by pages that have tabs.
`mod+k` is registered by the palette host (F04).

`Tour.jsx` takes the spotlight from the region's real `getBoundingClientRect()` (inset 0),
measured immediately, again after two frames and 60 ms, and on `resize`, any scroll (the
region's own scrolling container included) and a `ResizeObserver` on `document.body`; every
listener and the observer are removed on cleanup. The card sits beside the region, below when
there is no room beside it, and never covers it; a missing region shows the card centred
without a spotlight. The shade and the spotlight pass clicks through, so the console stays
usable during a step and Escape ends the tour (V20).
