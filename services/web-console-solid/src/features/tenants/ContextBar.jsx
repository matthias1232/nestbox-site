// Where am I working: organisation and data center, as one breadcrumb-like control.
//
// Reads the organisation tree the API returns for the caller, so it lists only data centers the
// caller can open. Switching remounts the page area (see App.jsx), because every screen reads
// the active data center when it loads.
//
// It reads like the mockup's `Platform ▸ Berlin`: the organisation in the eyebrow face, an arrow,
// then the data center as a borderless select. The words "Data center" are the select's
// accessible name only, never a visible label.

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
  // The organisation that holds the active data center, so the breadcrumb starts with it.
  const currentOrg = () => tree().find((org) => (org.environments || []).some((env) => env.id === session.activeEnvironment()));

  return (
    <Show when={total() > 0}>
      <div class="cc-context-bar">
        <Show when={currentOrg()}>
          <span class="eyebrow cc-context-org">{currentOrg().name}</span>
          <span class="cc-context-sep" aria-hidden="true">▸</span>
        </Show>
        <select
          id="context"
          aria-label="Data center"
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
