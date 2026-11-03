// Which images are on which nodes, and what should be.

import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import * as api from '../../api/images.js';
import * as tenancy from '../../api/tenancy.js';
import Empty from '../../components/Empty.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const TONE = { present: 'ok', pulling: 'warn', failed: 'bad', missing: 'bad' };

function formatBytes(value) {
  if (!value) return '';
  const units = ['B', 'KB', 'MB', 'GB'];
  let n = value;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i += 1; }
  return `${n.toFixed(n >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export default function ImagesPage(props) {
  const [matrix, setMatrix] = createSignal({ images: [], nodes: [], cells: [], assignments: [] });
  const [catalog, setCatalog] = createSignal([]);
  const [sites, setSites] = createSignal([]);
  const [form, setForm] = createSignal({ image: '', scope_type: 'environment', scope_id: '' });
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  async function load() {
    try {
      setMatrix(await api.matrix());
    } catch (failure) {
      setError(failure.message);
    }
  }

  onMount(async () => {
    await load();
    try { setCatalog((await api.registry()).items || []); } catch { /* the form takes a typed name too */ }
    try { setSites((await tenancy.sites()).items || []); } catch { /* ignore */ }
  });
  const timer = setInterval(load, 10000);
  onCleanup(() => clearInterval(timer));

  const cell = (image, node) => matrix().cells.find((c) => c.image === image && c.node_id === node.id);
  const isAdmin = () => props.role === 'admin';
  const targets = () => {
    const type = form().scope_type;
    if (type === 'site') return sites().map((s) => [s.id, s.name]);
    if (type === 'node') return matrix().nodes.map((n) => [n.id, n.name]);
    return [];
  };

  async function submit(event) {
    event.preventDefault();
    setError(null);
    setNotice(null);
    setBusy(true);
    try {
      const body = { image: form().image.trim(), scope_type: form().scope_type };
      if (body.scope_type !== 'environment') body.scope_id = form().scope_id;
      await api.assign(body);
      setNotice('Assigned. The nodes pick it up within a minute.');
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section class="page">
      <PageHeader
        title="Image registry &amp; sync"
        subtitle={<>Every node holds the same image bytes: the control plane keeps one registry and each node pulls what it was assigned. Nothing is copied between machines.</>}
        actions={<>
          <HelpTip anchor="nodes" />
        </>}
      />

      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <Show when={matrix().nodes.length === 0}>
        <Empty title="No node has joined this data center yet" hint="Add one on the Nodes page; images are assigned to nodes once they exist." />
      </Show>

      <Show when={matrix().nodes.length > 0 && matrix().images.length > 0}>
        <div class="table-scroll">
          <table>
            <thead>
              <tr><th>Image</th><For each={matrix().nodes}>{(node) => <th>{node.name}<br /><span class="muted small">{node.status}</span></th>}</For></tr>
            </thead>
            <tbody>
              <For each={matrix().images}>
                {(image) => (
                  <tr>
                    <td class="mono small">{image}</td>
                    <For each={matrix().nodes}>
                      {(node) => {
                        const c = () => cell(image, node);
                        return (
                          <td title={c()?.error || ''}>
                            <Show when={c()} fallback={<span class="muted">-</span>}>
                              <span class={`badge ${TONE[c().state] || ''}`}>{c().state}</span>
                              <span class="muted small"> {formatBytes(c().size_bytes)}</span>
                            </Show>
                          </td>
                        );
                      }}
                    </For>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

      <h2>Assignments</h2>
      <div class="table-scroll">
        <table>
          <thead><tr><th>Image</th><th>Goes to</th><th /></tr></thead>
          <tbody>
            <For each={matrix().assignments}>
              {(item) => (
                <tr>
                  <td class="mono small">{item.image}</td>
                  <td>{item.scope_type === 'environment' ? item.scope_name : `${item.scope_type} ${item.scope_name}`}</td>
                  <td class="actions">
                    <Show when={isAdmin()}>
                      <button class="tiny danger" disabled={busy()}
                              onClick={async () => { try { await api.unassign(item.id); await load(); } catch (f) { setError(f.message); } }}>
                        Withdraw
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
        <form class="editor" onSubmit={submit}>
          <h3>Assign an image</h3>
          <div class="form-grid">
            <label class="field"><span>Image</span>
              <input required list="registry-images" value={form().image} placeholder="nestbox-claude-code:local"
                     onInput={(e) => setForm({ ...form(), image: e.currentTarget.value })} />
              <datalist id="registry-images"><For each={catalog()}>{(name) => <option value={name} />}</For></datalist>
              <small class="muted">Pick from the registry, or type a name that is on its way.</small>
            </label>
            <label class="field"><span>Goes to</span>
              <select value={form().scope_type} onChange={(e) => setForm({ ...form(), scope_type: e.currentTarget.value, scope_id: '' })}>
                <option value="environment">Every node of this data center</option>
                <option value="site">Every node of one site</option>
                <option value="node">One node</option>
              </select>
            </label>
            <Show when={form().scope_type !== 'environment'}>
              <label class="field"><span>{form().scope_type === 'site' ? 'Site' : 'Node'}</span>
                <select required value={form().scope_id} onChange={(e) => setForm({ ...form(), scope_id: e.currentTarget.value })}>
                  <option value="">Choose ...</option>
                  <For each={targets()}>{([id, name]) => <option value={id}>{name}</option>}</For>
                </select>
              </label>
            </Show>
          </div>
          <div class="actions"><button type="submit" disabled={busy()}>Assign</button></div>
        </form>
      </Show>
    </section>
  );
}
