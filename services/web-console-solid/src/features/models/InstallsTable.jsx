// The installs of one model: node, state, provider status, reserved memory, enabled switch, actions.
import { Show, createSignal } from 'solid-js';
import Drawer from '../../components/Drawer.jsx';
import Meter from '../../components/Meter.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import Switch from '../../components/Switch.jsx';
import { gib } from '../../components/ResourceBars.jsx';
import ModelRuntimePanel from './ModelRuntimePanel.jsx';

// Props: rows (installs of one model), model, onChange(row, patch, label), onUninstall(row),
// onChanged() (after a runtime action), selected, onSelect (optional, enables checkboxes).
export default function InstallsTable(props) {
  const [panel, setPanel] = createSignal(null);
  const reserve = (row) => (
    <div class="install-reserve">
      <Meter label="RAM" value={row.reserved_ram_bytes || 0} max={Math.max(row.max_ram_bytes || 0, row.reserved_ram_bytes || 0, 1)} text={gib(row.reserved_ram_bytes || 0)} />
      <Show when={row.reserved_vram_bytes}>
        <Meter label="VRAM" value={row.reserved_vram_bytes} max={Math.max(row.max_vram_bytes || 0, row.reserved_vram_bytes, 1)} text={gib(row.reserved_vram_bytes)} />
      </Show>
    </div>
  );
  const columns = [
    { key: 'node_name', label: 'Node', sortable: true, render: (r) => <span class="mono">{r.node_name || r.node_id}</span> },
    {
      key: 'state', label: 'State', sortable: true,
      render: (r) => (
        <>
          <StatusBadge status={r.state} />
          <Show when={r.state === 'failed' && r.error}><div class="muted small">{r.error}</div></Show>
        </>
      ),
    },
    { key: 'provider_status', label: 'Provider', render: (r) => (r.provider_status ? <StatusBadge status={r.provider_status} /> : <span class="muted">—</span>) },
    { key: 'reserved_ram_bytes', label: 'Reserved', render: reserve },
    {
      key: 'enabled', label: 'Enabled',
      render: (r) => (
        <Show when={r.state === 'installed'} fallback={<span class="muted">—</span>}>
          <Switch
            checked={r.enabled}
            label={r.enabled ? 'On' : 'Off'}
            onChange={(value) => props.onChange(r, { enabled: value }, value ? 'switching on' : 'switched off')}
          />
        </Show>
      ),
    },
    {
      key: 'actions', label: 'Actions',
      render: (r) => (
        <span class="row-actions">
          <button type="button" class="secondary tiny" onClick={() => setPanel({ row: r, view: 'logs' })}>Logs</button>
          <button type="button" class="secondary tiny" onClick={() => setPanel({ row: r, view: 'metrics' })}>Metrics</button>
          <button type="button" class="danger tiny" onClick={() => props.onUninstall(r)}>Uninstall from {r.node_name || 'node'}</button>
        </span>
      ),
    },
  ];
  return (
    <>
      <SelectTable
        columns={columns} rows={props.rows} rowKey="node_id"
        selectable={!!props.onSelect} selected={props.selected} onSelect={props.onSelect}
        empty={{ title: 'Not installed anywhere', hint: 'Use "Deploy on nodes…" to install this model.' }}
      />
      <Drawer
        open={!!panel()}
        title={panel() ? `${panel().row.model_name || panel().row.model_key} on ${panel().row.node_name}` : ''}
        subtitle={panel()?.view === 'metrics' ? 'Runtime metrics' : 'Runtime logs'}
        onClose={() => setPanel(null)}
      >
        <Show when={panel()}>
          <ModelRuntimePanel row={panel().row} node={panel().row.node_name} onChanged={() => props.onChanged?.()} />
        </Show>
      </Drawer>
    </>
  );
}
