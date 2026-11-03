# features/home/ — the home page (F05)

| File | Responsibility |
|---|---|
| `HomePage.jsx` | The whole Nestbox on one page: tiles, map, health, tasks, latest alerts; a smaller member view. Polls the inventory and jobs, and `GET /readyz` once a minute for the disk warning |
| `homeStats.js` | Pure `totals`, `health`, `diskHealth`, `graphFromTree`, `pathFor` over the inventory tree (C3); `diskHealth` turns free bytes below 15 GiB into the first health row |
| `homeStats.test.js` | Tests for `homeStats.js` |
| `HealthList.jsx` | Rows of things needing attention, each a link with a status badge |
| `QuickActions.jsx` | New sandbox, Add node, Deploy model, My sandboxes buttons |
