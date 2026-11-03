// The Summary tab of a node: four gauges, tiles, the tunnel card and the facts table.
import { Show } from 'solid-js';
import Gauge from '../../components/Gauge.jsx';
import StatTile from '../../components/StatTile.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { ago, gib, toGib } from './inventoryTabs.js';

// Props: `summary` (the answer of GET /nodes/{id}/summary).
export default function NodeSummaryTab(props) {
  const s = () => props.summary;
  const load = () => s()?.load || null;
  const node = () => s()?.node || {};
  const tunnel = () => s()?.tunnel || {};
  const installs = () => s()?.installs || [];
  const images = () => s()?.images || {};
  const box = () => s()?.sandboxes || {};
  return (
    <div class="inv-tab">
      <Show when={!load()}>
        <p class="warn-note"><b>Runner unreachable.</b> The node's runner did not answer, so live load is not shown. Stored facts are below.</p>
      </Show>
      <Show when={load()}>
        <section class="inv-gauges" aria-label="Load">
          <Gauge label="CPU" value={Math.round(load().cpu_percent || 0)} max={100} unit="%" />
          <Gauge label="RAM" value={toGib(load().ram_used_bytes)} max={toGib(load().ram_total_bytes)} unit="GiB" />
          <Gauge label="VRAM" value={toGib(load().vram_used_bytes)} max={toGib(load().vram_total_bytes)} unit="GiB" />
          <Gauge label="Disk" value={toGib(load().disk_used_bytes)} max={toGib(load().disk_total_bytes)} unit="GiB" />
        </section>
      </Show>

      <section class="inv-tiles">
        <StatTile label="Sandboxes running" value={box().running ?? 0} unit={`/ ${box().total ?? 0}`} />
        <StatTile label="Allow-all" value={box().allow_all ?? 0} tone={box().allow_all ? 'warn' : undefined} trend={box().allow_all ? 'open to the internet' : 'none open'} />
        <StatTile label="Model installs" value={installs().length} />
        <StatTile label="Images present" value={images().present ?? 0} trend={`${images().missing ?? 0} missing · ${images().pulling ?? 0} pulling · ${images().failed ?? 0} failed`} tone={images().failed ? 'bad' : undefined} />
      </section>

      <div class="inv-columns">
        <section class="card inv-card">
          <h2 class="eyebrow">Tunnel</h2>
          <dl class="facts">
            <dt>State</dt>
            <dd>
              <StatusBadge status={tunnel().state === 'up' ? 'running' : tunnel().state === 'stale' ? 'degraded' : tunnel().state ? 'offline' : 'unknown'} label={tunnel().state || 'unknown'} />
            </dd>
            <dt>Last handshake</dt><dd>{ago(tunnel().handshake_at)}</dd>
            <dt>Received</dt><dd class="mono">{gib(tunnel().rx_bytes)}</dd>
            <dt>Sent</dt><dd class="mono">{gib(tunnel().tx_bytes)}</dd>
            <dt>Egress lock</dt><dd>{tunnel().egress_lock === 'exit_local' ? 'Exit locally' : tunnel().egress_lock === 'strict' ? 'Strict' : '–'}</dd>
          </dl>
        </section>

        <section class="card inv-card">
          <h2 class="eyebrow">Facts</h2>
          <dl class="facts">
            <dt>Kind</dt><dd>{String(node().kind || '–').replaceAll('_', ' ')}</dd>
            <dt>Site</dt><dd>{s()?.site?.name || '–'} <Show when={s()?.site?.kind}><span class="badge muted"><i />{s().site.kind}</span></Show></dd>
            <dt>Data center</dt><dd>{s()?.data_center?.name || '–'}</dd>
            <dt>Runner URL</dt><dd class="mono small">{node().runner_url || node().endpoint || '–'}</dd>
            <dt>Overlay IP</dt><dd class="mono small">{node().overlay_ip || node().address || '–'}</dd>
            <dt>GPU</dt><dd>{node().gpu_name || (node().container_gpu ? 'available' : 'none')}</dd>
            <dt>Joined</dt><dd class="mono small">{node().joined_at || node().created_at || '–'}</dd>
          </dl>
        </section>
      </div>
    </div>
  );
}
