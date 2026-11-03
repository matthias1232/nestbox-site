// Handing out sandboxes and inviting people (ADR 0045, 0046).
//
// An invitation is a link. For a sandbox it makes the person a member who sees and works in that
// one sandbox and nothing else. The link is shown once; only its hash is stored.

import { For, Show, createSignal, onMount } from 'solid-js';
import * as api from '../../api/tenancy.js';
import * as sandboxApi from '../../api/sandboxes.js';

export default function AccessSection(props) {
  const [invites, setInvites] = createSignal([]);
  const [boxes, setBoxes] = createSignal([]);
  const [form, setForm] = createSignal({ role: 'member', sandbox_id: '', email: '' });
  const [link, setLink] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [share, setShare] = createSignal({ sandbox: '', user: '' });

  async function load() {
    try {
      setInvites((await api.invitations()).items || []);
      const page = await sandboxApi.list({ limit: 200 });
      setBoxes(page.items || []);
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(load);

  async function guard(action) {
    setError(null);
    setBusy(true);
    try {
      await action();
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  const boxName = (item) => item.sandbox?.name ?? item.name;
  const boxId = (item) => item.sandbox?.id ?? item.id;
  const sandboxLabel = (item) => {
    if (!item.sandbox_id) return 'whole data center';
    const found = boxes().find((b) => boxId(b) === item.sandbox_id);
    return found ? boxName(found) : 'a sandbox';
  };
  const members = () => (props.users || []).filter((u) => u.role !== 'viewer');

  return (
    <div>
      <h2>Share a sandbox</h2>
      <p class="muted">Gives one person access to one sandbox. They work in it fully and see nothing else.</p>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <form class="editor" onSubmit={(event) => {
        event.preventDefault();
        guard(async () => {
          await api.grant(share().sandbox, share().user);
          setShare({ sandbox: '', user: '' });
        });
      }}>
        <div class="form-grid">
          <label class="field"><span>Sandbox</span>
            <select required value={share().sandbox} onChange={(e) => setShare({ ...share(), sandbox: e.currentTarget.value })}>
              <option value="">Choose ...</option>
              <For each={boxes()}>{(item) => <option value={boxId(item)}>{boxName(item)}</option>}</For>
            </select>
          </label>
          <label class="field"><span>Person</span>
            <select required value={share().user} onChange={(e) => setShare({ ...share(), user: e.currentTarget.value })}>
              <option value="">Choose ...</option>
              <For each={members()}>{(user) => <option value={user.id}>{user.display_name} ({user.role})</option>}</For>
            </select>
            <small class="muted">A viewer cannot work in a sandbox. Make them a member first.</small>
          </label>
        </div>
        <div class="actions"><button type="submit" disabled={busy()}>Share</button></div>
      </form>

      <h2>Invitations</h2>
      <div class="table-scroll">
        <table>
          <thead><tr><th>Role</th><th>Sandbox</th><th>Email</th><th>Expires</th><th>State</th><th /></tr></thead>
          <tbody>
            <For each={invites()}>
              {(item) => (
                <tr>
                  <td>{item.role}</td>
                  <td>{sandboxLabel(item)}</td>
                  <td class="mono small">{item.email || 'anyone with the link'}</td>
                  <td class="small">{new Date(item.expires_at).toLocaleDateString()}</td>
                  <td>{item.accepted_at ? 'used' : new Date(item.expires_at) < new Date() ? 'expired' : 'open'}</td>
                  <td class="actions">
                    <Show when={!item.accepted_at}>
                      <button class="tiny danger" disabled={busy()} onClick={() => guard(() => api.withdraw(item.id))}>Withdraw</button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <form class="editor" onSubmit={(event) => {
        event.preventDefault();
        guard(async () => {
          const body = { role: form().role };
          if (form().sandbox_id) body.sandbox_id = form().sandbox_id;
          if (form().email.trim()) body.email = form().email.trim();
          const made = await api.invite(body);
          setLink(`${window.location.origin}/?invite=${made.token}`);
        });
      }}>
        <h3>Invite someone</h3>
        <div class="form-grid">
          <label class="field"><span>Sandbox</span>
            <select value={form().sandbox_id} onChange={(e) => setForm({ ...form(), sandbox_id: e.currentTarget.value })}>
              <option value="">The whole data center</option>
              <For each={boxes()}>{(item) => <option value={boxId(item)}>{boxName(item)} only</option>}</For>
            </select>
            <small class="muted">For one sandbox the person is always a member.</small>
          </label>
          <label class="field"><span>Role</span>
            <select value={form().role} disabled={!!form().sandbox_id}
                    onChange={(e) => setForm({ ...form(), role: e.currentTarget.value })}>
              <option value="admin">admin</option><option value="operator">operator</option>
              <option value="member">member</option><option value="viewer">viewer</option>
            </select>
          </label>
          <label class="field"><span>Email (optional)</span>
            <input type="email" value={form().email} placeholder="only this address may use the link"
                   onInput={(e) => setForm({ ...form(), email: e.currentTarget.value })} />
          </label>
        </div>
        <div class="actions"><button type="submit" disabled={busy()}>Create invitation link</button></div>
      </form>
      <Show when={link()}>
        <p class="notice">
          Send this link once; it is not shown again:
          <br /><input class="mono" readonly style="width:100%" value={link()} onFocus={(e) => e.currentTarget.select()} />
        </p>
      </Show>
    </div>
  );
}
