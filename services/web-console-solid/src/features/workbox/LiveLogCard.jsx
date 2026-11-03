// The live log of one sandbox: network, DNS and inference events, newest on top,
// with a red lane for refused destinations that carries the answers (contract C9).
//
// The events are the ones `/api/v1/telemetry/stream` sends: the SSE name is the
// kind (`network`, `dns`, `inference`) and the frame is the event itself with the
// fields flat — `sandbox_id`, `id`, `ts`, `decision`, `destination`/`sni`/`port`
// or `hostname` or `model_key`/`endpoint`/`status_code`, `binary`, `protocol`.
// `controlStrip.js` `logLine` turns one into a line.

import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';

import * as alertsApi from '../../api/alerts.js';
import * as policies from '../../api/policies.js';
import AlertPopup from '../../components/AlertPopup.jsx';
import { allowBody, draftEvent, windowBody } from '../../components/alertAnswers.js';
import Empty from '../../components/Empty.jsx';
import { navigate, paths } from '../../routing.js';
import { useLive } from '../../state/live.jsx';
import { useUi } from '../../state/ui.jsx';
import { LOG_FILTERS, addLine, filterLines, laneAlerts, logLine } from './controlStrip.js';
import { environmentUrl } from '../../api/client.js';

// Props: sandboxId, onChanged (a rule or window was made).
export default function LiveLogCard(props) {
  const live = useLive();
  const ui = useUi();

  const [lines, setLines] = createSignal([]);
  const [alerts, setAlerts] = createSignal([]);
  const [results, setResults] = createSignal({});
  const [busyId, setBusyId] = createSignal(null);
  const [filter, setFilter] = createSignal('all');
  const [frozen, setFrozen] = createSignal(null);

  const shown = createMemo(() => filterLines(frozen() ?? lines(), filter()));
  const lane = createMemo(() => laneAlerts(alerts(), props.sandboxId, ui.alerts.isIgnored));

  function take(kind) {
    return (event) => {
      const line = logLine(kind, event, props.sandboxId);
      if (line) setLines((current) => addLine(current, line));
    };
  }

  onMount(() => {
    const release = live.wantTelemetry();
    const stops = [
      live.on('network', take('network')),
      live.on('dns', take('dns')),
      live.on('inference', take('inference')),
      live.on('alert', (alert) => {
        if (alert?.sandbox_id === props.sandboxId) setAlerts((current) => [...current, alert]);
      }),
    ];
    onCleanup(() => {
      release();
      for (const stop of stops) stop();
    });
    alertsApi
      .list({ window: '15m', sandbox: props.sandboxId })
      .then((answer) => setAlerts((current) => [...(answer?.items || []), ...current]))
      .catch(() => {});
  });

  function togglePause() {
    setFrozen(frozen() ? null : lines());
  }

  function popOut() {
    window.open(environmentUrl(paths.popLogs(props.sandboxId)), 'nestbox-logs', 'width=1100,height=700');
  }

  // Run one answer; the lane row says what happened instead of vanishing silently.
  async function answer(alert, work, done) {
    setBusyId(alert.id);
    try {
      await work();
      setResults({ ...results(), [alert.id]: done });
      props.onChanged?.();
    } catch (failure) {
      ui.toast.show({ tone: 'bad', text: failure.message || 'The answer failed.' });
    } finally {
      setBusyId(null);
    }
  }

  const allowNow = (alert) =>
    answer(alert, () => policies.allowDestination(props.sandboxId, allowBody(alert)), 'rule created');
  const allowFor = (alert, minutes) => {
    const body = windowBody(alert, minutes);
    return answer(
      alert,
      () => policies.openWindow(props.sandboxId, body.minutes, body.reason, 'bypass', body.channel),
      `allowed ${minutes === 60 ? '1 h' : `${minutes} min`}`,
    );
  };
  const draft = (alert) =>
    answer(alert, () => policies.draftFromEvent(draftEvent(alert), props.sandboxId), 'draft created');

  return (
    <section class="wb-card wb-log">
      <div class="draft-head">
        <h2>Live log</h2>
        <div class="wb-log-tools">
          <For each={LOG_FILTERS}>
            {(item) => (
              <button
                type="button"
                class={'chip tiny' + (filter() === item.id ? ' active' : '')}
                aria-pressed={filter() === item.id}
                onClick={() => setFilter(item.id)}
              >
                {item.label}
              </button>
            )}
          </For>
          <button type="button" class="tiny secondary" aria-pressed={Boolean(frozen())} onClick={togglePause}>
            {frozen() ? 'Resume' : 'Pause'}
          </button>
          <button type="button" class="tiny ghost" onClick={popOut}>Pop out</button>
        </div>
      </div>

      <Show when={lane().length > 0}>
        <div class="wb-log-lane" role="region" aria-label="Refused destinations">
          <table class="dense">
            <tbody>
              <For each={lane()}>
                {(alert) => (
                  <AlertPopup
                    inline
                    alert={alert}
                    busy={busyId() === alert.id}
                    result={results()[alert.id]}
                    onAllowNow={() => allowNow(alert)}
                    onAllowFor={(minutes) => allowFor(alert, minutes)}
                    onDraft={() => draft(alert)}
                    onIgnore={() => ui.alerts.ignore(alert.id)}
                    onOpenSandbox={() => navigate(paths.sandbox(props.sandboxId))}
                  />
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

      <div class="wb-log-body" tabindex="0" aria-label="Events of this sandbox, newest first">
        <Show
          when={shown().length > 0}
          fallback={<Empty title="Nothing yet." hint="Network, DNS and model requests of this box appear here as they happen." />}
        >
          <ul class="plain wb-log-list">
            <For each={shown()}>
              {(line) => (
                <li class={'wb-log-line' + (line.refused ? ' refused' : '')}>
                  <span class="mono muted small">{new Date(line.ts).toLocaleTimeString()}</span>
                  <span class={'badge ' + (line.refused ? 'bad' : line.decision === 'audit' ? 'warn' : 'ok')}>
                    <i />{line.refused ? 'refused' : line.decision === 'audit' ? 'seen' : 'allowed'}
                  </span>
                  <span class="badge muted"><i />{line.kind}</span>
                  <span class="mono wb-log-text">{line.text}</span>
                  <Show when={line.protocol || line.binary}>
                    <span class="muted small mono">
                      {[line.protocol, line.binary].filter(Boolean).join(' · ')}
                    </span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </section>
  );
}
