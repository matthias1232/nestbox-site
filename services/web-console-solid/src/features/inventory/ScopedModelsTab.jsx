// Model installs of one node, or of a whole data center grouped by model.
import { createMemo, createSignal } from 'solid-js';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import * as models from '../../api/models.js';
import { navigate, paths } from '../../routing.js';
import { gib } from './inventoryTabs.js';

// Props: `nodeName` (optional: only this node), `nodeNames` (optional Set: only these nodes), `onDeploy` (button callback).
export default function ScopedModelsTab(props) {
  const [items, setItems] = createSignal([]);
  const [error, setError] = createSignal(null);
  usePolling(() => models.installs(props.nodeName), {
    everyMs: 10000,
    onData: (answer) => {
      setItems(Array.isArray(answer) ? answer : answer?.items || []);
      setError(null);
    },
    onError: (failure) => setError(failure.message),
  });
  const rows = createMemo(() => {
    const names = props.nodeNames;
    const list = names ? items().filter((row) => names.has(row.node_name)) : items();
    return list.map((row) => ({ ...row, rowId: `${row.model_key}@${row.node_name}` }));
  });
  const columns = [
    { key: 'model_name', label: 'Model', sortable: true, render: (row) => <span>{row.model_name || row.model_key}</span> },
    { key: 'node_name', label: 'Node', sortable: true, render: (row) => <span class="mono small">{row.node_name}</span> },
    { key: 'state', label: 'State', sortable: true, render: (row) => <StatusBadge status={row.state} /> },
    { key: 'provider_status', label: 'Provider', render: (row) => <StatusBadge status={row.provider_status || 'unknown'} /> },
    { key: 'reserved_vram_bytes', label: 'Reserve', align: 'right', render: (row) => <span class="mono">{gib(row.reserved_vram_bytes ?? row.reserved_bytes)}</span> },
    {
      key: 'links',
      label: 'Details',
      render: (row) => (
        <span class="inv-row-actions">
          <button type="button" class="ghost tiny" aria-label={`Logs of ${row.model_key}`} onClick={() => navigate(`${paths.model(row.model_key)}?tab=logs`)}>Logs</button>
          <button type="button" class="ghost tiny" aria-label={`Metrics of ${row.model_key}`} onClick={() => navigate(`${paths.model(row.model_key)}?tab=metrics`)}>Metrics</button>
        </span>
      ),
    },
  ];
  return (
    <div class="inv-tab">
      <div class="controls">
        <button type="button" class="primary" onClick={() => props.onDeploy?.()}>Deploy a model here</button>
        <button type="button" class="secondary" onClick={() => navigate(paths.menu('clusters'))}>Model clusters</button>
      </div>
      {error() && <p class="error">{error()}</p>}
      <SelectTable
        columns={columns}
        rows={rows()}
        rowKey="rowId"
        empty={{ title: 'No model installed here', hint: 'Deploy a model to run it on these nodes.' }}
      />
    </div>
  );
}
