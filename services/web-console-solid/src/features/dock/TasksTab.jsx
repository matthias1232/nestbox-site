// Tasks tab of the dock: running and recent jobs with progress, status and the last log line.
import { For, Show, createSignal, onCleanup } from 'solid-js';
import Meter from '../../components/Meter.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import { useLive } from '../../state/live.jsx';
import * as jobsApi from '../../api/jobs.js';
import { navigate } from '../../routing.js';
import { labelFor, summaryLine } from '../jobs/jobLabels.js';


export default function TasksTab() {
  const [jobs, setJobs] = createSignal([]);
  const [lastLine, setLastLine] = createSignal({});
  const [error, setError] = createSignal('');
  const live = useLive();

  async function loadLines(list) {
    const running = list.filter((job) => job.status === 'running').slice(0, 5);
    for (const job of running) {
      try {
        const result = await jobsApi.logs(job.id, 1);
        const rows = result.items || [];
        const row = rows[rows.length - 1];
        if (row) setLastLine((map) => ({ ...map, [job.id]: row.message }));
      } catch {
        // A missing log line is not worth an error.
      }
    }
  }
  // A job the acceptance script starts is its own test subject, not the operator's work:
  // `params.source == "acceptance"` rides along in the payload, so such a job never enters
  // this list (and its log lines are not polled either).
  const visible = (job) => !(job.payload && job.payload.source === 'acceptance');
  const poll = usePolling(() => jobsApi.list(20), {
    everyMs: 5000,
    onData: (result) => { const list = (result.items || []).filter(visible); setJobs(list); setError(''); loadLines(list); },
    onError: (failure) => setError(failure.message),
  });
  onCleanup(live.on('job', () => poll.refresh()));

  const line = (job) => lastLine()[job.id] || job.error || '';

  return (
    <Show
      when={jobs().length}
      fallback={<p class="muted">{error() || 'No tasks yet. Bulk actions, drains and deployments appear here with their progress.'}</p>}
    >
      <table class="dock-table">
        <thead><tr><th>Task</th><th>Status</th><th>Progress</th><th>Last message</th><th /></tr></thead>
        <tbody>
          <For each={jobs()}>
            {(job) => (
              <tr>
                <td class="mono small">{labelFor(job.kind)}</td>
                <td><StatusBadge status={job.status} /></td>
                <td class="dock-progress">
                  <Meter label="" value={job.completed + job.failed} max={job.total} text={summaryLine(job)} />
                </td>
                <td class="small muted dock-line">{line(job)}</td>
                <td><button class="secondary tiny" onClick={() => navigate(`/operations?id=${job.id}`)}>Open</button></td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </Show>
  );
}
