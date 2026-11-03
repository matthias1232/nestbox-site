// Rent a machine, use it, hand it back.

import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import * as api from '../../api/hosting.js';
import * as tenancy from '../../api/tenancy.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const money = (milli) => `${(milli / 1000).toFixed(3)}`;

const BLANK_ACCOUNT = {
  kind: 'hetzner', name: '', token: '', max_hourly_milli: 0,
  api_url: '', node: '', template_vmid: '', address_pool: '', gateway: '', nameserver: '',
};

export default function HostingPage(props) {
  const [accounts, setAccounts] = createSignal([]);
  const [hosts, setHosts] = createSignal([]);
  const [sites, setSites] = createSignal([]);
  const [types, setTypes] = createSignal([]);
  const [account, setAccount] = createSignal({ ...BLANK_ACCOUNT });
  const [rent, setRent] = createSignal({ account_id: '', node_name: '', site_id: '', host_type: '', location: '', ttl_minutes: 240, idle_minutes: 60 });
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const isAdmin = () => props.role === 'admin';

  async function load() {
    try {
      setAccounts((await api.accounts()).items || []);
      setHosts((await api.hosts()).items || []);
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(async () => {
    await load();
    try { const list = (await tenancy.sites()).items || []; setSites(list); if (list[0]) setRent({ ...rent(), site_id: list[0].id }); } catch { /* ignore */ }
  });
  const timer = setInterval(load, 8000);
  onCleanup(() => clearInterval(timer));

  async function pickAccount(id) {
    setRent({ ...rent(), account_id: id, host_type: '', location: '' });
    setTypes([]);
    if (!id) return;
    try { setTypes((await api.types(id)).items || []); } catch (failure) { setError(failure.message); }
  }
  const chosenType = () => types().find((t) => t.name === rent().host_type);

  async function guard(action, done) {
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      await action();
      if (done) setNotice(done);
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  function addAccount(event) {
    event.preventDefault();
    const a = account();
    const config = a.kind === 'proxmox'
      ? {
          api_url: a.api_url, node: a.node, template_vmid: Number(a.template_vmid),
          address_pool: a.address_pool, gateway: a.gateway, nameserver: a.nameserver || undefined,
        }
      : {};
    guard(async () => {
      await api.addAccount({ kind: a.kind, name: a.name, token: a.token, config, max_hourly_milli: Math.round(Number(a.max_hourly_milli) * 1000) });
      setAccount({ ...BLANK_ACCOUNT });
    }, 'Account added. The token is stored encrypted and is not shown again.');
  }

  function rentMachine(event) {
    event.preventDefault();
    const r = rent();
    guard(
      () => api.burst({
        account_id: r.account_id, node_name: r.node_name.trim().toLowerCase(), site_id: r.site_id,
        host_type: r.host_type, location: r.location || undefined,
        ttl_minutes: Number(r.ttl_minutes), idle_minutes: Number(r.idle_minutes),
      }),
      'The machine is being created. It joins this data center by itself and shows up under Nodes in a minute or two.',
    );
  }

  return (
    <section class="page">
      <PageHeader
        title="Rented machines"
        subtitle={<>Rent a server for a while, move your work onto it, move it back, and hand the machine back. A machine is deleted when its time is up or it has sat empty, and never while data is still on it.</>}
        actions={<>
          <HelpTip anchor="nodes" />
        </>}
      />

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <h2>Provider accounts</h2>
      <div class="table-scroll">
        <table>
          <thead><tr><th>Account</th><th>Provider</th><th>Running</th><th>Cost now / h</th><th>Budget / h</th><th /></tr></thead>
          <tbody>
            <For each={accounts()}>
              {(item) => (
                <tr>
                  <td>{item.name}</td><td>{item.kind}</td><td>{item.hosts}</td>
                  <td>{money(item.running_hourly_milli)}</td>
                  <td>{item.max_hourly_milli ? money(item.max_hourly_milli) : 'no cap'}</td>
                  <td class="actions">
                    <Show when={isAdmin()}>
                      <button class="tiny danger" disabled={busy() || item.hosts > 0}
                              onClick={() => guard(() => api.removeAccount(item.id), `${item.name} was removed.`)}>Remove</button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <Show when={isAdmin()}>
        <form class="editor" onSubmit={addAccount}>
          <h3>Add a provider account</h3>
          <div class="form-grid">
            <label class="field"><span>Provider</span>
              <select value={account().kind} onChange={(e) => setAccount({ ...account(), kind: e.currentTarget.value })}>
                <option value="hetzner">Hetzner Cloud</option>
                <option value="digitalocean">DigitalOcean</option>
                <option value="vultr">Vultr</option>
                <option value="proxmox">Proxmox VE (your own)</option>
              </select>
            </label>
            <label class="field"><span>Name</span>
              <input required value={account().name} placeholder="hetzner-main" onInput={(e) => setAccount({ ...account(), name: e.currentTarget.value })} />
            </label>
            <label class="field"><span>{account().kind === 'proxmox' ? 'API token (user@realm!id=secret)' : 'API token'}</span>
              <input required type="password" autocomplete="off" value={account().token} onInput={(e) => setAccount({ ...account(), token: e.currentTarget.value })} />
              <small class="muted">Stored encrypted. Never shown again.</small>
            </label>
            <label class="field"><span>Budget per hour (0 = no cap)</span>
              <input type="number" min="0" step="0.01" value={account().max_hourly_milli} onInput={(e) => setAccount({ ...account(), max_hourly_milli: e.currentTarget.value })} />
              <small class="muted">All running machines of this account together, in the provider's currency.</small>
            </label>
            <Show when={account().kind === 'proxmox'}>
              <label class="field"><span>API address</span><input required value={account().api_url} placeholder="https://pve.example:8006" onInput={(e) => setAccount({ ...account(), api_url: e.currentTarget.value })} /></label>
              <label class="field"><span>Proxmox node</span><input required value={account().node} placeholder="pve02" onInput={(e) => setAccount({ ...account(), node: e.currentTarget.value })} /></label>
              <label class="field"><span>Template VM id</span><input required type="number" value={account().template_vmid} placeholder="9000" onInput={(e) => setAccount({ ...account(), template_vmid: e.currentTarget.value })} />
                <small class="muted">Made with deploy/scripts/make-proxmox-template.sh.</small></label>
              <label class="field"><span>Address pool</span><input required value={account().address_pool} placeholder="192.168.55.60-192.168.55.69" onInput={(e) => setAccount({ ...account(), address_pool: e.currentTarget.value })} /></label>
              <label class="field"><span>Gateway</span><input required value={account().gateway} placeholder="192.168.55.1" onInput={(e) => setAccount({ ...account(), gateway: e.currentTarget.value })} /></label>
            </Show>
          </div>
          <div class="actions"><button type="submit" disabled={busy()}>Add account</button></div>
        </form>

        <Show when={accounts().length > 0}>
          <form class="editor" onSubmit={rentMachine}>
            <h3>Rent a machine</h3>
            <div class="form-grid">
              <label class="field"><span>Account</span>
                <select required value={rent().account_id} onChange={(e) => pickAccount(e.currentTarget.value)}>
                  <option value="">Choose ...</option>
                  <For each={accounts()}>{(a) => <option value={a.id}>{a.name} ({a.kind})</option>}</For>
                </select>
              </label>
              <label class="field"><span>Machine type</span>
                <select required value={rent().host_type} onChange={(e) => setRent({ ...rent(), host_type: e.currentTarget.value, location: '' })}>
                  <option value="">Choose ...</option>
                  <For each={types()}>
                    {(t) => <option value={t.name}>{t.name} - {t.cores} cores, {Math.round(t.memory_mb / 1024)} GB{t.hourly_milli === null ? ', no published price' : t.hourly_milli ? `, ${money(t.hourly_milli)}/h` : ''}</option>}
                  </For>
                </select>
              </label>
              <Show when={chosenType()?.locations?.length > 0}>
                <label class="field"><span>Location</span>
                  <select value={rent().location} onChange={(e) => setRent({ ...rent(), location: e.currentTarget.value })}>
                    <option value="">Provider default</option>
                    <For each={chosenType().locations}>{(l) => <option value={l}>{l}</option>}</For>
                  </select>
                </label>
              </Show>
              <label class="field"><span>Node name</span>
                <input required pattern="[a-z0-9]([a-z0-9-]*[a-z0-9])?" maxlength="42" value={rent().node_name} placeholder="gpu-rent-1" onInput={(e) => setRent({ ...rent(), node_name: e.currentTarget.value })} />
              </label>
              <label class="field"><span>Site</span>
                <select required value={rent().site_id} onChange={(e) => setRent({ ...rent(), site_id: e.currentTarget.value })}>
                  <For each={sites()}>{(s) => <option value={s.id}>{s.name}</option>}</For>
                </select>
              </label>
              <label class="field"><span>Hand back after (minutes)</span>
                <input type="number" min="15" max="10080" value={rent().ttl_minutes} onInput={(e) => setRent({ ...rent(), ttl_minutes: e.currentTarget.value })} />
              </label>
              <label class="field"><span>... or when empty for (minutes, 0 = never)</span>
                <input type="number" min="0" value={rent().idle_minutes} onInput={(e) => setRent({ ...rent(), idle_minutes: e.currentTarget.value })} />
              </label>
            </div>
            <div class="actions"><button type="submit" disabled={busy()}>Rent the machine</button></div>
          </form>
        </Show>
      </Show>

      <h2>Machines</h2>
      <div class="table-scroll">
        <table>
          <thead><tr><th>Node</th><th>Account</th><th>Type</th><th>State</th><th>Address</th><th>Cost so far</th><th>Handed back</th><th /></tr></thead>
          <tbody>
            <For each={hosts()}>
              {(host) => (
                <tr>
                  <td>{host.node_name}</td><td>{host.account}</td><td>{host.host_type}</td>
                  <td><span class={`badge ${host.state === 'running' ? 'ok' : host.state === 'destroyed' ? '' : 'warn'}`}>{host.state}</span></td>
                  <td class="mono small">{host.address || '-'}</td>
                  <td>{money(host.cost_so_far_milli)}</td>
                  <td class="small">
                    {host.destroyed_at ? new Date(host.destroyed_at).toLocaleString() : new Date(host.expires_at).toLocaleString()}
                    <Show when={host.note}><br /><span class="muted">{host.note}</span></Show>
                  </td>
                  <td class="actions">
                    <Show when={isAdmin() && !host.destroyed_at}>
                      <button class="tiny danger" disabled={busy()} onClick={() => guard(() => api.destroy(host.id), `${host.node_name} was handed back.`)}>Delete now</button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </section>
  );
}
