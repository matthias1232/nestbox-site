// The decisions view: markup only. State and calls live in `useDecisions`.
//
// This is the "what must I decide" screen, not the "what happened" screen. It
// lists only denials, grouped by destination, and offers the three answers a
// denial has: open it for a while, draft a rule, or leave it and hide the row.

import { For, Show } from 'solid-js';
import { DURATIONS } from './useWindows.js';
import { channelFor } from './useDecisions.js';

function age(ts) {
  const ms = Date.now() - new Date(ts).getTime();
  if (!Number.isFinite(ms) || ms < 0) return '—';
  const minutes = Math.round(ms / 60000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  return `${Math.round(minutes / 6) / 10} h ago`;
}

export default function DecisionsPanel(props) {
  const decisions = props.decisions;
  return (
    <section class="decisions">
      <header class="controls">
        <div>
          <h2>Waiting for a decision</h2>
          <p class="muted small">
            Only destinations that were stopped. Grouped, because a decision is about a
            destination and not about each attempt. Nothing here changes until you choose.
          </p>
        </div>
        <select
          value={decisions.windowSpan()}
          onChange={(event) => {
            decisions.setWindowSpan(event.currentTarget.value);
            decisions.load();
          }}
        >
          <For each={['15m', '1h', '6h', '24h']}>
            {(span) => <option value={span}>last {span}</option>}
          </For>
        </select>
        <button class="secondary tiny" onClick={decisions.load} disabled={decisions.loading()}>
          {decisions.loading() ? 'Reading…' : 'Refresh'}
        </button>
        <Show when={decisions.hiddenCount() > 0}>
          <button
            class="secondary tiny"
            onClick={() => decisions.setShowHidden(!decisions.showHidden())}
          >
            {decisions.showHidden()
              ? `Hide the ${decisions.hiddenCount()} hidden`
              : `Show ${decisions.hiddenCount()} hidden`}
          </button>
        </Show>
      </header>

      <Show when={decisions.error()}>
        <p class="error">{decisions.error()}</p>
      </Show>
      <Show when={decisions.notice()}>
        <p class="notice">{decisions.notice()}</p>
      </Show>

      <Show
        when={decisions.visible().length > 0}
        fallback={
          <div class="empty">
            <strong>Nothing is blocked in this window</strong>
            <p class="muted">
              Every request the proxy saw was allowed, or nothing tried to reach the network.
              Blocked destinations appear here as soon as a sandbox hits one.
            </p>
          </div>
        }
      >
        <div class="table-scroll">
          <table class="dense">
            <thead>
              <tr>
                <th>Sandbox</th>
                <th>Destination</th>
                <th class="narrow">Channel</th>
                <th class="narrow">Blocked</th>
                <th class="narrow">Last</th>
                <th>Why</th>
                <th>Open it</th>
                <th class="narrow" />
              </tr>
            </thead>
            <tbody>
              <For each={decisions.visible()}>
                {(item) => (
                  <tr>
                    <td class="small"><span class="origin">{item.sandbox_name || 'unknown sandbox'}</span></td>
                    <td class="mono small">
                      {item.host}
                      {item.port ? `:${item.port}` : ''}
                    </td>
                    <td class="narrow small">{channelFor(item.protocol)}</td>
                    <td class="narrow mono">{item.denials}×</td>
                    <td class="narrow small muted">{age(item.last_seen)}</td>
                    <td class="small muted">{item.reason || '—'}</td>
                    <td>
                      <div class="actions">
                        <For each={DURATIONS}>
                          {(minutes) => (
                            <button
                              class="tiny"
                              disabled={decisions.busy()}
                              title={`Allow for ${minutes} minutes, then revert`}
                              onClick={() => decisions.allowFor(item, minutes)}
                            >
                              {minutes} min
                            </button>
                          )}
                        </For>
                        <button
                          class="tiny secondary"
                          disabled={decisions.busy()}
                          title="Draft a permanent rule; it is reviewed before it applies"
                          onClick={() => decisions.draft(item)}
                        >
                          Draft rule
                        </button>
                      </div>
                    </td>
                    <td class="narrow">
                      <button class="tiny ghost" title="Hide from this view" onClick={() => decisions.hide(item)}>
                        Hide
                      </button>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </section>
  );
}
