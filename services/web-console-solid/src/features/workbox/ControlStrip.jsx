// The one-click control strip of a Workbox: allow everything, proxy mode, recording
// and three channel switches. Every click is one `quick.set` call and the control
// summary in the answer redraws the strip (contract C12).

import { For, Show, createEffect, createMemo, createSignal } from 'solid-js';

import * as quick from '../../api/quick.js';
import Countdown from '../../components/Countdown.jsx';
import HelpTip from '../../components/HelpTip.jsx';
import Segmented from '../../components/Segmented.jsx';
import Switch from '../../components/Switch.jsx';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import {
  ALLOW_ALL_CHOICES,
  CHANNEL_SWITCHES,
  FOR_CHOICES,
  MODE_CHOICES,
  RECORDING_CHOICES,
  allowAllChoice,
  channelAction,
  labelFor,
  mayControl,
  nextAllowAllAction,
  recordingAction,
  stripState,
} from './controlStrip.js';

// Props: sandboxId, control (the summary), onChanged (reload the page's picture).
export default function ControlStrip(props) {
  const session = useSession();
  const ui = useUi();

  // The answer of the last click; a fresher poll from the page replaces it.
  const [latest, setLatest] = createSignal(null);
  createEffect(() => {
    props.control;
    setLatest(null);
  });
  const [busy, setBusy] = createSignal(false);
  const [lastChoice, setLastChoice] = createSignal(null);
  const [forMinutes, setForMinutes] = createSignal({ internet: 60, dns: 60, mcp: 60 });

  const summary = () => latest() || props.control || {};
  const state = createMemo(() => stripState(summary()));
  const allowed = () =>
    mayControl(session.identity()?.role, props.control?.owner_user_id, session.identity()?.user_id);
  const locked = () => busy() || !allowed();

  // One call, one redraw; a failure is a toast and the strip keeps what the server has.
  async function send(action) {
    setBusy(true);
    try {
      const answer = await quick.set(props.sandboxId, action.what, action.value, action.minutes);
      if (answer && typeof answer === 'object') setLatest(answer);
      props.onChanged?.();
    } catch (failure) {
      ui.toast.show({ tone: 'bad', text: failure.message || 'The change was refused.' });
    } finally {
      setBusy(false);
    }
  }

  function pickAllowAll(choice) {
    setLastChoice(choice);
    send(nextAllowAllAction(state(), choice));
  }

  function pickMode(value) {
    if (value === state().mode) return;
    // The proxy-off mode reverts by itself after 15 minutes; the banner says so.
    send({ what: 'egress_mode', value, minutes: value === 'off_allow_all' ? 15 : null });
  }

  return (
    <section class="wb-strip" aria-label="Quick controls">
      <Show when={!allowed()}>
        <p class="muted small wb-strip-note">This box belongs to someone else; the controls are read-only.</p>
      </Show>

      <div class={'wb-strip-group' + (state().allowAll.on ? ' on' : '')}>
        <span class="wb-strip-label">Allow everything</span>
        <Segmented
          options={ALLOW_ALL_CHOICES.map((item) => ({
            value: item.value,
            label: item.label,
            tone: item.value === 'off' ? undefined : 'bad',
          }))}
          value={allowAllChoice(state(), lastChoice())}
          disabled={locked()}
          onChange={pickAllowAll}
        />
        <Show when={state().allowAll.on}>
          <span class="wb-strip-state">
            <Show when={state().allowAll.until} fallback={<span class="small">on, until you stop it</span>}>
              <Countdown prefix="ends in " until={state().allowAll.until} onExpire={() => props.onChanged?.()} />
            </Show>
          </span>
        </Show>
      </div>

      <div class="wb-strip-group">
        <span class="wb-strip-label">
          Proxy mode <HelpTip anchor="egress-modes" />
        </span>
        <Segmented
          options={MODE_CHOICES}
          value={state().mode}
          disabled={locked()}
          onChange={pickMode}
        />
        <Show when={summary().egress?.revert_at}>
          <span class="wb-strip-state">
            <Countdown prefix="back in " until={summary().egress.revert_at} onExpire={() => props.onChanged?.()} />
          </span>
        </Show>
      </div>

      <div class="wb-strip-group">
        <span class="wb-strip-label">Recording</span>
        <Segmented
          options={RECORDING_CHOICES.map((item) => ({
            value: item.value,
            label: item.label,
            title: item.help,
          }))}
          value={state().recording}
          disabled={locked()}
          onChange={(value) => value !== state().recording && send(recordingAction(value))}
        />
        <span class="wb-strip-state muted small">{labelFor('recording', state().recording).split(' — ')[1]}</span>
      </div>

      <For each={CHANNEL_SWITCHES}>
        {(item) => {
          const channel = () => state().channels[item.key];
          return (
            <div class="wb-strip-group">
              <Switch
                label={`${item.label} ${channel().on ? 'open' : 'ruled'}`}
                checked={channel().on}
                disabled={locked()}
                onChange={(on) => send(channelAction(item.key, on, forMinutes()[item.key]))}
              />
              <label class="wb-strip-for small">
                <span class="muted">for</span>
                <select
                  class="tiny"
                  aria-label={`How long to open ${item.label}`}
                  disabled={locked()}
                  value={forMinutes()[item.key]}
                  onChange={(event) =>
                    setForMinutes({ ...forMinutes(), [item.key]: Number(event.currentTarget.value) })}
                >
                  <For each={FOR_CHOICES}>
                    {(choice) => <option value={choice.minutes}>{choice.label}</option>}
                  </For>
                </select>
              </label>
              <Show when={channel().on}>
                <span class="wb-strip-state">
                  <Show when={channel().until} fallback={<span class="small">until you stop it</span>}>
                    <Countdown prefix="ends in " until={channel().until} onExpire={() => props.onChanged?.()} />
                  </Show>
                </span>
              </Show>
            </div>
          );
        }}
      </For>
    </section>
  );
}
