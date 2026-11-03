// Wizard step 1: the allow list. Props: spec, onSpec(next), suggestions [{host,port,protocol,count}].

import { For, Show, createSignal } from 'solid-js';
import { addAllow, entries, removeEntry, updateEntry } from './policyWizard.js';
import { RULE_HELP } from './ruleHelp.js';
import { splitList } from './policyForms.js';

const PROTOCOLS = ['rest', 'http', 'websocket', 'graphql', 'mcp', 'jsonrpc', 'tcp'];

// The allow list as a table, a quick-add row and the suggestions from what the sandbox reached.
export default function NetworkStep(props) {
  const [host, setHost] = createSignal('');
  const [port, setPort] = createSignal('443');
  const [protocol, setProtocol] = createSignal('rest');
  const rows = () => entries(props.spec);

  function add() {
    props.onSpec(addAllow(props.spec, { host: host(), port: port(), protocol: protocol() }));
    setHost('');
  }

  return (
    <div class="pw-step">
      <h2>Network</h2>
      <p class="muted small">
        Default deny: a sandbox can reach only what is listed here. Nothing listed means nothing reachable.
        This section applies to running sandboxes within about a minute.
      </p>

      <div class="pw-add">
        <label class="pw-field"><span>Host</span>
          <input class="pf-input" value={host()} placeholder="api.example.com" onInput={(e) => setHost(e.currentTarget.value)}
            onKeyDown={(e) => e.key === 'Enter' && add()} />
        </label>
        <label class="pw-field"><span>Port</span>
          <input class="pf-input" type="number" min="1" max="65535" value={port()} onInput={(e) => setPort(e.currentTarget.value)} />
        </label>
        <label class="pw-field"><span>Protocol</span>
          <select class="pf-input" value={protocol()} onChange={(e) => setProtocol(e.currentTarget.value)}>
            <For each={PROTOCOLS}>{(p) => <option value={p}>{p}</option>}</For>
          </select>
        </label>
        <button type="button" class="secondary" disabled={!host().trim()} onClick={add}>Add allow entry</button>
      </div>
      <p class="muted small">{RULE_HELP.protocol}</p>

      <Show when={rows().length > 0} fallback={<p class="notice">No destination is allowed yet. Add one above or take a suggestion.</p>}>
        <div class="table-scroll">
          <table class="pf-table">
            <thead>
              <tr><th>Host</th><th>Port</th><th>Protocol</th><th>Access</th><th>Methods</th><th>Binaries</th><th /></tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row, i) => (
                  <tr>
                    <td><input class="pf-input mono" aria-label="Host" value={row.endpoint.host}
                      onChange={(e) => props.onSpec(updateEntry(props.spec, i(), { host: e.currentTarget.value.trim() }))} /></td>
                    <td><input class="pf-input mono" type="number" aria-label="Port" value={row.endpoint.port ?? ''}
                      onChange={(e) => props.onSpec(updateEntry(props.spec, i(), { port: e.currentTarget.value === '' ? undefined : Number(e.currentTarget.value) }))} /></td>
                    <td>
                      <select class="pf-input" aria-label="Protocol" value={row.endpoint.protocol || 'rest'}
                        onChange={(e) => props.onSpec(updateEntry(props.spec, i(), { protocol: e.currentTarget.value }))}>
                        <For each={PROTOCOLS}>{(p) => <option value={p}>{p}</option>}</For>
                      </select>
                    </td>
                    <td>
                      <select class="pf-input" aria-label="Access" value={row.endpoint.access || 'allow'}
                        onChange={(e) => props.onSpec(updateEntry(props.spec, i(), { access: e.currentTarget.value }))}>
                        <option value="allow">allow</option>
                        <option value="deny">deny</option>
                      </select>
                    </td>
                    <td><input class="pf-input mono" aria-label="Methods" placeholder="any" value={(row.endpoint.methods || []).join(', ')}
                      onChange={(e) => {
                        const list = splitList(e.currentTarget.value).map((m) => m.toUpperCase());
                        props.onSpec(updateEntry(props.spec, i(), { methods: list.length ? list : undefined }));
                      }} /></td>
                    <td><input class="pf-input mono" aria-label="Binaries" placeholder="any" value={(row.endpoint.binaries || []).join(', ')}
                      onChange={(e) => {
                        const list = splitList(e.currentTarget.value);
                        props.onSpec(updateEntry(props.spec, i(), { binaries: list.length ? list : undefined }));
                      }} /></td>
                    <td><button type="button" class="tiny" aria-label={`Remove ${row.endpoint.host}`} onClick={() => props.onSpec(removeEntry(props.spec, i()))}>Remove</button></td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
        <p class="muted small">Methods: {RULE_HELP.method} Binaries: {RULE_HELP.binary}</p>
      </Show>

      <h3>Suggestions from the last 24 h</h3>
      <Show when={(props.suggestions || []).length > 0}
        fallback={<p class="muted small">{props.hasSandbox ? 'The sandbox has not been refused anything in the last 24 h.' : 'Choose a sandbox (open the wizard from its policy tab) to see what it reached.'}</p>}>
        <ul class="pw-suggest">
          <For each={props.suggestions}>
            {(s) => (
              <li>
                <span class="mono">{s.host}:{s.port}</span>
                <span class="muted small">{s.protocol} · {s.count}× · {s.decision || 'seen'}</span>
                <button type="button" class="tiny secondary" aria-label={`Add ${s.host} as allow`}
                  onClick={() => props.onSpec(addAllow(props.spec, { host: s.host, port: s.port, protocol: s.protocol }))}>
                  Add as allow
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
