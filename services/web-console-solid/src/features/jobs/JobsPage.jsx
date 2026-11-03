// Tasks: every job with a readable label, progress and a drawer for results and logs; schedules in a second drawer.
import { Show, createMemo, createSignal, onCleanup } from 'solid-js';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import Drawer from '../../components/Drawer.jsx';
import Meter from '../../components/Meter.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { usePolling } from '../../components/Poll.js';
import { useLive } from '../../state/live.jsx';
import { useUi } from '../../state/ui.jsx';
import * as api from '../../api/jobs.js';
import JobDrawer from './JobDrawer.jsx';
import { durationText, labelFor, summaryLine } from './jobLabels.js';
import '../../styles/jobs.css';

const FILTERS = [
  { id: 'running', label: 'Running', test: (j) => j.status === 'running' || j.status === 'pending' },
  { id: 'failed', label: 'Failed', test: (j) => j.status === 'failed' || j.status === 'partial' || j.failed > 0 },
  { id: 'all', label: 'All', test: () => true },
];

// The `?id=<uuid>` of the address opens that task's drawer (the dock's "Open" button links here).
function idFromAddress() {
  return new URLSearchParams(window.location.search).get('id');
}

export default function JobsPage() {
  const [jobs, setJobs] = createSignal([]);
  const [schedules, setSchedules] = createSignal([]);
  const [filter, setFilter] = createSignal('all');
  const [openId, setOpenId] = createSignal(idFromAddress());
  const [showSchedules, setShowSchedules] = createSignal(false);
  const [error, setError] = createSignal('');
  const live = useLive();
  const ui = useUi();

  const poll = usePolling(async () => {
    const [jobList, scheduleList] = await Promise.all([api.list(100), api.schedules()]);
    return { jobs: jobList.items || [], schedules: scheduleList.items || [] };
  }, {
    everyMs: 5000,
    onData: (data) => { setJobs(data.jobs); setSchedules(data.schedules); setError(''); },
    onError: (failure) => setError(failure.message),
  });
  onCleanup(live.on('job', () => poll.refresh()));

  const shown = createMemo(() => jobs().filter(FILTERS.find((f) => f.id === filter()).test));
  const count = (id) => jobs().filter(FILTERS.find((f) => f.id === id).test).length;
  const openJob = () => jobs().find((j) => j.id === openId()) || (openId() ? { id: openId(), kind: '', status: '' } : null);

  async function run(schedule) {
    try {
      await api.runSchedule(schedule.id);
      ui.toast.show({ tone: 'ok', text: `Started ${schedule.name}.`, undo: null, ms: 4000 });
      poll.refresh();
    } catch (failure) {
      ui.toast.show({ tone: 'bad', text: failure.message, undo: null, ms: 6000 });
    }
  }

  const columns = [
    { key: 'kind', label: 'Task', sortable: true, render: (j) => <strong>{labelFor(j.kind)}</strong> },
    { key: 'status', label: 'Status', sortable: true, render: (j) => <StatusBadge status={j.status} /> },
    { key: 'completed', label: 'Progress', render: (j) => <Meter label="" value={(j.completed || 0) + (j.failed || 0)} max={j.total || 0} text={summaryLine(j)} /> },
    { key: 'started_at', label: 'Started', sortable: true, render: (j) => <span class="mono small">{j.started_at || j.created_at || '—'}</span> },
    { key: 'duration', label: 'Duration', render: (j) => <span class="mono small">{durationText(j) || '—'}</span> },
    { key: 'open', label: '', render: (j) => <button class="secondary tiny" onClick={(e) => { e.stopPropagation(); setOpenId(j.id); }}>Open</button> },
  ];
  const scheduleColumns = [
    { key: 'name', label: 'Name', render: (s) => <span class="mono small">{s.name}</span> },
    { key: 'cron', label: 'Cron', render: (s) => <span class="mono small">{s.cron}</span> },
    { key: 'enabled', label: 'Enabled', render: (s) => (s.enabled ? 'yes' : 'no') },
    { key: 'last_run_at', label: 'Last run', render: (s) => <span class="mono small">{s.last_run_at || 'never'}</span> },
    { key: 'last_status', label: 'Result', render: (s) => (s.last_status ? <StatusBadge status={s.last_status} /> : <span class="muted small">—</span>) },
    { key: 'run', label: '', render: (s) => <button class="secondary tiny" onClick={() => run(s)}>Run now</button> },
  ];

  return (
    <section class="page jobs-page">
      <Breadcrumb items={[{ label: 'Operate' }, { label: 'Tasks' }]} />
      <PageHeader
        title="Tasks"
        subtitle={`${jobs().length} tasks · ${schedules().length} schedules`}
        actions={
          <>
            <div class="jobs-filters" role="group" aria-label="Filter tasks">
              {FILTERS.map((f) => (
                <button type="button" class="chip" classList={{ active: filter() === f.id }} aria-pressed={filter() === f.id} onClick={() => setFilter(f.id)}>
                  {f.label}<b>{count(f.id)}</b>
                </button>
              ))}
            </div>
            <button class="secondary" onClick={() => setShowSchedules(true)}>Schedules…</button>
            <button class="secondary" onClick={() => poll.refresh()}>Refresh</button>
          </>
        }
      />
      <Show when={error()}><p class="error">{error()}</p></Show>
      <SelectTable
        columns={columns}
        rows={shown()}
        rowKey="id"
        onRowClick={(j) => setOpenId(j.id)}
        empty={{ title: 'No tasks here', hint: 'Bulk actions, drains and deployments appear as tasks with their progress.' }}
      />
      <JobDrawer job={openJob()} onClose={() => setOpenId(null)} onRetried={(id) => { poll.refresh(); if (id) setOpenId(id); }} />
      <Drawer open={showSchedules()} title="Schedules" subtitle="Recurring tasks" onClose={() => setShowSchedules(false)}>
        <SelectTable columns={scheduleColumns} rows={schedules()} rowKey="id" empty={{ title: 'No schedules' }} />
      </Drawer>
    </section>
  );
}
