// The agent situation page: organisation totals, the live map, and one agent
// in detail. The page is markup and layout; the stream lives in the hook.

import { Show, createSignal, onMount } from 'solid-js';
import Empty from '../../components/Empty.jsx';
import AgentSituationMap from './AgentSituationMap.jsx';
import AgentPanel from './AgentPanel.jsx';
import AllChats from './AllChats.jsx';
import RequestStream from './RequestStream.jsx';
import Segmented from '../../components/Segmented.jsx';
import { useLiveAgents } from './useLiveAgents.js';
import { formatCost, formatCount } from './format.js';
import './live.css';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const CONNECTION = {
  connecting: { tone: 'warn', label: 'connecting' },
  live: { tone: 'ok', label: 'live' },
  lost: { tone: 'bad', label: 'disconnected' },
};

const VIEWS = [
  { value: 'map', label: 'Agenten-Karte' },
  { value: 'requests', label: 'Requests live' },
  { value: 'chats', label: 'Alle Chats' },
];

export default function LivePage() {
  const [view, setView] = createSignal('map');
  const live = useLiveAgents();
  onMount(() => live.start());

  const connection = () => CONNECTION[live.connection()] || CONNECTION.connecting;

  return (
    <section class="page live-page">
      <PageHeader
        title="Agent situation"
        subtitle={<>What every agent is doing now, and what it did in the last five minutes.</>}
        actions={<>
          <span class={`badge ${connection().tone}`}>{connection().label}</span>
          <HelpTip anchor="getting-started" />
        </>}
      />

      <Show when={live.connection() === 'lost'}>
        <div class="warn-note live-banner" role="status">
          <span>
            Connection lost. The map keeps the last known state, but no new event
            is arriving and the counters are frozen.
          </span>
          <button class="tiny" onClick={live.retry}>Retry now</button>
        </div>
      </Show>

      <Show when={live.snapshotError() && live.agents().length === 0 && !live.loading()}>
        <Empty
          title="Agent situation needs the live agents route"
          hint="The control API did not answer the live agents snapshot, so this page has nothing to draw. It fills in as soon as the route exists; see the project status for the open question."
        />
      </Show>

      <Show when={live.snapshotError()}>
        <p class="notice small">
          Snapshot unavailable ({live.snapshotError()}). Only events received since
          this page opened are shown.
        </p>
      </Show>

      <div class="cards live-strip">
        <article>
          <span>Agents active</span>
          <b>{live.loading() ? '—' : formatCount(live.totals().active)}</b>
          <small>{live.loading() ? 'reading snapshot' : `${formatCount(live.totals().agents)} known`}</small>
        </article>
        <article>
          <span>Requests · 5 min</span>
          <b>{live.loading() ? '—' : formatCount(live.totals().requests)}</b>
          <small>snapshot + events since it arrived</small>
        </article>
        <article>
          <span>Tokens · 5 min</span>
          <b>{live.loading() ? '—' : (live.totals().tokens === null ? 'n/a' : formatCount(live.totals().tokens))}</b>
          <small>{live.totals().tokens === null ? 'not reported by the stream' : 'reported by the stream'}</small>
        </article>
        <article>
          <span>Cost · 5 min</span>
          <b>{live.loading() ? '—' : formatCost(live.totals().cost)}</b>
          <small>{live.totals().cost === null ? 'not reported' : 'snapshot + events since it arrived'}</small>
        </article>
      </div>

      <div class="controls">
        <Segmented options={VIEWS} value={view()} onChange={setView} />
      </div>

      <Show when={view() === 'requests'}><RequestStream /></Show>
      <Show when={view() === 'chats'}><AllChats /></Show>

      <Show when={view() === 'map'}>
      <div class="live-body">
        <section class="live-map-panel" aria-label="Agent map">
          <Show
            when={live.agents().length > 0}
            fallback={(
              <Show
                when={!live.loading()}
                fallback={<p class="muted">Connecting to the live agent stream…</p>}
              >
                <Empty
                  title="No agent activity yet"
                  hint="This is not an error. The snapshot is empty and no event has arrived; an idle environment looks exactly like this."
                />
              </Show>
            )}
          >
            <AgentSituationMap
              agents={live.agents()}
              pulses={live.pulses()}
              selectedKey={live.selectedKey()}
              onSelect={live.select}
            />
          </Show>
        </section>

        <AgentPanel agent={live.selected()} />
      </div>

      <p class="muted small live-note">
        Counters combine the last snapshot with the events received since. Tokens
        and per-request cost are counted only when the stream reports them; an
        unreported value is <code>n/a</code>, never zero. A moving dot marks one
        real event, and nothing moves while the environment is idle.
      </p>
      </Show>
    </section>
  );
}
