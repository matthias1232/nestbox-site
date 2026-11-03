// Customers, data centers and sites.

import { For, Show, createSignal, onMount } from 'solid-js';
import * as api from '../../api/tenancy.js';
import PermissionsTab from '../users/PermissionsTab.jsx';

const KINDS = [
  ['home', 'Home - a machine you run yourself'],
  ['cloud', 'Cloud - rented by the hour'],
  ['edge', 'Edge - a small machine near the work'],
];

export default function TenantsPage(props) {
  const [tree, setTree] = createSignal([]);
  const [sites, setSites] = createSignal([]);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [customer, setCustomer] = createSignal('');
  const [center, setCenter] = createSignal('');
  const [site, setSite] = createSignal({ name: '', kind: 'cloud', provider: '', region: '' });

  async function load() {
    try {
      setTree((await api.organizations()).items || []);
      setSites((await api.sites()).items || []);
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(load);

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

  const isAdmin = () => props.role === 'admin';
  // Only a provider's administrator can add customers; the API says no to anyone else, this
  // just keeps the form out of the way of people who cannot use it.
  const isProvider = () => tree().some((org) => org.kind === 'platform' || org.kind === 'partner');

  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>Data centers &amp; sites</h1>
          <p class="muted">
            A data center is where one team runs its sandboxes. A site is a place that has nodes:
            your own machine, a rented server, a small box at the edge.
          </p>
        </div>
      </header>

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <For each={tree()}>
        {(org) => (
          <div class="card">
            <h3>{org.name} <span class="muted small">{org.kind}</span></h3>
            <div class="table-scroll">
              <table>
                <thead><tr><th>Data center</th><th>Sites</th><th>Nodes</th><th>Sandboxes</th></tr></thead>
                <tbody>
                  <For each={org.environments}>
                    {(env) => (
                      <tr>
                        <td>{env.name}</td><td>{env.sites}</td><td>{env.nodes}</td><td>{env.sandboxes}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
            <h4 class="eyebrow">Permissions</h4>
            <PermissionsTab scopeType="organization" scopeId={org.id} />
          </div>
        )}
      </For>

      <h2>Sites of this data center</h2>
      <div class="table-scroll">
        <table>
          <thead><tr><th>Name</th><th>Kind</th><th>Provider</th><th>Region</th><th>Overlay</th><th>Nodes</th><th /></tr></thead>
          <tbody>
            <For each={sites()}>
              {(item) => (
                <tr>
                  <td>{item.name}</td>
                  <td>{item.kind}</td>
                  <td>{item.provider || '-'}</td>
                  <td>{item.region || '-'}</td>
                  <td class="mono small">{item.overlay_cidr || '-'}</td>
                  <td>{item.nodes}</td>
                  <td class="actions">
                    <Show when={isAdmin()}>
                      <button class="tiny danger" disabled={busy() || item.nodes > 0}
                              title={item.nodes > 0 ? 'Move or remove its nodes first' : 'Remove this site'}
                              onClick={() => guard(() => api.removeSite(item.id), `${item.name} was removed.`)}>
                        Remove
                      </button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <Show when={isAdmin()}>
        <form class="editor" onSubmit={(event) => {
          event.preventDefault();
          guard(async () => {
            await api.createSite(site());
            setSite({ name: '', kind: 'cloud', provider: '', region: '' });
          }, 'Site added.');
        }}>
          <h3>Add a site</h3>
          <div class="form-grid">
            <label class="field"><span>Name</span>
              <input required maxlength="80" value={site().name} placeholder="Hetzner Falkenstein"
                     onInput={(e) => setSite({ ...site(), name: e.currentTarget.value })} />
            </label>
            <label class="field"><span>Kind</span>
              <select value={site().kind} onChange={(e) => setSite({ ...site(), kind: e.currentTarget.value })}>
                <For each={KINDS}>{([value, text]) => <option value={value}>{text}</option>}</For>
              </select>
            </label>
            <label class="field"><span>Provider</span>
              <input value={site().provider} placeholder="hetzner, brev, proxmox ..."
                     onInput={(e) => setSite({ ...site(), provider: e.currentTarget.value })} />
            </label>
            <label class="field"><span>Region</span>
              <input value={site().region} placeholder="fsn1"
                     onInput={(e) => setSite({ ...site(), region: e.currentTarget.value })} />
            </label>
          </div>
          <div class="actions"><button type="submit" disabled={busy()}>Add site</button></div>
        </form>

        <form class="editor" onSubmit={(event) => {
          event.preventDefault();
          guard(async () => { await api.createDataCenter(center()); setCenter(''); }, 'Data center created.');
        }}>
          <h3>Add a data center</h3>
          <label class="field"><span>Name</span>
            <input required maxlength="80" value={center()} placeholder="Research"
                   onInput={(e) => setCenter(e.currentTarget.value)} />
            <small class="muted">You become its administrator. It starts with one site.</small>
          </label>
          <div class="actions"><button type="submit" disabled={busy()}>Create data center</button></div>
        </form>
      </Show>

      <Show when={isAdmin() && isProvider()}>
        <form class="editor" onSubmit={(event) => {
          event.preventDefault();
          guard(async () => { await api.createCustomer(customer()); setCustomer(''); }, 'Customer created.');
        }}>
          <h3>Add a customer</h3>
          <label class="field"><span>Company name</span>
            <input required maxlength="80" value={customer()} placeholder="Acme GmbH"
                   onInput={(e) => setCustomer(e.currentTarget.value)} />
            <small class="muted">
              The customer gets its own data center and site. You reach it from here; its people
              reach only their own.
            </small>
          </label>
          <div class="actions"><button type="submit" disabled={busy()}>Create customer</button></div>
        </form>
      </Show>
    </section>
  );
}
