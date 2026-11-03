// MCP servers: the registry every sandbox picks its tools from.
//
// A server is defined once here and attached to the sandboxes that should use it. An HTTP
// server is reached through the sandbox's supervisor, so each call is checked against policy
// and logged; attaching it allows that one endpoint for that sandbox. A stdio server runs
// inside the sandbox and cannot be observed, which the list says plainly.

import { For, Show, createSignal, onMount } from 'solid-js';
import * as api from '../../api/mcp.js';
import * as sandboxApi from '../../api/sandboxes.js';
import SandboxPicker from '../policies/SandboxPicker.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { MCP_HELP } from './mcpHelp.js';
import '../../styles/sandbox-hierarchy.css';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const blank = {
  key: '', name: '', description: '', transport: 'http', endpoint: '', command: '', args: '',
  auth_type: 'none', env_slots: '', packs: '', enabled: true,
};
const lines = (text) => text.split('\n').map((item) => item.trim()).filter(Boolean);

export default function McpPage(props) {
  const [servers, setServers] = createSignal([]);
  const [sandboxes, setSandboxes] = createSignal([]);
  const [form, setForm] = createSignal({ ...blank });
  const [editing, setEditing] = createSignal(null); // key being edited
  const [attaching, setAttaching] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const isAdmin = () => props.role === 'admin';

  async function load() {
    try {
      setServers((await api.list()).items || []);
      setSandboxes(((await sandboxApi.list({ limit: 100 })).items || []).filter((item) => item.status === 'running'));
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(load);

  const body = () => {
    const f = form();
    return {
      key: f.key.trim(), name: f.name.trim(), description: f.description.trim() || null,
      transport: f.transport,
      endpoint: f.transport === 'stdio' ? null : f.endpoint.trim() || null,
      command: f.transport === 'stdio' ? f.command.trim() || null : null,
      args: f.transport === 'stdio' ? lines(f.args) : [],
      auth_type: f.auth_type, env_slots: lines(f.env_slots), packs: lines(f.packs), enabled: f.enabled,
    };
  };

  async function save(event) {
    event.preventDefault();
    setError(null);
    setNotice(null);
    try {
      if (editing()) await api.update(editing(), body());
      else await api.create(body());
      setNotice(editing() ? `“${form().name}” saved.` : `“${form().name}” added to the registry.`);
      setForm({ ...blank });
      setEditing(null);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  function edit(server) {
    setEditing(server.key);
    setForm({
      key: server.key, name: server.name, description: server.description || '', transport: server.transport,
      endpoint: server.endpoint || '', command: server.command || '', args: (server.args || []).join('\n'),
      auth_type: server.auth_type, env_slots: (server.env_slots || []).join('\n'),
      packs: (server.packs || []).join('\n'), enabled: server.enabled,
    });
    window.scrollTo({ top: document.body.scrollHeight, behavior: 'smooth' });
  }

  async function remove(server) {
    if (!window.confirm(`Delete “${server.name}” from the registry?`)) return;
    setError(null);
    try {
      await api.remove(server.key);
      setNotice(`“${server.name}” was deleted.`);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  async function attachTo(ids) {
    const server = attaching();
    setError(null);
    const results = await Promise.allSettled(ids.map((id) => api.attach(id, server.key)));
    const failed = results.filter((item) => item.status === 'rejected');
    setNotice(`“${server.name}” attached to ${results.length - failed.length} sandbox(es).${failed.length ? ` ${failed.length} failed: ${failed.map((f) => f.reason.message).join('; ')}` : ''}`);
    setAttaching(null);
  }

  const set = (patch) => setForm({ ...form(), ...patch });

  return (
    <section class="page">
      <PageHeader
        title="MCP servers"
        subtitle={<>Tool servers an agent can call. Define one here, attach it to the sandboxes that should use it. Calls to an <b>http</b> server are checked and logged; a <b>stdio</b> server runs inside the sandbox and cannot be observed.</>}
        actions={<>
          <HelpTip anchor="policies" />
        </>}
      />
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <Show when={attaching()}>
        <SandboxPicker
          title={`Attach “${attaching().name}” to sandboxes`}
          hint={attaching().transport === 'stdio'
            ? 'A stdio server runs inside the sandbox; attaching records the choice and writes it to the agent configuration.'
            : `Each chosen sandbox may then reach ${attaching().endpoint || 'this server'} and nothing else because of it.`}
          sandboxes={sandboxes()} allowAll={false} confirmLabel="Attach" onConfirm={attachTo} onCancel={() => setAttaching(null)}
        />
      </Show>

      <div class="table-scroll">
        <table class="mcp-table">
          <thead><tr><th>Name</th><th>Transport</th><th>Endpoint or command</th><th>Credential</th><th>Agents</th><th>Status</th><th /></tr></thead>
          <tbody>
            <For each={servers()}>
              {(server) => (
                <tr>
                  <td><b>{server.name}</b><div class="muted small">{server.key}{server.description ? ` · ${server.description}` : ''}</div></td>
                  <td>
                    <span class={`runtime-tag ${server.transport === 'stdio' ? 'docker' : 'openshell'}`}
                          title={server.transport === 'stdio' ? 'Runs inside the sandbox; not observable' : 'Crosses the boundary: checked and logged'}>
                      {server.transport}
                    </span>
                  </td>
                  <td class="mono small">{server.endpoint || `${server.command || ''} ${(server.args || []).join(' ')}`}</td>
                  <td class="small">{server.auth_type}{server.env_slots?.length ? ` · ${server.env_slots.join(', ')}` : ''}</td>
                  <td class="small">{server.packs?.length ? server.packs.join(', ') : 'all'}</td>
                  <td><StatusBadge status={server.enabled ? 'running' : 'stopped'} label={server.enabled ? 'enabled' : 'disabled'} /></td>
                  <td class="actions">
                    <button class="tiny" disabled={!server.enabled} onClick={() => setAttaching(server)}>Attach…</button>
                    <Show when={isAdmin()}>
                      <button class="tiny" onClick={() => edit(server)}>Edit</button>
                      <button class="tiny danger" onClick={() => remove(server)}>Delete</button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <Show when={isAdmin()} fallback={<p class="muted small">Only an admin adds or changes servers in the registry.</p>}>
        <form class="editor" onSubmit={save}>
          <h3>{editing() ? `Edit ${editing()}` : 'Add an MCP server'}</h3>
          <div class="form-grid">
            <label class="field"><span>Id</span>
              <input required disabled={!!editing()} value={form().key} placeholder="github" onInput={(e) => set({ key: e.currentTarget.value })} />
              <small class="muted">{MCP_HELP.key}</small></label>
            <label class="field"><span>Name</span>
              <input required value={form().name} placeholder="GitHub" onInput={(e) => set({ name: e.currentTarget.value })} />
              <small class="muted">{MCP_HELP.name}</small></label>
            <label class="field"><span>What it is for</span>
              <input value={form().description} onInput={(e) => set({ description: e.currentTarget.value })} />
              <small class="muted">{MCP_HELP.description}</small></label>
            <label class="field"><span>Transport</span>
              <select value={form().transport} onChange={(e) => set({ transport: e.currentTarget.value })}>
                <option value="http">http (observed)</option><option value="sse">sse (observed)</option><option value="stdio">stdio (inside the sandbox)</option>
              </select>
              <small class="muted">{MCP_HELP.transport}</small></label>
            <Show when={form().transport !== 'stdio'}>
              <label class="field"><span>Endpoint URL</span>
                <input required value={form().endpoint} placeholder="https://api.example.com/mcp" onInput={(e) => set({ endpoint: e.currentTarget.value })} />
                <small class="muted">{MCP_HELP.endpoint}</small></label>
            </Show>
            <Show when={form().transport === 'stdio'}>
              <label class="field"><span>Command</span>
                <input required value={form().command} placeholder="npx" onInput={(e) => set({ command: e.currentTarget.value })} />
                <small class="muted">{MCP_HELP.command}</small></label>
              <label class="field"><span>Arguments</span>
                <textarea rows="3" value={form().args} placeholder={'-y\n@modelcontextprotocol/server-filesystem'} onInput={(e) => set({ args: e.currentTarget.value })} />
                <small class="muted">{MCP_HELP.args}</small></label>
            </Show>
            <label class="field"><span>Credential</span>
              <select value={form().auth_type} onChange={(e) => set({ auth_type: e.currentTarget.value })}>
                <option value="none">none</option><option value="token">token</option><option value="oauth">oauth</option>
              </select>
              <small class="muted">{MCP_HELP.auth_type}</small></label>
            <label class="field"><span>Environment variables needed</span>
              <textarea rows="2" value={form().env_slots} placeholder="GITHUB_PERSONAL_ACCESS_TOKEN" onInput={(e) => set({ env_slots: e.currentTarget.value })} />
              <small class="muted">{MCP_HELP.env_slots}</small></label>
            <label class="field"><span>Agents that may use it</span>
              <textarea rows="2" value={form().packs} placeholder="claude-code" onInput={(e) => set({ packs: e.currentTarget.value })} />
              <small class="muted">{MCP_HELP.packs}</small></label>
            <label class="field rp-check"><span>Enabled</span>
              <input type="checkbox" checked={form().enabled} onChange={(e) => set({ enabled: e.currentTarget.checked })} />
              <small class="muted">{MCP_HELP.enabled}</small></label>
          </div>
          <div class="actions">
            <button type="submit" class="primary">{editing() ? 'Save changes' : 'Add server'}</button>
            <Show when={editing()}><button type="button" class="secondary" onClick={() => { setEditing(null); setForm({ ...blank }); }}>Cancel</button></Show>
          </div>
        </form>
      </Show>
    </section>
  );
}
