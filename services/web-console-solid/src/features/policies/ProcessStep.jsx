// Wizard step 4: process fields and Landlock paths. Props: spec, onSpec(next).

import { For, Show } from 'solid-js';
import { joinList } from './policyForms.js';
import { landlockRules, setLandlock, setProcess } from './policyWizard.js';

const ACCESS = ['read', 'write', 'execute'];

// ADR 0008: only the network section changes under a running workload.
export default function ProcessStep(props) {
  const proc = () => props.spec.process || {};
  const rules = () => landlockRules(props.spec);
  const put = (fields) => props.onSpec(setProcess(props.spec, fields));
  const putRule = (i, patch) => props.onSpec(setLandlock(props.spec, rules().map((r, k) => (k === i ? { ...r, ...patch } : r))));

  return (
    <div class="pw-step">
      <h2>Process and Landlock</h2>
      <p class="warn-note">These sections apply after a recreate of the sandbox (ADR 0008). Saving them on a running sandbox is reported as “requires recreate”.</p>
      <div class="pw-add">
        <label class="pw-field"><span>Run as user</span>
          <input class="pf-input mono" value={proc().run_as_user ?? ''} placeholder="sandbox" onChange={(e) => put({ run_as_user: e.currentTarget.value })} />
        </label>
        <label class="pw-field"><span>Max processes</span>
          <input class="pf-input mono" type="number" min="1" max="4096" value={proc().max_processes ?? ''} onChange={(e) => put({ max_processes: e.currentTarget.value })} />
        </label>
      </div>
      <label class="pw-field"><span>Allowed executables (comma or one per line)</span>
        <textarea class="pf-input mono" rows="3" value={joinList(proc().binaries)} placeholder="/usr/bin/git, /usr/bin/curl"
          onChange={(e) => put({ binaries: e.currentTarget.value })} />
      </label>
      <label class="pw-field"><span>Environment variables that survive</span>
        <input class="pf-input mono" value={joinList(proc().env_allow)} placeholder="PATH, HOME" onChange={(e) => put({ env_allow: e.currentTarget.value })} />
      </label>

      <h3>Landlock paths</h3>
      <p class="muted small">Kernel-enforced grants, the same shape as the filesystem section.</p>
      <Show when={rules().length > 0} fallback={<p class="muted small">No Landlock rule.</p>}>
        <table class="pf-table">
          <thead><tr><th>Path</th><th>Access</th><th /></tr></thead>
          <tbody>
            <For each={rules()}>
              {(rule, i) => (
                <tr>
                  <td><input class="pf-input mono" aria-label="Landlock path" value={rule.path} onChange={(e) => putRule(i(), { path: e.currentTarget.value })} /></td>
                  <td>
                    <span class="pf-checks">
                      <For each={ACCESS}>
                        {(a) => (
                          <label>
                            <input type="checkbox" checked={(rule.access || []).includes(a)}
                              onChange={(e) => {
                                const set = new Set(rule.access || []);
                                if (e.currentTarget.checked) set.add(a); else set.delete(a);
                                putRule(i(), { access: [...set] });
                              }} />{a}
                          </label>
                        )}
                      </For>
                    </span>
                  </td>
                  <td><button type="button" class="tiny" onClick={() => props.onSpec(setLandlock(props.spec, rules().filter((_, k) => k !== i())))}>Remove</button></td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>
      <button type="button" class="secondary tiny" onClick={() => props.onSpec({ ...props.spec, landlock: { rules: [...rules(), { path: '/', access: ['read'] }] } })}>Add Landlock path</button>
    </div>
  );
}
