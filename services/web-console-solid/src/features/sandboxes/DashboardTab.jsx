// Dashboard tab: what this sandbox is doing right now.
//
// Status the control plane believes next to what the runtime observed, live usage
// from the sampler, the busiest processes, recent blocks, and the controls for
// its resources. Everything refreshes every five seconds while the tab is open.
import { For, Show, createSignal } from 'solid-js';
import * as deck from '../../api/deck.js';
import { formatBytes } from '../../components/Meter.jsx';
import Gauge from '../../components/Gauge.jsx';
import Sparkline from '../../components/Sparkline.jsx';
import HostLoad from '../../components/HostLoad.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import ResourcesPanel from './ResourcesPanel.jsx';
import '../../styles/deck.css';

export default function DashboardTab(props) {
  const id = () => props.sandbox?.id;
  const [series, setSeries] = createSignal([]);
  const [procs, setProcs] = createSignal(null);
  const [blocks, setBlocks] = createSignal([]);
  const [mcp, setMcp] = createSignal([]);
  const [error, setError] = createSignal(null);

  usePolling(async () => {
    const [m, p, t, s] = await Promise.all([
      deck.metrics(id(), '1h').catch(() => null),
      deck.processes(id()).catch(() => null),
      deck.traffic(id(), { window: '1h', filter: 'blocked', limit: 5 }).catch(() => null),
      deck.mcpAttached(id()).catch(() => null),
    ]);
    return { m, p, t, s };
  }, {
    everyMs: 5000,
    onData: ({ m, p, t, s }) => {
      setSeries(m?.points || []);
      setProcs(p);
      setBlocks(t?.items || []);
      setMcp(s?.items || []);
      setError(null);
    },
    onError: (e) => setError(e.message),
  });

  const last = () => series()[series().length - 1];
  const runtime = () => props.sandbox?.runtime || {};
  const spec = () => props.sandbox?.spec || {};
  const memoryLimitBytes = () => { const m = String(spec().memory_limit || '').match(/^(\d+)b$/); return m ? Number(m[1]) : null; };

  return (
    <section class="tab-panel">
      <Show when={error()}><p class="error">{error()}</p></Show>
      <HostLoad />
      <div class="deck-grid">
        <div class="deck-card">
          <h3>Status</h3>
          <dl class="deck-facts">
            <dt>Lifecycle</dt><dd><StatusBadge status={props.sandbox?.status} /></dd>
            <dt>Health</dt><dd><StatusBadge status={props.sandbox?.health} /></dd>
            <dt>Pack</dt><dd class="mono">{props.sandbox?.pack_key}</dd>
            <dt>Image</dt><dd class="mono small">{props.sandbox?.image}</dd>
            <dt>Node</dt><dd>{props.sandbox?.node_name || '—'}</dd>
            <dt>Network address</dt><dd class="mono">{runtime().ip || '—'} <span class="muted small">(inside the sandbox network)</span></dd>
            <dt>Uptime</dt><dd>{runtime().uptime_seconds != null ? `${Math.round(runtime().uptime_seconds / 60)} min` : '—'}</dd>
          </dl>
        </div>

        <div class="deck-card">
          <h3>CPU</h3>
          <Gauge label="Now" value={Math.round(last()?.cpu_pct_avg ?? 0)} max={Math.round(Math.max(100, (spec().cpu_limit || 1) * 100))} unit="%" />
          <p class="muted small">{(last()?.cpu_pct_avg ?? 0).toFixed(1)}% · limit {spec().cpu_limit ?? '—'} cores</p>
          <Sparkline label="CPU trend" width={300} height={48} points={series().map((p) => p.cpu_pct_avg)} />
          <Show when={series().length === 0}><p class="muted small">No samples yet. The runner reports every five seconds.</p></Show>
        </div>

        <div class="deck-card">
          <h3>Memory</h3>
          <Gauge label="Now" value={Math.round(((last()?.mem_bytes_avg ?? 0) / 1073741824) * 10) / 10} max={Math.round(((last()?.mem_limit || 0) / 1073741824) * 10) / 10} unit="GiB" />
          <p class="muted small">{formatBytes(last()?.mem_bytes_avg)} of {formatBytes(last()?.mem_limit)}</p>
          <Sparkline label="Memory trend" width={300} height={48} points={series().map((p) => p.mem_bytes_avg)} max={last()?.mem_limit} />
        </div>

        <div class="deck-card">
          <h3>Network and disk (last hour)</h3>
          <dl class="deck-facts">
            <dt>Received</dt><dd class="mono">{formatBytes(last()?.net_rx)}</dd>
            <dt>Sent</dt><dd class="mono">{formatBytes(last()?.net_tx)}</dd>
            <dt>Disk read</dt><dd class="mono">{formatBytes(last()?.blk_read)}</dd>
            <dt>Disk written</dt><dd class="mono">{formatBytes(last()?.blk_write)}</dd>
            <dt>Processes</dt><dd class="mono">{last()?.pids_max ?? '—'}</dd>
          </dl>
        </div>

        <div class="deck-card">
          <h3>Busiest processes</h3>
          <Show when={procs()?.items?.length} fallback={<p class="muted small">No process table.</p>}>
            <table class="dense"><tbody>
              <For each={[...procs().items].sort((a, b) => Number(b.cpu_pct) - Number(a.cpu_pct)).slice(0, 5)}>
                {(row) => <tr><td class="mono small">{row.pid}</td><td class="small">{row.cpu_pct}%</td><td class="mono small">{String(row.command).slice(0, 48)}</td></tr>}
              </For>
            </tbody></table>
          </Show>
        </div>

        <div class="deck-card">
          <h3>Last blocks</h3>
          <Show when={blocks().length} fallback={<p class="muted small">Nothing was blocked in the last hour.</p>}>
            <For each={blocks()}>{(e) => <p class="small mono">{String(e.ts).slice(11, 19)} {e.host}:{e.port} <span class="muted">{e.reason}</span></p>}</For>
          </Show>
        </div>

        <div class="deck-card">
          <h3>MCP servers</h3>
          <Show when={mcp().length} fallback={<p class="muted small">No MCP server is attached.</p>}>
            <For each={mcp()}>{(s) => <p class="small"><span class="mono">{s.server_key}</span> <StatusBadge status={s.status} /></p>}</For>
          </Show>
        </div>
      </div>

      <h3>Resources</h3>
      <p class="muted small">Change what this sandbox may use. Limits apply to the running sandbox at once.</p>
      <ResourcesPanel sandbox={props.sandbox} reload={props.reload} />
    </section>
  );
}
