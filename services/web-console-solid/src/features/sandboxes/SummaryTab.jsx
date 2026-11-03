// The Summary tab: the first thing an operator sees about one sandbox.
//
// Tiles for state, node, network mode and people; gauges and sparklines for CPU and memory from
// the sampler (every 5 s); the last refused destinations with a one-click "Allow 15 min"; the
// granted models; the first mounts; and who is connected right now.
import { For, Show, createSignal } from 'solid-js';
import * as deck from '../../api/deck.js';
import * as alertsApi from '../../api/alerts.js';
import * as mountsApi from '../../api/mounts.js';
import * as policies from '../../api/policies.js';
import StatTile from '../../components/StatTile.jsx';
import Gauge from '../../components/Gauge.jsx';
import Sparkline from '../../components/Sparkline.jsx';
import Countdown from '../../components/Countdown.jsx';
import PresenceDots from '../../components/PresenceDots.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import { useUi } from '../../state/ui.jsx';
import { modeLabel } from './overview.js';

const GIB = 1024 * 1024 * 1024;
const TREND_POINTS = 60;
const ALLOW_MINUTES = 15;
const MOUNTS_SHOWN = 5;

// Bytes as GiB with one decimal, the unit the gauges print.
function gib(bytes) {
  return Math.round(((Number(bytes) || 0) / GIB) * 10) / 10;
}

// Uptime seconds as "h min", or a dash.
function uptime(seconds) {
  if (seconds == null) return '—';
  const minutes = Math.round(seconds / 60);
  return minutes >= 60 ? `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, '0')} min` : `${minutes} min`;
}

// Props: sandbox, control (the control summary or null), revision, onTab(id).
export default function SummaryTab(props) {
  const ui = useUi();
  const id = () => props.sandbox?.id;
  const [series, setSeries] = createSignal([]);
  const [refused, setRefused] = createSignal([]);
  const [mounts, setMounts] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [allowing, setAllowing] = createSignal(null);

  usePolling(async () => {
    const [metrics, alerts, mountList] = await Promise.all([
      deck.metrics(id(), '1h').catch(() => null),
      alertsApi.list({ sandbox: id() }).catch(() => null),
      mountsApi.get(id()).catch(() => null),
    ]);
    return { metrics, alerts, mountList };
  }, {
    everyMs: 5000,
    onData: ({ metrics, alerts, mountList }) => {
      setSeries((metrics?.points || []).slice(-TREND_POINTS));
      setRefused((alerts?.items || []).slice(0, 5));
      setMounts(mountList);
      setError(null);
    },
    onError: (failure) => setError(failure.message),
  });

  const last = () => series()[series().length - 1];
  const spec = () => props.sandbox?.spec || {};
  const runtime = () => props.sandbox?.runtime || {};
  const cpuMax = () => Math.max(100, (spec().cpu_limit || 1) * 100);
  const memMax = () => last()?.mem_limit || 0;
  const egress = () => props.control?.egress || null;
  const people = () =>
    (Array.isArray(props.control?.presence) ? props.control.presence : []).map((entry) => ({
      name: entry.user_name || 'SSH',
      kind: entry.kind,
      view_only: entry.view_only,
    }));
  const models = () => (Array.isArray(props.control?.models) ? props.control.models : []);

  async function allow(alert) {
    setAllowing(alert.id);
    try {
      await policies.openWindow(alert.sandbox_id || id(), ALLOW_MINUTES, `alert ${alert.host}`, 'bypass', alert.channel || 'proxy');
      ui?.toast.show({ tone: 'ok', text: `${alert.host} is allowed for ${ALLOW_MINUTES} min.` });
      setRefused((list) => list.filter((entry) => entry.id !== alert.id));
    } catch (failure) {
      ui?.toast.show({ tone: 'bad', text: failure?.message || 'Could not open the window.' });
    } finally {
      setAllowing(null);
    }
  }

  return (
    <section class="tab-panel ac-summary">
      <Show when={error()}><p class="error">{error()}</p></Show>

      <div class="ac-tiles">
        <StatTile label="State" value={props.sandbox?.status || '—'} trend={`up ${uptime(runtime().uptime_seconds)}`}
          tone={props.sandbox?.status === 'running' ? 'ok' : props.sandbox?.status === 'failed' ? 'bad' : undefined} />
        <StatTile label="Node" value={props.control?.node?.name || props.sandbox?.node_name || '—'}
          trend={props.control?.node?.tunnel_state ? `tunnel ${props.control.node.tunnel_state}` : undefined} />
        <div class="stat-tile" data-tone={egress()?.mode === 'off_allow_all' ? 'bad' : undefined}>
          <span class="eyebrow">Network mode</span>
          <span class="figure">{modeLabel(egress()?.mode)}</span>
          <Show when={egress()?.revert_at}>
            <span class="muted small">back to {modeLabel(egress().revert_to)} in <Countdown until={egress().revert_at} /></span>
          </Show>
        </div>
        <div class="stat-tile" data-tone={props.control?.allow_all ? 'warn' : undefined}>
          <span class="eyebrow">Allow all</span>
          <span class="figure">{props.control?.allow_all ? 'on' : 'off'}</span>
          <Show when={props.control?.allow_all && props.control?.allow_all_until}>
            <span class="muted small">ends in <Countdown until={props.control.allow_all_until} /></span>
          </Show>
        </div>
        <div class="stat-tile">
          <span class="eyebrow">Connected</span>
          <span class="figure">{people().length}</span>
          <PresenceDots people={people()} />
        </div>
      </div>

      <div class="ac-grid">
        <div class="deck-card">
          <h3>CPU and memory</h3>
          <Show when={series().length} fallback={<p class="muted small">No samples yet. The runner reports every five seconds.</p>}>
            <div class="ac-gauges">
              <Gauge label="CPU" value={Math.round(last()?.cpu_pct_avg ?? 0)} max={Math.round(cpuMax())} unit="%" />
              <Gauge label="Memory" value={gib(last()?.mem_bytes_avg)} max={gib(memMax())} unit="GiB" />
            </div>
            <div class="ac-trends">
              <div><span class="eyebrow">CPU, last samples</span><Sparkline label="CPU trend" points={series().map((p) => p.cpu_pct_avg)} max={cpuMax()} /></div>
              <div><span class="eyebrow">Memory, last samples</span><Sparkline label="Memory trend" points={series().map((p) => p.mem_bytes_avg)} max={memMax() || undefined} /></div>
            </div>
          </Show>
        </div>

        <div class="deck-card">
          <h3>Last refused</h3>
          <Show when={refused().length} fallback={<p class="muted small">Nothing is waiting for an answer.</p>}>
            <ul class="ac-list">
              <For each={refused()}>
                {(alert) => (
                  <li>
                    <span class="mono small">{alert.host}:{alert.port}</span>
                    <span class="muted small"> {alert.channel} · {alert.count}×</span>
                    <button class="tiny secondary" disabled={allowing() === alert.id} onClick={() => allow(alert)}>
                      Allow {ALLOW_MINUTES} min
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>

        <div class="deck-card">
          <h3>Models</h3>
          <Show when={models().length} fallback={<p class="muted small">No model is granted to this sandbox.</p>}>
            <ul class="ac-list">
              <For each={models()}>
                {(model) => <li><span class="mono small">{model.model_name || model.name}</span>{model.is_default ? <span class="muted small"> default</span> : null}</li>}
              </For>
            </ul>
          </Show>
        </div>

        <div class="deck-card">
          <h3>Mounts</h3>
          <Show when={mounts()?.roots?.length} fallback={<p class="muted small">Nothing is mounted.</p>}>
            <ul class="ac-list">
              <For each={mounts().roots.slice(0, MOUNTS_SHOWN)}>
                {(root) => (
                  <li>
                    <span class="mono small">{root.mount_path}</span>
                    <span class="muted small"> {root.slug} · {root.read_only ? 'read-only' : 'read-write'}</span>
                  </li>
                )}
              </For>
            </ul>
            <Show when={mounts().roots.length > MOUNTS_SHOWN}>
              <button class="ghost tiny" onClick={() => props.onTab?.('mounts')}>All {mounts().roots.length} mounts</button>
            </Show>
          </Show>
        </div>

        <div class="deck-card">
          <h3>Open sessions</h3>
          <Show when={(props.control?.presence || []).length} fallback={<p class="muted small">Nobody is connected.</p>}>
            <ul class="ac-list">
              <For each={props.control.presence}>
                {(entry) => (
                  <li>
                    <span class="small">{entry.user_name || 'SSH'}</span>
                    <span class="muted small"> {entry.kind}{entry.view_only ? ' · watching' : ''}</span>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>

        <div class="deck-card">
          <h3>Health</h3>
          <p class="small"><StatusBadge status={props.sandbox?.health} /></p>
        </div>
      </div>
    </section>
  );
}
