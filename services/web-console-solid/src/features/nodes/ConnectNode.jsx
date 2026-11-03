// Connect an existing machine by its address: name, site, SSH login, and how the
// node reaches the internet.
//
// `JoinNode` hands out a command to paste on the machine. This form is for the
// machine whose address and login the operator already has (a rented GPU box, a
// server in a rack): the control plane logs in over SSH once and installs the
// node itself. The password or key is used for that one call and is not stored,
// which is why both credential fields are emptied as soon as the request is
// answered, whatever the answer was.
//
// Props: `onChanged` (called after the request was accepted).

import { For, Show, createSignal, onMount } from 'solid-js';
import * as api from '../../api/nodesConnect.js';
import * as tenancy from '../../api/tenancy.js';

const DEFAULT_SSH_PORT = '22';
const DEFAULT_SSH_USER = 'root';
const DEFAULT_WG_PORT = '51820';

export default function ConnectNode(props) {
  const [sites, setSites] = createSignal([]);
  const [site, setSite] = createSignal('');
  const [name, setName] = createSignal('');
  const [host, setHost] = createSignal('');
  const [sshPort, setSshPort] = createSignal(DEFAULT_SSH_PORT);
  const [sshUser, setSshUser] = createSignal(DEFAULT_SSH_USER);
  const [credential, setCredential] = createSignal('password');
  const [password, setPassword] = createSignal('');
  const [privateKey, setPrivateKey] = createSignal('');
  const [wgPort, setWgPort] = createSignal(DEFAULT_WG_PORT);
  const [lock, setLock] = createSignal('strict');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);

  onMount(async () => {
    try {
      const list = (await tenancy.sites()).items || [];
      setSites(list);
      if (list.length > 0) setSite(list[0].id);
    } catch (failure) {
      setError(failure.message);
    }
  });

  // A credential field is only required for the method that is selected, so the
  // hidden one must not block the form.
  function hasCredential() {
    return credential() === 'password' ? password().length > 0 : privateKey().trim().length > 0;
  }

  function ready() {
    return Boolean(site() && name().trim() && host().trim() && hasCredential());
  }

  async function submit(event) {
    event.preventDefault();
    if (busy() || !ready()) return;
    setError(null);
    setNotice(null);
    setBusy(true);
    const body = {
      node_name: name().trim().toLowerCase(),
      site_id: site(),
      host: host().trim(),
      ssh_port: Number(sshPort()) || Number(DEFAULT_SSH_PORT),
      ssh_user: sshUser().trim() || DEFAULT_SSH_USER,
      password: credential() === 'password' ? password() : null,
      private_key: credential() === 'key' ? privateKey() : null,
      wg_listen_port: Number(wgPort()) || Number(DEFAULT_WG_PORT),
      egress_lock: lock(),
    };
    try {
      const answer = await api.connect(body);
      setNotice(
        `Connecting ${answer?.node_name || body.node_name}… this takes a few minutes; ` +
          'the node appears below when its tunnel is up.',
      );
      props.onChanged?.();
    } catch (failure) {
      setError(failure.message);
    } finally {
      // The secret leaves the page the moment the request is answered.
      setPassword('');
      setPrivateKey('');
      setBusy(false);
    }
  }

  return (
    <form class="editor" onSubmit={submit}>
      <h3>Connect a machine by address</h3>
      <p class="muted">
        For a machine that already exists and whose address you know. This control plane logs in
        over SSH once, installs the node and sets up its tunnel; nothing is installed on a machine
        it cannot reach.
      </p>
      <div class="form-grid">
        <label class="field"><span>Site</span>
          <select required value={site()} onChange={(e) => setSite(e.currentTarget.value)}>
            <For each={sites()}>{(item) => <option value={item.id}>{item.name} ({item.kind})</option>}</For>
          </select>
        </label>
        <label class="field"><span>Node name</span>
          <input required pattern="[a-z0-9]([a-z0-9-]*[a-z0-9])?" maxlength="42" value={name()}
                 placeholder="gpu-hetzner-1" onInput={(e) => setName(e.currentTarget.value)} />
          <small class="muted">Lowercase letters, digits and dashes.</small>
        </label>
        <label class="field"><span>Address</span>
          <input required value={host()} placeholder="203.0.113.10 or box.example.com"
                 onInput={(e) => setHost(e.currentTarget.value)} />
          <small class="muted">The public IP or host name this gateway reaches over SSH.</small>
        </label>
        <label class="field"><span>SSH port</span>
          <input type="number" min="1" max="65535" value={sshPort()}
                 onInput={(e) => setSshPort(e.currentTarget.value)} />
        </label>
        <label class="field"><span>SSH user</span>
          <input required maxlength="64" value={sshUser()}
                 onInput={(e) => setSshUser(e.currentTarget.value)} />
          <small class="muted">A user that may become root.</small>
        </label>
        <label class="field"><span>WireGuard port</span>
          <input type="number" min="1" max="65535" value={wgPort()}
                 onInput={(e) => setWgPort(e.currentTarget.value)} />
          <small class="muted">UDP on the node, where this gateway's tunnels end.</small>
        </label>
      </div>

      <p class="muted small">Sign in with</p>
      <div class="form-grid">
        <label class="draft-card">
          <span class="mono small">
            <input type="radio" name="connect-credential" value="password" checked={credential() === 'password'}
                   onChange={() => setCredential('password')} />{' '}Password
          </span>
        </label>
        <label class="draft-card">
          <span class="mono small">
            <input type="radio" name="connect-credential" value="key" checked={credential() === 'key'}
                   onChange={() => setCredential('key')} />{' '}Private key
          </span>
        </label>
      </div>
      <Show
        when={credential() === 'password'}
        fallback={
          <label class="field"><span>Private key</span>
            <textarea rows="6" class="mono" value={privateKey()} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                      onInput={(e) => setPrivateKey(e.currentTarget.value)} />
          </label>
        }
      >
        <label class="field"><span>Password</span>
          <input type="password" autocomplete="off" value={password()}
                 onInput={(e) => setPassword(e.currentTarget.value)} />
        </label>
      </Show>
      <p class="muted small">Used once to install the node, never stored.</p>

      <p class="muted small">Traffic from this node</p>
      <div class="form-grid">
        <label class="draft-card">
          <span class="mono small">
            <input type="radio" name="connect-lock" value="strict" checked={lock() === 'strict'}
                   onChange={() => setLock('strict')} />{' '}Strict — everything goes through this
            gateway (recommended)
          </span>
        </label>
        <label class="draft-card">
          <span class="mono small">
            <input type="radio" name="connect-lock" value="exit_local" checked={lock() === 'exit_local'}
                   onChange={() => setLock('exit_local')} />{' '}Exit locally — decisions here, traffic
            leaves at the node
          </span>
        </label>
      </div>

      <p class="muted small">
        The machine must allow inbound UDP {wgPort() || DEFAULT_WG_PORT} (WireGuard) from this
        gateway's public address.
      </p>

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <div class="actions">
        <button type="submit" disabled={busy() || !ready()}>
          {busy() ? 'Connecting…' : 'Connect machine'}
        </button>
      </div>
    </form>
  );
}
