// A table of the sandboxes of one node, site or data center, with Open and Workbox actions.
// Selecting rows hands the ids to the shared selection store, which the bulk bar (F21) reads.
import { createMemo, createSignal, onCleanup } from 'solid-js';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import { useUi } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';

// Props: `load` (function returning a promise of a list or `{ items }`), `filter` (optional row predicate),
// `emptyHint` (text).
export default function ScopedSandboxesTab(props) {
  const ui = useUi();
  const [items, setItems] = createSignal([]);
  const [error, setError] = createSignal(null);
  usePolling(() => props.load(), {
    everyMs: 10000,
    onData: (answer) => {
      const list = Array.isArray(answer) ? answer : answer?.items || [];
      setItems(list.map((entry) => entry.sandbox || entry));
      setError(null);
    },
    onError: (failure) => setError(failure.message),
  });
  onCleanup(() => ui?.selection.clear());

  const rows = createMemo(() => (props.filter ? items().filter(props.filter) : items()));
  const selected = () => (ui?.selection.kind() === 'sandbox' ? ui.selection.ids() : new Set());

  const columns = [
    { key: 'name', label: 'Name', sortable: true, render: (row) => <span class="mono">{row.name}</span> },
    { key: 'status', label: 'Status', sortable: true, render: (row) => <StatusBadge status={row.status} /> },
    { key: 'owner_name', label: 'Owner', sortable: true, render: (row) => row.owner_name || <span class="muted">shared</span> },
    { key: 'pack_key', label: 'Pack', sortable: true, render: (row) => <span class="mono small">{row.pack_key || '–'}</span> },
    { key: 'egress_mode', label: 'Egress mode', render: (row) => (row.egress_mode ? <span class="mono small">{String(row.egress_mode).replaceAll('_', ' ')}</span> : '–') },
    { key: 'allow_all', label: 'Allow-all', render: (row) => (row.allow_all ? <span class="badge bad"><i />allow-all</span> : <span class="muted">no</span>) },
    {
      key: 'actions',
      label: 'Actions',
      render: (row) => (
        <span class="inv-row-actions">
          <button type="button" class="ghost tiny" aria-label={`Open ${row.name}`} onClick={() => navigate(paths.sandbox(row.id))}>Open</button>
          <button type="button" class="ghost tiny" aria-label={`Workbox of ${row.name}`} onClick={() => navigate(paths.box(row.id))}>Workbox</button>
        </span>
      ),
    },
  ];

  return (
    <div class="inv-tab">
      {error() && <p class="error">{error()}</p>}
      <SelectTable
        columns={columns}
        rows={rows()}
        rowKey="id"
        selectable
        selected={selected()}
        onSelect={(set) => ui?.selection.set('sandbox', [...set])}
        empty={{ title: 'No sandboxes here', hint: props.emptyHint || 'Create one from the Sandboxes page.' }}
      />
    </div>
  );
}
