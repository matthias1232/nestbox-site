// Pure logic of the shared terminal sessions: which tabs exist for this person,
// and how to attach one from a machine of your own.
//
// The tab list has two sources. `api/sandboxes.js terminalTabs` returns the tabs
// this user keeps in the sandbox; `api/presence.js tabsAll` returns every user's
// tabs with who is sitting in which. Merging them is a fixture of shapes, not of
// the DOM, so it lives here and is tested without a browser (`sessions.test.js`).

// The ssh user the sandbox's door answers for. `connect.ssh_user` says the same;
// a caller that knows better passes it as the fourth argument.
const DEFAULT_USER = 'nestbox';

// The people of one tab, in the shape `components/PresenceDots.jsx` reads.
// A tab the server reports without viewers still names the person who made it,
// otherwise a shared tab looks like nobody's.
function peopleOf(viewers, creator) {
  const list = (viewers || [])
    .filter((viewer) => viewer)
    .map((viewer) => ({
      name: viewer.user_name || 'someone',
      kind: viewer.kind || 'terminal',
      view_only: viewer.view_only === true,
    }));
  if (list.length) return list;
  if (creator) return [{ name: creator, kind: 'terminal', view_only: false }];
  return [];
}

// My list merged with everybody's: my order first, then the tabs only others
// have, each tab exactly once and keeping my title when I named it.
//
// `mine` items are `{id, label, alive}` (the server's per-user list);
// `all` items are `{tab_id, title, alive, created_by_name, viewers}`.
// The answer carries `mine`, so the caller knows which tabs it may drop from its
// own list without touching anybody else's shell.
export function mergeTabs(mine, all) {
  const known = new Map();
  for (const item of all || []) {
    if (item?.tab_id) known.set(item.tab_id, item);
  }
  const out = [];
  const seen = new Set();
  for (const item of mine || []) {
    if (!item?.id || seen.has(item.id)) continue;
    seen.add(item.id);
    const shared = known.get(item.id);
    out.push({
      id: item.id,
      label: item.label || shared?.title || item.id,
      // The all-user list is the fresher word on whether the shell is running.
      alive: shared ? shared.alive !== false : item.alive !== false,
      mine: true,
      created_by_name: shared?.created_by_name || null,
      viewers: shared?.viewers || [],
      people: peopleOf(shared?.viewers, shared?.created_by_name),
    });
  }
  for (const item of all || []) {
    if (!item?.tab_id || seen.has(item.tab_id)) continue;
    seen.add(item.tab_id);
    out.push({
      id: item.tab_id,
      label: item.title || item.tab_id,
      alive: item.alive !== false,
      mine: false,
      created_by_name: item.created_by_name || null,
      viewers: item.viewers || [],
      people: peopleOf(item.viewers, item.created_by_name),
    });
  }
  return out;
}

// The line that attaches this tab from a terminal of your own, or null while the
// sandbox has no address or no SSH door: a command with a hole in it is worse
// than an honest "not yet".
export function sshCommand(host, port, tab, user = DEFAULT_USER) {
  if (!host || !port || !tab) return null;
  return `ssh -t ${user || DEFAULT_USER}@${host} -p ${port} attach ${tab}`;
}

// The same connection for PuTTY, whose window asks for the fields one by one.
export function puttyHint(host, port, tab) {
  if (!host || !port || !tab) return null;
  return `PuTTY: Host ${host}, Port ${port}, Remote command: attach ${tab}`;
}
