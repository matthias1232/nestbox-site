// Wizard step: egress mode and an evaluation window. Props: sandboxId, egressMode, minutes, onMode(mode|null), onMinutes(n).

import { For, Show } from 'solid-js';
import Switch from '../../components/Switch.jsx';
import { EGRESS_MODES } from './policyWizard.js';

// The step only proposes; the Review step lists the calls and Apply makes them.
export default function ObservationStep(props) {
  return (
    <div class="pw-step">
      <h2>Observation</h2>
      <Show when={props.sandboxId} fallback={<p class="notice">Egress mode and evaluation windows belong to a sandbox. Open the wizard from a sandbox to propose them; the policy itself is saved either way.</p>}>
        <p class="muted small">How the sandbox's traffic is handled while this policy is in force. Leave it unchanged to keep the current mode.</p>
        <div class="pw-cards" role="radiogroup" aria-label="Egress mode">
          <button type="button" role="radio" aria-checked={props.egressMode === null} classList={{ 'pw-card': true, active: props.egressMode === null }} onClick={() => props.onMode(null)}>
            <strong>Keep current</strong><span class="muted small">Do not change the egress mode.</span>
          </button>
          <For each={EGRESS_MODES}>
            {(mode) => (
              <button type="button" role="radio" aria-checked={props.egressMode === mode.value} classList={{ 'pw-card': true, active: props.egressMode === mode.value }} onClick={() => props.onMode(mode.value)}>
                <strong>{mode.label}</strong><span class="muted small">{mode.text}</span>
              </button>
            )}
          </For>
        </div>
        <div class="pw-window">
          <Switch label="Open an evaluation window after apply" checked={props.minutes > 0} onChange={(on) => props.onMinutes(on ? 15 : 0)} />
          <Show when={props.minutes > 0}>
            <label class="pw-field"><span>Minutes</span>
              <input class="pf-input mono" type="number" min="1" max="1440" value={props.minutes} onChange={(e) => props.onMinutes(Math.max(1, Number(e.currentTarget.value) || 1))} />
            </label>
            <p class="muted small">During the window traffic is allowed quietly while the proxy keeps recording what the rules would have blocked.</p>
          </Show>
        </div>
      </Show>
    </div>
  );
}
