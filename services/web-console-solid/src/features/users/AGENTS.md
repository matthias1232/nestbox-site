# features/users/ — users and permissions

| File | Responsibility |
|---|---|
| `UsersPage.jsx` | Users & permissions page: People table, "This data center" permissions tab, New user drawer (F22) |
| `AccessSection.jsx` | Share a sandbox and invitation list (exists, embedded in People) |
| `PermissionsTab.jsx` | Permissions per scope (`organization`, `environment`, `sandbox`): table, add person, invite, withdraw |
| `InviteDialog.jsx` | Create an invitation link; shows it once in a copy box |
| `permissions.js` | Pure helpers: `rolesFor`, `viaLabel`, `canRemove`, `canChangeRole`, `roleSentence`, `addableUsers`, `inviteLink` |
| `permissions.test.js` | Tests for `permissions.js` |
