// Move a sandbox to another node and back.

import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import * as api from '../../api/moves.js';
import * as nodeApi from '../../api/nodes.js';
import * as sandboxApi from '../../api/sandboxes.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const STEPS = ['check', 'image', 'snapshot', 'stop', 'delta', 'restore', 'cutover', 'recreate', 'verify', 'done'];

function size(bytes) {
  if (!bytes) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let n = bytes; let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i += 1; }
  return `${n.toFixed(n >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export default function MovesPage() {
  const [moves, setMoves] = createSignal([]);
  const [nodes, setNodes] = createSignal([]);
  const [boxes, setBoxes] = createSignal([]);
  const [form, setForm] = createSignal({ sandbox: '', node: '', recover: false });
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [open, setOpen] = createSignal(null);

  async function load() {
    try { setMoves((await api.list()).items || []); } catch (failure) { setError(failure.message); }
  }
  onMount(async () => {
    await load();
    try { setNodes((await nodeApi.list()).items || []); } catch { /* ignore */ }
    try { setBoxes((await sandboxApi.list({ limit: 200 })).items || []); } catch { /* ignore */ }
  });
  const timer = setInterval(load, 3000);
  onCleanup(() => clearInterval(timer));

  const boxId = (item) => item.sandbox?.id ?? item.id;
  const boxName = (item) => item.sandbox?.name ?? item.name;
  const boxNode = (item) => item.node_name ?? item.sandbox?.node_name;
  const chosen = () => boxes().find((b) => boxId(b) === form().sandbox);
  const targets = () => nodes().filter((n) => n.name !== boxNode(chosen() || {}) && n.status === 'online');

  async function submit(event) {
    event.preventDefault();
    setError(null);
    setBusy(true);
    try {
      const started = form().recover
        ? await api.recover(form().sandbox, form().node)
        : await api.start(form().sandbox, form().node);
      setOpen(started.id);
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  const progress = (move) => {
    const index = STEPS.indexOf(move.step);
    return move.state === 'done' ? 100 : Math.max(5, Math.round(((index + 1) / STEPS.length) * 100));
  };

  return (
    <section class="page">
      <PageHeader
        title="Moves"
        subtitle={<>Move a sandbox with its folders and the agent's login to another node, work there, and move it back. Only what changed travels on the way back. The old node keeps its files as a cold copy.</>}
        actions={<>
          <HelpTip anchor="moves" />
        </>}
      />

      <Show when={error()}><p class="error">{error()}</p></Show>

      <form class="editor" onSubmit={submit}>
        <h3>Move a sandbox</h3>
        <div class="form-grid">
          <label class="field"><span>Sandbox</span>
            <select required value={form().sandbox} onChange={(e) => setForm({ ...form(), sandbox: e.currentTarget.value, node: '' })}>
              <option value="">Choose ...</option>
              <For each={boxes()}>{(item) => <option value={boxId(item)}>{boxName(item)} (on {boxNode(item) || '?'})</option>}</For>
            </select>
          </label>
          <label class="field"><span>To node</span>
            <select required value={form().node} onChange={(e) => setForm({ ...form(), node: e.currentTarget.value })}>
              <option value="">Choose ...</option>
              <For each={targets()}>{(node) => <option value={node.name}>{node.display_name || node.name}</option>}</For>
            </select>
            <small class="muted">Only online nodes of this data center are offered.</small>
          </label>
        </div>
        <label class="field">
          <span><input type="checkbox" checked={form().recover} onChange={(e) => setForm({ ...form(), recover: e.currentTarget.checked })} /> The node is gone: recover from the last snapshot</span>
          <small class="muted">Only for a node that is offline. The work since its last scheduled snapshot is lost; a normal move loses nothing.</small>
        </label>
        <div class="actions"><button type="submit" disabled={busy() || !form().sandbox || !form().node}>{form().recover ? 'Recover on that node' : 'Start the move'}</button></div>
      </form>

      <h2>History</h2>
      <div class="table-scroll">
        <table>
          <thead><tr><th>Sandbox</th><th>From</th><th>To</th><th>State</th><th>Step</th><th>Sent</th><th>Started</th></tr></thead>
          <tbody>
            <For each={moves()}>
              {(move) => (
                <>
                  <tr onClick={() => setOpen(open() === move.id ? null : move.id)} style="cursor:pointer">
                    <td>{move.sandbox_name}</td><td>{move.from_node}</td><td>{move.to_node}</td>
                    <td><span class={`badge ${move.state === 'done' ? 'ok' : move.state === 'failed' ? 'bad' : 'warn'}`}>{move.state}</span></td>
                    <td>
                      {move.step}
                      <Show when={move.state === 'running'}>
                        <progress max="100" value={progress(move)} style="width:80px;margin-left:8px" />
                      </Show>
                    </td>
                    <td>{size(move.bytes_moved)}</td>
                    <td class="small">{new Date(move.started_at).toLocaleString()}</td>
                  </tr>
                  <Show when={open() === move.id}>
                    <tr><td colspan="7">
                      <Show when={move.error}><p class="error">{move.error}</p></Show>
                      <ol class="small">
                        <For each={move.log}>{(entry) => <li><strong>{entry.step}</strong> {entry.detail}</li>}</For>
                      </ol>
                    </td></tr>
                  </Show>
                </>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </section>
  );
}
