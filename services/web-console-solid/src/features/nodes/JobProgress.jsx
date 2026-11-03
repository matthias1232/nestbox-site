// A job's step list with ticks, its last log lines and its end state.
// Props: jobId, steps (optional, from progressStates(); default: plain job bar),
// withModel (bool), joinCommand (optional copy box), onFinished(job).
import { For, Show, createSignal, onCleanup } from 'solid-js';
import * as jobsApi from '../../api/jobs.js';
import { usePolling } from '../../components/Poll.js';
import { useLive } from '../../state/live.jsx';
import { jobFinished, progressStates } from './nodeWizard.js';

const MARK = { done: '✓', active: '…', failed: '✕', waiting: '·' };
const WORD = { done: 'done', active: 'running', failed: 'failed', waiting: 'waiting' };

// Poll interval for the job and its log; the live `job` event refreshes sooner.
const POLL_MS = 3000;

export default function JobProgress(props) {
  const [job, setJob] = createSignal(null);
  const [logs, setLogs] = createSignal([]);
  const [error, setError] = createSignal(null);
  const [copied, setCopied] = createSignal(false);
  let reported = false;

  async function load() {
    const [next, rows] = await Promise.all([jobsApi.get(props.jobId), jobsApi.logs(props.jobId, 50)]);
    return { next, rows: rows.items || [] };
  }
  const poll = usePolling(load, {
    everyMs: POLL_MS,
    onData: ({ next, rows }) => {
      setJob(next);
      setLogs(rows);
      setError(null);
      if (jobFinished(next) && !reported) {
        reported = true;
        props.onFinished?.(next);
      }
    },
    onError: (failure) => setError(failure.message),
  });
  const live = useLive();
  if (live) onCleanup(live.on('job', () => poll.refresh()));

  const states = () => progressStates(job(), logs(), Boolean(props.withModel));
  const lastLine = () => {
    const rows = logs();
    return rows.length ? rows[rows.length - 1].message : '';
  };

  async function copy() {
    try {
      await navigator.clipboard.writeText(props.joinCommand);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div class="job-progress">
      <Show when={props.joinCommand}>
        <div class="notice">
          <p>Run this as root on the machine; this page waits for it.</p>
          <input class="mono" readonly style="width:100%" aria-label="Join command" value={props.joinCommand}
                 onFocus={(e) => e.currentTarget.select()} />
          <button type="button" class="secondary tiny" onClick={copy}>{copied() ? 'Copied' : 'Copy command'}</button>
        </div>
      </Show>
      <Show when={props.steps !== false}>
        <ol class="job-steps">
          <For each={states()}>
            {(step) => (
              <li class={`job-step ${step.state}`}>
                <span class="mono" aria-hidden="true">{MARK[step.state]}</span>{' '}
                {step.label} <span class="muted small">{WORD[step.state]}</span>
              </li>
            )}
          </For>
        </ol>
      </Show>
      <Show when={job()}>
        <p class="muted small">
          {job().status}{job().total > 0 ? ` · ${job().completed}/${job().total} done${job().failed ? `, ${job().failed} failed` : ''}` : ''}
          {lastLine() ? ` · ${lastLine()}` : ''}
        </p>
        <Show when={job().error}><p class="error">{job().error}</p></Show>
      </Show>
      <Show when={!job() && !error()}><p class="muted">Loading…</p></Show>
      <Show when={error()}><p class="error">{error()}</p></Show>
    </div>
  );
}
