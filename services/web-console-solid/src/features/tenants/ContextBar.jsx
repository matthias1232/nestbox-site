// Where am I working: organisation and data center.
//
// Reads the organisation tree the API returns for the caller, so it lists only data centers the
// caller can open. Switching remounts the page area (see App.jsx), because every screen reads
// the active data center when it loads.

import { For, Show, createSignal, onMount } from 'solid-js';
import { useSession } from '../../state/session.jsx';
import * as api from '../../api/tenancy.js';

export default function ContextBar() {
  const session = useSession();
  const [tree, setTree] = createSignal([]);

  onMount(async () => {
    try {
      setTree((await api.organizations()).items || []);
    } catch {
      /* the bar is a convenience; the page still works without it */
    }
  });

  const total = () => tree().reduce((n, org) => n + org.environments.length, 0);

  return (
    <Show when={total() > 0}>
      <div class="cc-context-bar">
        <label for="context">Data center</label>
        <select
          id="context"
          value={session.activeEnvironment()}
          onChange={(event) => session.switchEnvironment(event.currentTarget.value)}
        >
          <For each={tree()}>
            {(org) => (
              <optgroup label={org.name}>
                <For each={org.environments}>
                  {(env) => <option value={env.id}>{env.name}</option>}
                </For>
              </optgroup>
            )}
          </For>
        </select>
      </div>
    </Show>
  );
}
