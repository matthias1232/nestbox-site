// The live traffic page: one table, three always-visible filters, a red lane of refused
// destinations with inline answers, pause, a pop-out, and the evaluation windows (master
// switches and decisions) in a side panel.
//
// Answering never creates a rule from a click: "Allow now" is the existing one-step allow,
// "15 min" / "1 h" open a bounded window, "Draft rule" saves a draft that is reviewed later.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import PageHeader from '../shell/PageHeader.jsx';
import AlertPopup from '../../components/AlertPopup.jsx';
import TrafficTable from '../../components/TrafficTable.jsx';
import Segmented from '../../components/Segmented.jsx';
import { allowBody, draftEvent, windowBody } from '../../components/alertAnswers.js';
import { navigate, paths } from '../../routing.js';
import { useSession } from '../../state/session.jsx';
import { useLive } from '../../state/live.jsx';
import { useInventory } from '../shell/useInventory.js';
import * as policies from '../../api/policies.js';
import * as telemetry from '../../api/telemetry.js';
import { WINDOWS, useTrafficStream } from './useTrafficStream.js';
import { useWindows } from './useWindows.js';
import { useDecisions } from './useDecisions.js';
import MasterSwitches from './MasterSwitches.jsx';
import DecisionsPanel from './DecisionsPanel.jsx';
import {
  DECISION_CHIPS, KIND_CHIPS, applyFilters, counts, isRefused, nodeBySandbox, sandboxOptions, targetOptions,
} from './trafficFilters.js';
import '../../styles/traffic.css';
import { environmentUrl } from '../../api/client.js';

const RED_LANE_MAX = 6;

// Refused rows grouped by destination and sandbox, newest first, as alert-shaped objects.
function redLane(rows, hidden) {
  const groups = new Map();
  for (const row of rows) {
    if (!isRefused(row) || !row.sandboxId) continue;
    const id = `${row.sandboxId}|${row.host}|${row.port ?? ''}`;
    if (hidden[id]) continue;
    const found = groups.get(id);
    if (found) {
      found.count += 1;
    } else {
      groups.set(id, {
        id,
        sandbox_id: row.sandboxId,
        sandbox_name: row.sandbox,
        host: row.host,
        port: row.port,
        protocol: row.protocol,
        channel: row.channel,
        decision: row.decision,
        reason: row.reason,
        count: 1,
        first_at: row.ts,
        last_at: row.ts,
      });
    }
  }
  return [...groups.values()].slice(0, RED_LANE_MAX);
}

// The traffic view shared by the page and the pop-out. `popout` hides the pop-out button and
// the inventory tree (a pop-out filters by sandbox name only).
export function TrafficView(props) {
  const traffic = useTrafficStream();
  const windows = useWindows();
  const decisions = useDecisions();
  const session = useSession();
  const live = useLive();
  const inventory = props.popout ? null : useInventory();

  const [kind, setKind] = createSignal('all');
  const [decision, setDecision] = createSignal('all');
  const [target, setTarget] = createSignal('');
  const [panel, setPanel] = createSignal(!props.popout);
  const [panelTab, setPanelTab] = createSignal('switches');
  const [hidden, setHidden] = createSignal({});
  const [busy, setBusy] = createSignal({});
  const [done, setDone] = createSignal({});
  const [notice, setNotice] = createSignal(null);
  const [draft, setDraft] = createSignal(null);

  onMount(() => {
    traffic.load();
    traffic.start();
    windows.load();
    decisions.load();
  });

  const tree = () => (inventory ? inventory.root() : null);
  const nodeMap = createMemo(() => nodeBySandbox(tree()));
  const options = createMemo(() => {
    const fromTree = targetOptions(tree());
    return fromTree.length ? fromTree : sandboxOptions(traffic.names());
  });
  const filters = () => ({ kind: kind(), decision: decision(), target: target() });
  const rows = createMemo(() => applyFilters(traffic.rows(), filters(), nodeMap()));
  const chipCounts = createMemo(() => counts(traffic.rows(), filters(), nodeMap()));
  const lane = createMemo(() => redLane(rows(), hidden()));
  const isAdmin = () => session.identity()?.role === 'admin';
  const connection = () => {
    const state = live ? live.status() : traffic.connected() ? 'connected' : 'off';
    return traffic.connected() ? state : 'off';
  };

  // Answer one red-lane entry: 'now' | '15' | '60' | 'draft' | 'ignore'.
  async function answer(alert, choice) {
    if (choice === 'ignore') {
      setHidden((map) => ({ ...map, [alert.id]: true }));
      return;
    }
    setBusy((map) => ({ ...map, [alert.id]: true }));
    setNotice(null);
    try {
      if (choice === 'now') {
        await policies.allowDestination(alert.sandbox_id, allowBody(alert));
        setDone((map) => ({ ...map, [alert.id]: `Allowed ${alert.host}` }));
      } else if (choice === '15' || choice === '60') {
        const body = windowBody(alert, choice);
        await policies.openWindow(body.sandbox_id, body.minutes, body.reason, body.mode, body.channel, false);
        setDone((map) => ({ ...map, [alert.id]: `Open for ${body.minutes} min` }));
      } else if (choice === 'draft') {
        const result = await policies.draftFromEvent(draftEvent(alert), alert.sandbox_id);
        setDone((map) => ({ ...map, [alert.id]: 'Draft rule saved' }));
        setDraft(result);
      }
    } catch (failure) {
      setNotice(failure.message || 'The answer failed.');
    } finally {
      setBusy((map) => {
        const next = { ...map };
        delete next[alert.id];
        return next;
      });
    }
  }

  // A table row is answered like a red-lane entry built from it.
  function answerRow(row, choice) {
    return answer(redLane([row], {})[0] || {
      id: `${row.sandboxId}|${row.host}|${row.port ?? ''}`,
      sandbox_id: row.sandboxId, host: row.host, port: row.port, protocol: row.protocol,
      channel: row.channel, decision: row.decision, reason: row.reason, count: 1,
      first_at: row.ts, last_at: row.ts,
    }, choice);
  }

  async function seed() {
    setNotice(null);
    try {
      await telemetry.seedDemo();
      await traffic.load();
    } catch (failure) {
      setNotice(failure.message);
    }
  }

  function popOut() {
    window.open(environmentUrl(paths.popTraffic()), 'nestbox-traffic', 'popup,width=1200,height=800');
  }

  const status = () => (
    <span class={`badge ${connection() === 'connected' ? 'ok' : connection() === 'reconnecting' ? 'warn' : 'muted'}`}>
      <i />
      {connection() === 'connected' ? 'streaming' : connection() === 'reconnecting' ? 'reconnecting' : 'not streaming'}
    </span>
  );

  const actions = () => (
    <>
      <select
        aria-label="Time window"
        value={traffic.windowSpan()}
        onChange={(event) => { traffic.setWindowSpan(event.currentTarget.value); traffic.load(); }}
      >
        <For each={WINDOWS}>{(span) => <option value={span}>last {span}</option>}</For>
      </select>
      <button class={traffic.paused() ? 'primary' : 'secondary'} onClick={traffic.togglePause}>
        {traffic.paused() ? `Resume (${traffic.pending().length})` : 'Pause'}
      </button>
      <button class="secondary" onClick={traffic.clear}>Clear</button>
      <Show when={!props.popout}>
        <button class="secondary" onClick={popOut}>Pop out</button>
      </Show>
      <Show when={isAdmin()}>
        <button class="ghost" onClick={seed}>Seed demo</button>
      </Show>
      <button class="ghost" aria-pressed={panel()} onClick={() => setPanel(!panel())}>
        {panel() ? 'Hide windows' : 'Show windows'}
      </button>
    </>
  );

  return (
    <section class="page traffic-page">
      <PageHeader
        title="Live traffic"
        status={status()}
        subtitle={`${rows().length} rows in view${traffic.paused() ? ` · ${traffic.pending().length} held while paused` : ''}`}
        actions={actions()}
      />

      <Show when={traffic.error() || notice()}>
        <p class="error" role="alert">{traffic.error() || notice()}</p>
      </Show>

      <div class="traffic-filters" role="group" aria-label="Traffic filters">
        <label class="traffic-filter">
          <span class="eyebrow">Kind</span>
          <Segmented
            value={kind()}
            onChange={setKind}
            options={KIND_CHIPS.map((chip) => ({ value: chip.id, label: `${chip.label} ${chipCounts().kind[chip.id]}` }))}
          />
        </label>
        <label class="traffic-filter">
          <span class="eyebrow">Decision</span>
          <Segmented
            value={decision()}
            onChange={setDecision}
            options={DECISION_CHIPS.map((chip) => ({
              value: chip.id,
              label: `${chip.label} ${chipCounts().decision[chip.id]}`,
              tone: chip.id === 'refused' ? 'bad' : undefined,
            }))}
          />
        </label>
        <label class="traffic-filter">
          <span class="eyebrow">Sandbox or node</span>
          <select aria-label="Sandbox or node" value={target()} onChange={(event) => setTarget(event.currentTarget.value)}>
            <option value="">All sandboxes and nodes</option>
            <For each={options()}>{(item) => <option value={item.value}>{item.label}</option>}</For>
          </select>
        </label>
      </div>

      <div class={panel() ? 'traffic-layout with-panel' : 'traffic-layout'}>
        <div class="traffic-main">
          <Show when={lane().length}>
            <section class="red-lane" aria-label="Refused destinations waiting for an answer">
              <h2 class="eyebrow">Refused · {lane().length} waiting</h2>
              <table class="dock-table">
                <thead>
                  <tr><th>Destination</th><th>Sandbox</th><th>Channel</th><th>Seen</th><th>Answer</th></tr>
                </thead>
                <tbody>
                  <For each={lane()}>
                    {(alert) => (
                      <AlertPopup
                        inline
                        alert={alert}
                        busy={!!busy()[alert.id]}
                        result={done()[alert.id]}
                        onAllowNow={() => answer(alert, 'now')}
                        onAllowFor={(minutes) => answer(alert, String(minutes))}
                        onDraft={() => answer(alert, 'draft')}
                        onIgnore={() => answer(alert, 'ignore')}
                        onOpenSandbox={() => navigate(paths.sandbox(alert.sandbox_id))}
                      />
                    )}
                  </For>
                </tbody>
              </table>
            </section>
          </Show>

          <Show
            when={rows().length > 0}
            fallback={
              <div class="empty">
                <strong>{traffic.loading() ? 'Reading the window…' : 'No request matches these filters'}</strong>
                <p class="muted">
                  The table shows the last {traffic.windowSpan()} and then appends what arrives. An idle sandbox
                  is quiet, not broken: run something that reaches the network and the row appears within seconds.
                </p>
                <Show when={kind() !== 'all' || decision() !== 'all' || target()}>
                  <button class="secondary" onClick={() => { setKind('all'); setDecision('all'); setTarget(''); }}>
                    Clear filters
                  </button>
                </Show>
              </div>
            }
          >
            <TrafficTable events={rows()} onAnswer={answerRow} />
          </Show>
        </div>

        <Show when={panel()}>
          <aside class="traffic-panel" aria-label="Evaluation windows">
            <div class="facets" role="tablist">
              <button class={panelTab() === 'switches' ? 'chip active' : 'chip'} aria-pressed={panelTab() === 'switches'} onClick={() => setPanelTab('switches')}>
                Windows
              </button>
              <button
                class={panelTab() === 'decisions' ? 'chip active' : 'chip'}
                aria-pressed={panelTab() === 'decisions'}
                onClick={() => { setPanelTab('decisions'); decisions.load(); }}
              >
                Decisions
              </button>
            </div>
            <Show when={panelTab() === 'switches'}><MasterSwitches switches={windows} /></Show>
            <Show when={panelTab() === 'decisions'}><DecisionsPanel decisions={decisions} /></Show>
          </aside>
        </Show>
      </div>

      <Show when={draft()}>
        <div class="draft-card">
          <h3>Rule draft</h3>
          <p class="muted small">
            Nothing is enforced yet. A draft must be reviewed and approved, and only then does it become a rule.
          </p>
          <pre>{JSON.stringify(draft().draft, null, 2)}</pre>
          <div class="actions">
            <button
              onClick={async () => {
                try {
                  await policies.decideDraft(draft().id, true, null);
                  setDraft(null);
                } catch (failure) {
                  setNotice(failure.message);
                }
              }}
            >
              Approve and enforce
            </button>
            <button
              class="secondary"
              onClick={async () => {
                try {
                  await policies.decideDraft(draft().id, false, 'rejected from console');
                  setDraft(null);
                } catch (failure) {
                  setNotice(failure.message);
                }
              }}
            >
              Reject
            </button>
            <button class="ghost" onClick={() => setDraft(null)}>Dismiss</button>
          </div>
        </div>
      </Show>
    </section>
  );
}

// The Live traffic page inside the shell.
export default function TrafficPage() {
  return <TrafficView />;
}
