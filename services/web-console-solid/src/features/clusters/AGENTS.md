# features/clusters/ — model clusters (F09)

A model cluster is several installs of one model behind one provider (not the old gateway "Cluster health" page).

| File | Responsibility |
|---|---|
| `ClustersPage.jsx` | `/clusters`: table of clusters with health and endpoint, New cluster button |
| `ClusterDetailPage.jsx` | `/clusters/<id>`: members, rename/strategy form, add/remove member, delete (lists sandboxes on 409) |
| `NewClusterDialog.jsx` | Create dialog: name, model, strategy, members; nothing is written before Create |
| `useClusters.js` | `useClusterList()` and `useCluster(id)`: 10 s poll plus refresh on `models` events |
| `clusterHealth.js` | Pure helpers: `health`, `strategyLabel`, `canRemove`, `pickableInstalls`, `modelsWithInstalls`, `usersFromError` |
| `clusterHealth.test.js` | Tests for the helpers |

Usage card: `GET accounting/usage` has no provider filter, so the detail page only explains where usage is shown.
