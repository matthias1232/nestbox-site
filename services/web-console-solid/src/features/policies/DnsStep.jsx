// Wizard step 2: DNS allow and block patterns. Props: dns [{action,pattern}], onDns(list), suggestions [{host,count}].

import { For, Show, createSignal } from 'solid-js';
import { setDns } from './policyWizard.js';
import { DNS_HELP } from './ruleHelp.js';

// DNS patterns are saved as DNS rules of the policy, not inside the document.
export default function DnsStep(props) {
  const [pattern, setPattern] = createSignal('');
  const [action, setAction] = createSignal('deny');
  const add = (act, text) => props.onDns(setDns([...props.dns, { action: act, pattern: text }]));

  return (
    <div class="pw-step">
      <h2>DNS</h2>
      <p class="muted small">{DNS_HELP.action}</p>
      <div class="pw-add">
        <label class="pw-field"><span>Action</span>
          <select class="pf-input" value={action()} onChange={(e) => setAction(e.currentTarget.value)}>
            <option value="deny">block</option>
            <option value="allow">allow</option>
          </select>
        </label>
        <label class="pw-field"><span>Pattern</span>
          <input class="pf-input mono" value={pattern()} placeholder=".tracker.example"
            onInput={(e) => setPattern(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') { add(action(), pattern()); setPattern(''); } }} />
        </label>
        <button type="button" class="secondary" disabled={!pattern().trim()} onClick={() => { add(action(), pattern()); setPattern(''); }}>Add DNS rule</button>
      </div>
      <p class="muted small">{DNS_HELP.pattern}</p>

      <Show when={props.dns.length > 0} fallback={<p class="notice">No DNS rule: every name resolves, and the network entries still decide the connection.</p>}>
        <table class="pf-table">
          <thead><tr><th>Action</th><th>Pattern</th><th /></tr></thead>
          <tbody>
            <For each={props.dns}>
              {(rule, i) => (
                <tr>
                  <td><span class={`badge ${rule.action === 'allow' ? 'ok' : 'bad'}`}><i />{rule.action === 'allow' ? 'allow' : 'block'}</span></td>
                  <td class="mono">{rule.pattern}</td>
                  <td><button type="button" class="tiny" aria-label={`Remove ${rule.pattern}`} onClick={() => props.onDns(props.dns.filter((_, k) => k !== i()))}>Remove</button></td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>

      <h3>Names the sandbox asked for</h3>
      <Show when={(props.suggestions || []).length > 0} fallback={<p class="muted small">No DNS decision to suggest from.</p>}>
        <ul class="pw-suggest">
          <For each={props.suggestions}>
            {(s) => (
              <li>
                <span class="mono">{s.host}</span>
                <span class="muted small">{s.count}× · {s.decision || 'seen'}</span>
                <button type="button" class="tiny secondary" onClick={() => add('deny', s.host)} aria-label={`Block ${s.host}`}>Block</button>
                <button type="button" class="tiny secondary" onClick={() => add('allow', s.host)} aria-label={`Allow ${s.host}`}>Allow</button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
