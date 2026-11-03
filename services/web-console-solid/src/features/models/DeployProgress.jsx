// After Finish: one row per target with a meter, an overall meter, the log tail and the cluster link.
import { Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import Meter from '../../components/Meter.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import * as jobsApi from '../../api/jobs.js';
import * as models from '../../api/models.js';
import { useLive } from '../../state/live.jsx';
import { navigate, paths } from '../../routing.js';
import { progressRows, overall, clusterIdOf } from './bulkDeploy.js';

const POLL_MS = 3000;
const TONE = { installed: 'ok', failed: 'bad', installing: 'warn', queued: 'muted', skipped: 'muted' };

// Props: jobId, modelKey, targets ([{ node, fit, skipped }]), onDone().
export default function DeployProgress(props) {
  const [job, setJob] = createSignal(null);
  const [logs, setLogs] = createSignal([]);
  const [installs, setInstalls] = createSignal([]);
  const [error, setError] = createSignal(null);
  const live = useLive();
  let timer = null;
  let alive = true;

  // Read the job, its log and this model's installs; stop polling once the job is final.
  async function tick() {
    try {
      const [j, l, i] = await Promise.all([
        jobsApi.get(props.jobId),
        jobsApi.logs(props.jobId, 200),
        models.installs(),
      ]);
      if (!alive) return;
      setJob(j);
      setLogs(l.items || []);
      setInstalls((i.items || []).filter((row) => row.model_key === props.modelKey));
      setError(null);
      if (['succeeded', 'failed', 'partial', 'cancelled'].includes(j.status) && timer) {
        clearInterval(timer);
        timer = null;
        props.onDone?.(j);
      }
    } catch (failure) {
      if (alive) setError(failure.message);
    }
  }

  onMount(() => {
    tick();
    timer = setInterval(tick, POLL_MS);
  });
  // A `job` event for our job means progress: read it now instead of waiting for the timer.
  const stop = live ? live.on('job', () => tick()) : () => {};
  onCleanup(() => {
    alive = false;
    stop();
    if (timer) clearInterval(timer);
  });

  const rows = createMemo(() => progressRows(props.targets, installs(), logs()));
  const total = createMemo(() => overall(rows()));
  const clusterId = () => clusterIdOf(job());

  const columns = [
    { key: 'node', label: 'Node', sortable: true, render: (r) => <span class="mono">{r.node}</span> },
    { key: 'state', label: 'State', sortable: true, render: (r) => <span class={`badge ${TONE[r.state]}`}><i />{r.state}</span> },
    { key: 'percent', label: 'Progress', render: (r) => <Meter label="" value={r.percent} max={100} text={`${r.percent}%`} /> },
    { key: 'tail', label: 'Last line', render: (r) => <span class="muted small">{r.tail || '—'}</span> },
  ];

  return (
    <div class="deploy-progress">
      <Show when={error()}><p class="error" role="alert">{error()}</p></Show>
      <Meter
        label={`Job ${job()?.status || 'starting'}`}
        value={total().done + total().failed}
        max={Math.max(1, total().total)}
        text={`${total().done} installed · ${total().failed} failed · ${total().total} total`}
      />
      <SelectTable columns={columns} rows={rows()} rowKey="node" empty={{ title: 'No targets', hint: 'The server skipped every node.' }} />
      <div class="deploy-progress-actions">
        <Show when={clusterId()}>
          <button type="button" class="primary" onClick={() => navigate(paths.cluster(clusterId()))}>Open cluster</button>
        </Show>
        <button type="button" class="secondary" onClick={() => navigate(paths.model(props.modelKey))}>Open model page</button>
      </div>
    </div>
  );
}
