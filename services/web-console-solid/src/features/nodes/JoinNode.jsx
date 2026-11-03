// Add a machine as a node: pick a site and a name, get one line to paste on the machine.
//
// The line downloads an install script that sets up WireGuard, trades a one-time token for an
// identity and starts the node's runner. The token is good for 15 minutes and works once. The
// private key is made on the machine and never sent.

import { For, Show, createSignal, onMount } from 'solid-js';
import * as api from '../../api/nodes.js';
import * as tenancy from '../../api/tenancy.js';

export default function JoinNode(props) {
  const [sites, setSites] = createSignal([]);
  const [site, setSite] = createSignal('');
  const [name, setName] = createSignal('');
  const [made, setMade] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  onMount(async () => {
    try {
      const list = (await tenancy.sites()).items || [];
      setSites(list);
      if (list.length > 0) setSite(list[0].id);
    } catch (failure) {
      setError(failure.message);
    }
  });

  async function submit(event) {
    event.preventDefault();
    setError(null);
    setMade(null);
    setBusy(true);
    try {
      setMade(await api.joinToken(name().trim().toLowerCase(), site()));
      props.onChanged?.();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <form class="editor" onSubmit={submit}>
      <h3>Add a node</h3>
      <p class="muted">
        Any Linux machine with a public address or outbound internet: a rented server, a cloud GPU,
        a box at home. It needs no open port; it dials out to this control plane.
      </p>
      <div class="form-grid">
        <label class="field"><span>Site</span>
          <select required value={site()} onChange={(e) => setSite(e.currentTarget.value)}>
            <For each={sites()}>{(item) => <option value={item.id}>{item.name} ({item.kind})</option>}</For>
          </select>
        </label>
        <label class="field"><span>Node name</span>
          <input required pattern="[a-z0-9]([a-z0-9-]*[a-z0-9])?" maxlength="42" value={name()} placeholder="gpu-brev-1"
                 onInput={(e) => setName(e.currentTarget.value)} />
          <small class="muted">Lowercase letters, digits and dashes.</small>
        </label>
      </div>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <div class="actions"><button type="submit" disabled={busy() || !site()}>Create join command</button></div>
      <Show when={made()}>
        <p class="notice">
          Run this on <strong>{made().node_name}</strong> as root. It works once, until{' '}
          {new Date(made().expires_at).toLocaleTimeString()}:
          <br />
          <input class="mono" readonly style="width:100%" value={made().command}
                 onFocus={(e) => e.currentTarget.select()} />
        </p>
      </Show>
    </form>
  );
}
