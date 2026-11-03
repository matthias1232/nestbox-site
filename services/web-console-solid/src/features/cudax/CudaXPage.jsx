// CUDA-X is a real catalogue, not a wish list.
//
// A component can be assigned and installed on any node. A node without a GPU,
// an old driver or a missing container runtime produces a warning next to the
// row and the state `installed_unsupported`; it never disables the action. That
// is the point of this screen: an operator may prepare a machine before the
// hardware arrives, and the console must say what will not work instead of
// pretending the action is impossible.

import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import * as cudax from '../../api/cudax.js';
import * as nodesApi from '../../api/nodes.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

export default function CudaXPage() {
  const [components, setComponents] = createSignal([]);
  const [installs, setInstalls] = createSignal([]);
  const [nodes, setNodes] = createSignal([]);
  const [node, setNode] = createSignal('');
  const [search, setSearch] = createSignal('');
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal('');
  const [force, setForce] = createSignal({});

  async function load() {
    const [catalog, assigned, machines] = await Promise.all([
      cudax.components(),
      cudax.installs(node() || undefined),
      nodesApi.list(),
    ]);
    setComponents(catalog.items || []);
    setInstalls(assigned.items || []);
    setNodes(machines.items || []);
    if (!node() && machines.items?.length) {
      setNode(machines.items[0].name);
    }
  }

  onMount(() => {
    load().catch((failure) => setError(failure.message));
  });

  const installationFor = (key) =>
    installs().find((row) => row.component?.key === key && row.node_name === node());

  const warningsFor = (key) => installationFor(key)?.assessment?.warnings || [];

  const visible = createMemo(() => {
    const needle = search().trim().toLowerCase();
    if (!needle) return components();
    return components().filter((item) =>
      `${item.name} ${item.key} ${item.category}`.toLowerCase().includes(needle));
  });

  const summary = createMemo(() => {
    const rows = installs().filter((row) => row.node_name === node());
    return {
      installed: rows.filter((row) => row.state === 'installed').length,
      unsupported: rows.filter((row) => row.state === 'installed_unsupported').length,
      failed: rows.filter((row) => row.state === 'failed').length,
      warnings: rows.reduce((count, row) => count + (row.assessment?.warnings?.length || 0), 0),
    };
  });

  async function run(key, action, label) {
    setBusy(`${action}:${key}`);
    setError(null);
    setNotice(null);
    try {
      if (action === 'assign') {
        const result = await cudax.assign(key, node(), Boolean(force()[key]));
        setNotice(`${result.component?.name || key}: assigned to ${result.node_name}.`);
      } else if (action === 'install') {
        const result = await cudax.install(key, node(), Boolean(force()[key]));
        setNotice(
          `${result.component}: ${result.state} on ${result.node}` +
          (result.warnings?.length ? ` — ${result.warnings.length} warning(s).` : '.'),
        );
      } else {
        await cudax.remove(key, node());
        setNotice(`${key}: assignment removed from ${node()}.`);
      }
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy('');
    }
  }

  return (
    <section class="page">
      <PageHeader
        title="CUDA-X"
        subtitle={<>Components you can install on a node. Warnings describe what the hardware will not support; they never replace your decision.</>}
        actions={<>
          <HelpTip anchor="nodes" />
        </>}
      />

      <div class="toolbar">
        <label class="field">
          <span>Node</span>
          <select value={node()} onChange={(event) => {
            setNode(event.currentTarget.value);
            load().catch((failure) => setError(failure.message));
          }}>
            <For each={nodes()}>{(item) => <option value={item.name}>{item.name}</option>}</For>
          </select>
        </label>
        <label class="field">
          <span>Find a component</span>
          <input value={search()} placeholder="TensorRT, NCCL, driver …"
                 onInput={(event) => setSearch(event.currentTarget.value)} />
        </label>
        <div class="muted small">
          {summary().installed} installed · {summary().unsupported} installed with warnings ·{' '}
          {summary().failed} failed · {summary().warnings} warning(s)
        </div>
      </div>

      <Show when={notice()}><p class="notice">{notice()}</p></Show>
      <Show when={error()}><p class="error">{error()}</p></Show>

      <div class="table-scroll">
        <table>
          <thead>
            <tr>
              <th>Component</th>
              <th>What it is for</th>
              <th>Requirements</th>
              <th>State on {node() || 'this node'}</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            <For each={visible()}>{(item) => {
              const row = () => installationFor(item.key);
              return (
                <tr>
                  <td>
                    <b>{item.name}</b>
                    <div class="muted small mono">{item.key} · {item.version}</div>
                    <Show when={item.docs_url}>
                      <a class="small" href={item.docs_url} target="_blank" rel="noreferrer">docs</a>
                    </Show>
                  </td>
                  <td class="small">{item.description}</td>
                  <td class="small mono">
                    <div>{item.requires_gpu ? 'NVIDIA GPU' : 'no GPU required'}</div>
                    <Show when={item.min_driver}><div class="muted">driver ≥ {item.min_driver}</div></Show>
                    <Show when={item.min_cuda}><div class="muted">CUDA ≥ {item.min_cuda}</div></Show>
                  </td>
                  <td class="small">
                    <Show when={row()} fallback={<span class="badge">not assigned</span>}>
                      <span class={`badge ${row().state === 'failed' ? 'warn' : ''}`}>
                        <i />{row().state.replaceAll('_', ' ')}
                      </span>
                      <For each={warningsFor(item.key)}>
                        {(warning) => <div class="muted small">{warning}</div>}
                      </For>
                      <Show when={row().last_error}>
                        <div class="error small">{row().last_error}</div>
                      </Show>
                    </Show>
                  </td>
                  <td class="actions">
                    <label class="small">
                      <input type="checkbox" checked={Boolean(force()[item.key])}
                             onChange={(event) => setForce({ ...force(), [item.key]: event.currentTarget.checked })} />
                      {' '}force
                    </label>
                    <button class="tiny" disabled={busy() !== ''}
                            onClick={() => run(item.key, 'assign', 'Assign')}>
                      {row() ? 'Re-assess' : 'Assign'}
                    </button>
                    <button class="tiny" disabled={busy() !== ''}
                            onClick={() => run(item.key, 'install', 'Install')}>
                      Install
                    </button>
                    <Show when={row()}>
                      <button class="tiny danger" disabled={busy() !== ''}
                              onClick={() => run(item.key, 'remove', 'Remove')}>
                        Remove
                      </button>
                    </Show>
                  </td>
                </tr>
              );
            }}</For>
          </tbody>
        </table>
      </div>

      <p class="muted small">
        Installing pulls the images the catalog declares for the component. Components distributed
        only as host packages are recorded and reported as unsupported instead of pretending the
        packages were installed.
      </p>
    </section>
  );
}
