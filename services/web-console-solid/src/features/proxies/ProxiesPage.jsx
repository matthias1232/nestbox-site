// Exit proxies: the catalogue of places traffic can leave from.
//
// The rows are seeded from the appliance's firewall (DE, CH, US, AT, Tor), and
// this screen is where an operator adds, edits, disables or removes one. What it
// deliberately does not do is claim a country: `country` is what the operator
// typed, and the only proof of an exit is a request that comes back from it.
//
// A password is never typed into this form. An exit that needs one gets a
// locator (`file:///…`), and the value stays on the host (ADR 0005).

import { For, Show, createSignal, onMount } from 'solid-js';

import * as api from '../../api/proxies.js';
import StatusBadge from '../../components/StatusBadge.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const blank = {
  name: '',
  kind: 'http',
  host: '',
  port: '',
  country: '',
  provider: 'custom',
  username: '',
  password_ref: '',
  enabled: true,
};

export default function ProxiesPage(props) {
  const [items, setItems] = createSignal([]);
  const [form, setForm] = createSignal({ ...blank });
  const [editing, setEditing] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  const isAdmin = () => props.role === 'admin' || props.role === 'operator';

  async function load() {
    try {
      setItems((await api.list()).items || []);
      setError(null);
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(load);

  function edit(proxy) {
    setEditing(proxy.id);
    setForm({
      name: proxy.name,
      kind: proxy.kind,
      host: proxy.host,
      port: String(proxy.port),
      country: proxy.country || '',
      provider: proxy.provider || 'custom',
      username: proxy.username || '',
      password_ref: proxy.password_ref || '',
      enabled: proxy.enabled,
    });
  }

  function reset() {
    setEditing(null);
    setForm({ ...blank });
  }

  async function save(event) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    setNotice(null);
    const f = form();
    const body = {
      name: f.name.trim(),
      kind: f.kind,
      host: f.host.trim(),
      port: Number(f.port),
      country: f.country.trim() || null,
      provider: f.provider.trim() || null,
      username: f.username.trim() || null,
      password_ref: f.password_ref.trim() || null,
    };
    try {
      if (editing()) {
        const current = items().find((item) => item.id === editing());
        await api.update(editing(), { ...body, enabled: f.enabled, version: current.version });
        setNotice(`“${body.name}” updated.`);
      } else {
        await api.create(body);
        setNotice(`“${body.name}” added.`);
      }
      reset();
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function destroy(proxy) {
    if (!window.confirm(`Remove the exit “${proxy.name}”? Sandboxes and providers using it fall back to the node's own route.`)) return;
    setError(null);
    setNotice(null);
    try {
      await api.remove(proxy.id);
      setNotice(`“${proxy.name}” removed.`);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  return (
    <section class="page">
      <PageHeader
        title="Exit proxies"
        subtitle={<>Where traffic leaves the node. A sandbox chooses one in its Policies tab; a provider profile chooses one in Providers. An exit is used when the sandbox's egress mode is <b>Direct</b>; in the proxied modes the node's own egress proxy is the exit, because that is where policy is enforced.</>}
        actions={<>
          <HelpTip anchor="egress-modes" />
        </>}
      />

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <Show when={isAdmin()}>
        <form class="panel" onSubmit={save}>
          <h2>{editing() ? 'Edit exit' : 'Add an exit'}</h2>
          <div class="grid two">
            <label>
              Name
              <input value={form().name} placeholder="DE — exit 1 (HTTP)" required
                onInput={(e) => setForm({ ...form(), name: e.currentTarget.value })} />
            </label>
            <label>
              Kind
              <select value={form().kind} onChange={(e) => setForm({ ...form(), kind: e.currentTarget.value })}>
                <option value="http">HTTP (CONNECT)</option>
                <option value="socks5">SOCKS5</option>
              </select>
            </label>
            <label>
              Host
              <input value={form().host} placeholder="192.168.55.1" required
                onInput={(e) => setForm({ ...form(), host: e.currentTarget.value })} />
            </label>
            <label>
              Port
              <input type="number" min="1" max="65535" value={form().port} placeholder="22541" required
                onInput={(e) => setForm({ ...form(), port: e.currentTarget.value })} />
            </label>
            <label>
              Country (optional)
              <input value={form().country} placeholder="DE"
                onInput={(e) => setForm({ ...form(), country: e.currentTarget.value })} />
            </label>
            <label>
              Provider (optional)
              <input value={form().provider} placeholder="mullvad"
                onInput={(e) => setForm({ ...form(), provider: e.currentTarget.value })} />
            </label>
            <label>
              Username (optional)
              <input value={form().username}
                onInput={(e) => setForm({ ...form(), username: e.currentTarget.value })} />
            </label>
            <label>
              Password locator (never the value)
              <input value={form().password_ref} placeholder="file:///etc/nestbox/secrets/exit-de"
                onInput={(e) => setForm({ ...form(), password_ref: e.currentTarget.value })} />
            </label>
          </div>
          <Show when={editing()}>
            <label class="inline">
              <input type="checkbox" checked={form().enabled}
                onChange={(e) => setForm({ ...form(), enabled: e.currentTarget.checked })} />
              Enabled
            </label>
          </Show>
          <div class="controls">
            <button class="primary" type="submit" disabled={busy()}>
              {editing() ? 'Save' : 'Add'}
            </button>
            <Show when={editing()}>
              <button class="secondary" type="button" onClick={reset}>Cancel</button>
            </Show>
          </div>
        </form>
      </Show>

      <div class="panel">
        <h2>Catalogue</h2>
        <Show when={items().length > 0} fallback={<p class="muted">No exit is configured. A sandbox with a proxied egress mode still uses the node's egress proxy; only Direct needs an exit.</p>}>
          <div class="table-scroll">
            <table>
              <thead>
                <tr><th>Name</th><th>Kind</th><th>Address</th><th>Country</th><th>Provider</th><th>State</th><th /></tr>
              </thead>
              <tbody>
                <For each={items()}>
                  {(proxy) => (
                    <tr>
                      <td>{proxy.name}</td>
                      <td class="mono small">{proxy.kind}</td>
                      <td class="mono small">{proxy.host}:{proxy.port}</td>
                      <td class="small">{proxy.country || '—'}</td>
                      <td class="small">{proxy.provider || '—'}</td>
                      <td>
                        <StatusBadge status={proxy.enabled ? 'running' : 'stopped'}
                          label={proxy.enabled ? 'enabled' : 'disabled'} />
                      </td>
                      <td class="actions">
                        <Show when={isAdmin()}>
                          <button class="tiny" onClick={() => edit(proxy)}>Edit</button>
                          <button class="tiny danger" onClick={() => destroy(proxy)}>Remove</button>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </div>
    </section>
  );
}
