# features/home/ — the home page (F05)

| File | Responsibility |
|---|---|
| `HomePage.jsx` | The whole Nestbox on one page: tiles, map, health, tasks, latest alerts; a smaller member view |
| `homeStats.js` | Pure `totals`, `health`, `graphFromTree`, `pathFor` over the inventory tree (C3) |
| `homeStats.test.js` | Tests for `homeStats.js` |
| `HealthList.jsx` | Rows of things needing attention, each a link with a status badge |
| `QuickActions.jsx` | New sandbox, Add node, Deploy model, My sandboxes buttons |
