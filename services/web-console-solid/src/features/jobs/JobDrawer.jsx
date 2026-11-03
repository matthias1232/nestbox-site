// The detail of one task: facts, what it was asked to do, per-target results, the raw log, retry.
import { For, Show, createSignal, onCleanup } from 'solid-js';
import Drawer from '../../components/Drawer.jsx';
import LogView from '../../components/LogView.jsx';
import Meter from '../../components/Meter.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import { useLive } from '../../state/live.jsx';
import * as jobsApi from '../../api/jobs.js';
import * as bulkApi from '../../api/bulk.js';
import * as modelsApi from '../../api/models.js';
import { durationText, failedTargets, hasTargets, labelFor, logText, parseTargets, summaryLine } from './jobLabels.js';

// What the task was asked to do, as short "name: value" facts (never the whole payload).
function payloadFacts(job) {
  const p = job.payload || {};
  const facts = [];
  if (p.action) facts.push(['Action', String(p.action)]);
  if (p.model_key) facts.push(['Model', String(p.model_key)]);
  if (Array.isArray(p.ids)) facts.push(['Sandboxes', String(p.ids.length)]);
  if (Array.isArray(p.nodes)) facts.push(['Nodes', String(p.nodes.length)]);
  if (p.to_node) facts.push(['To node', String(p.to_node)]);
  return facts;
}

// The retry call for a bulk job and its failed targets, or null when the payload cannot support one.
function retryPlan(job, failed) {
  const p = job.payload || {};
  if (failed.length === 0) return null;
  if (job.kind === 'sandbox_bulk_action' && p.action) {
    return () => bulkApi.sandboxes(failed, p.action, p.params || {});
  }
  if (job.kind === 'model_bulk_install' && p.model_key && Array.isArray(p.nodes)) {
    return () => modelsApi.installBulk(p.model_key, { nodes: failed, force: false });
  }
  return null;
}

// Props: job (summary row or null), onClose, onRetried(newJobId).
export default function JobDrawer(props) {
  const [detail, setDetail] = createSignal(null);
  const [rows, setRows] = createSignal([]);
  const [error, setError] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const live = useLive();
  const current = () => detail() || props.job;

  const poll = usePolling(async () => {
    if (!props.job) return null;
    const [job, logs] = await Promise.all([jobsApi.get(props.job.id), jobsApi.logs(props.job.id, 500)]);
    return { job, logs: logs.items || [] };
  }, {
    everyMs: 4000,
    onData: (data) => { if (data) { setDetail(data.job); setRows(data.logs); setError(''); } },
    onError: (failure) => setError(failure.message),
  });
  onCleanup(live.on('job', () => poll.refresh()));

  const targets = () => parseTargets(rows());
  const failed = () => failedTargets(rows());
  const retry = () => (current() ? retryPlan(current(), failed()) : null);

  async function runRetry() {
    const call = retry();
    if (!call) return;
    setBusy(true);
    try {
      const answer = await call();
      props.onRetried?.(answer.job_id);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  const columns = [
    { key: 'target', label: 'Target', sortable: true, render: (t) => <span class="mono small">{t.target}</span> },
    { key: 'ok', label: 'Result', sortable: true, render: (t) => <StatusBadge status={t.ok ? 'succeeded' : 'failed'} label={t.ok ? 'ok' : 'failed'} /> },
    { key: 'message', label: 'Message', render: (t) => <span class="small">{t.message}</span> },
  ];

  return (
    <Drawer open={!!props.job} title={props.job ? labelFor(props.job.kind) : ''} subtitle={current() ? summaryLine(current()) : ''} onClose={props.onClose}>
      <Show when={current()}>
        {(job) => (
          <div class="job-drawer">
            <Show when={error()}><p class="error">{error()}</p></Show>
            <dl class="job-facts">
              <dt>Status</dt><dd><StatusBadge status={job().status} /></dd>
              <dt>Started</dt><dd class="mono small">{job().started_at || job().created_at || '—'}</dd>
              <dt>Duration</dt><dd class="mono small">{durationText(job()) || '—'}</dd>
              <For each={payloadFacts(job())}>{([name, value]) => (<><dt>{name}</dt><dd class="mono small">{value}</dd></>)}</For>
            </dl>
            <Meter label="Progress" value={(job().completed || 0) + (job().failed || 0)} max={job().total || 0} text={summaryLine(job())} />
            <Show when={job().error}><p class="error">{job().error}</p></Show>

            <Show when={hasTargets(job().kind) || targets().length > 0}>
              <h3 class="eyebrow">Results per target</h3>
              <SelectTable columns={columns} rows={targets()} rowKey="target" empty={{ title: 'No results yet', hint: 'Each target appears here when it finishes.' }} />
              <Show when={retry()}>
                <div class="job-retry">
                  <button class="primary" disabled={busy()} onClick={runRetry}>Retry {failed().length} failed</button>
                </div>
              </Show>
            </Show>

            <h3 class="eyebrow">Log</h3>
            <LogView text={logText(rows())} empty="No log entries." />
          </div>
        )}
      </Show>
    </Drawer>
  );
}
