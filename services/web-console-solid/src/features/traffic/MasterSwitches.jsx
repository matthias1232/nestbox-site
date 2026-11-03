// The master-switch panel: markup only. The state and the calls live in
// `useWindows`, so this file stays a description of what an operator can do.
//
// Every switch is a duration. Nothing here is "off until further notice": that
// is what a policy is for, and a switch that can be left open is the one that
// stays open over a weekend.
//
// The panel lives in a 280–480 px side column, so each channel is a card, not a
// table row: a header line with the name and the state, the description under
// it, and the durations wrapping below. A four-column table in that width
// squeezes the description to one word per line.

import { For, Show } from 'solid-js';
import { ALWAYS_ON, CHANNELS, DURATIONS } from './useWindows.js';

function remaining(expiresAt) {
  const ms = new Date(expiresAt).getTime() - Date.now();
  if (!Number.isFinite(ms) || ms <= 0) return 'expired';
  const minutes = Math.ceil(ms / 60000);
  return minutes >= 60 ? `${(minutes / 60).toFixed(1)} h` : `${minutes} min`;
}

// `word` is the state in one word for the card header; `detail` is the running
// countdown shown with the close control, because a time is what an operator
// checks before deciding nothing else is needed.
function terms(window) {
  if (!window) return { tone: 'muted', word: 'enforcing', detail: null };
  if (window.mode === 'audit') {
    return { tone: 'warn', word: 'audit', detail: `observing · ${remaining(window.expires_at)}` };
  }
  return { tone: 'ok', word: 'open', detail: `open · ${remaining(window.expires_at)}` };
}

export default function MasterSwitches(props) {
  const switches = props.switches;
  return (
    <section class="master-switches">
      <header class="controls">
        <div>
          <h2>Master switches</h2>
          <p class="muted small">
            A switch opens one firewall for a fixed time, then reverts on its own. Observation
            mode lets everything through <em>and</em> records what would have been blocked.
          </p>
        </div>
        <button class="secondary tiny" onClick={switches.load} disabled={switches.loading()}>
          {switches.loading() ? 'Reading…' : 'Refresh'}
        </button>
      </header>

      <Show when={switches.observing()}>
        <p class="warn-note">
          <b>Observation mode is active.</b> Requests are allowed and recorded as blocks, so the
          traffic list shows what a strict policy would have stopped. Nothing is being enforced
          for the channel below while this lasts.
        </p>
      </Show>
      <Show when={switches.error()}>
        <p class="error">{switches.error()}</p>
      </Show>
      <Show when={switches.notice()}>
        <p class="notice">{switches.notice()}</p>
      </Show>

      <div class="ms-cards">
        <For each={CHANNELS}>
          {(channel) => {
            const current = () => switches.stateFor(channel.id);
            const state = () => terms(current());
            return (
              <article class="ms-card">
                <div class="ms-card-head">
                  <b class="ms-card-name">{channel.label}</b>
                  <span class="ms-state" data-tone={state().tone}>
                    <i />
                    {state().word}
                  </span>
                </div>
                <p class="ms-card-desc">{channel.description}</p>
                <Show when={current()}>
                  <p class="ms-card-reason">{current().reason}</p>
                </Show>
                <div class="ms-card-actions">
                  <For each={DURATIONS}>
                    {(minutes) => (
                      <button
                        class="tiny"
                        disabled={switches.busy()}
                        onClick={() => switches.flip(channel.id, minutes, 'bypass')}
                      >
                        {minutes} min
                      </button>
                    )}
                  </For>
                  <button
                    class="tiny secondary"
                    disabled={switches.busy()}
                    title="Allow and record what would have been blocked"
                    onClick={() => switches.flip(channel.id, 30, 'audit')}
                  >
                    Observe 30 min
                  </button>
                  <Show when={current()}>
                    <span class="ms-countdown">{state().detail}</span>
                    <button
                      class="tiny danger"
                      disabled={switches.busy()}
                      onClick={() => switches.revoke(current().id)}
                    >
                      Revoke
                    </button>
                  </Show>
                </div>
              </article>
            );
          }}
        </For>

        <article class="ms-card ms-card-fixed">
          <div class="ms-card-head">
            <b class="ms-card-name">{ALWAYS_ON.label}</b>
            <span class="ms-state" data-tone="ok">
              <i />
              always on
            </span>
          </div>
          <p class="ms-card-desc">{ALWAYS_ON.description}</p>
          <p class="ms-card-note">
            Not switchable by design: it is the path the sandbox’s tools and model calls take.
          </p>
        </article>
      </div>
    </section>
  );
}
