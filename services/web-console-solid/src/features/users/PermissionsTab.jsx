// Who has access to a scope, with which role and from where; change it, invite by link.
// Used on a data center page, the Admin Center and the organization (contract C13).
import { For, Show, createEffect, createSignal, on } from 'solid-js';
import '../../styles/users.css';
import * as permissionsApi from '../../api/permissions.js';
import * as usersApi from '../../api/users.js';
import * as tenancy from '../../api/tenancy.js';
import { useSession } from '../../state/session.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import InviteDialog from './InviteDialog.jsx';
import { rolesFor, viaLabel, roleSentence, canRemove, canChangeRole, addableUsers } from './permissions.js';

// Props: scopeType ('organization' | 'environment' | 'sandbox'), scopeId.
export default function PermissionsTab(props) {
  const session = useSession();
  const [rows, setRows] = createSignal([]);
  const [invitations, setInvitations] = createSignal([]);
  const [users, setUsers] = createSignal([]);
  const [pick, setPick] = createSignal({ user: '', role: rolesFor(props.scopeType)[0] || 'member' });
  const [inviting, setInviting] = createSignal(false);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [loaded, setLoaded] = createSignal(false);

  const me = () => session.identity()?.user_id ?? null;
  const readOnly = () => props.scopeType === 'organization';
  const roles = () => rolesFor(props.scopeType);

  async function load() {
    try {
      const data = await permissionsApi.list(props.scopeType, props.scopeId);
      setRows((data.items || []).map((r) => ({ ...r, key: `${r.user_id}:${r.via}` })));
      setInvitations(data.invitations || []);
      if (!readOnly()) {
        try { setUsers((await usersApi.list()).items || []); } catch { setUsers([]); }
      }
      setError(null);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setLoaded(true);
    }
  }
  createEffect(on(() => [props.scopeType, props.scopeId], load));

  async function guard(action) {
    setBusy(true);
    setError(null);
    try {
      await action();
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  const body = (userId, role) => ({ user_id: userId, scope_type: props.scopeType, scope_id: props.scopeId, role });
  const changeRole = (row, role) => guard(() => permissionsApi.set(body(row.user_id, role)));
  const removeRow = (row) => {
    if (!window.confirm(`Remove ${row.display_name || row.email} from this scope?`)) return;
    guard(() => permissionsApi.remove({ user_id: row.user_id, scope_type: props.scopeType, scope_id: props.scopeId }));
  };
  const add = (event) => {
    event.preventDefault();
    const choice = pick();
    if (!choice.user) return;
    guard(async () => {
      await permissionsApi.set(body(choice.user, props.scopeType === 'sandbox' ? 'member' : choice.role));
      setPick({ ...choice, user: '' });
    });
  };

  const columns = [
    { key: 'display_name', label: 'Name', sortable: true, render: (r) => r.display_name || r.email },
    { key: 'email', label: 'Email', sortable: true, render: (r) => <span class="mono small">{r.email}</span> },
    {
      key: 'role', label: 'Role', sortable: true,
      render: (r) => (
        <Show when={!readOnly() && canChangeRole(r, me(), props.scopeType)} fallback={<span>{r.role}</span>}>
          <select value={r.role} disabled={busy()} aria-label={`Role of ${r.display_name || r.email}`}
                  onChange={(e) => changeRole(r, e.currentTarget.value)}>
            <For each={roles()}>{(role) => <option value={role}>{role}</option>}</For>
          </select>
        </Show>
      ),
    },
    { key: 'via', label: 'From', sortable: true, render: (r) => <span class="badge muted"><i />{viaLabel(r.via)}</span> },
    { key: 'since', label: 'Since', sortable: true, type: 'date', render: (r) => <span class="small">{r.since ? new Date(r.since).toLocaleDateString() : '-'}</span> },
    {
      key: 'actions', label: '',
      render: (r) => (
        <Show when={!readOnly() && canRemove(r, me(), rows())}>
          <button type="button" class="tiny danger" disabled={busy()} onClick={() => removeRow(r)}>Remove</button>
        </Show>
      ),
    },
  ];

  return (
    <div class="users-permissions">
      <Show when={readOnly()}>
        <p class="notice">Organisation admins are set by the organisation tree. This list is read-only.</p>
      </Show>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={loaded()} fallback={<p class="muted">Loading…</p>}>
        <SelectTable
          columns={columns}
          rows={rows()}
          rowKey="key"
          empty={{ title: 'Nobody has access yet', hint: readOnly() ? 'No organisation admins.' : 'Add a person or invite someone by link.' }}
        />
      </Show>

      <Show when={!readOnly()}>
        <form class="users-add" onSubmit={add}>
          <label class="field"><span>Person</span>
            <select value={pick().user} onChange={(e) => setPick({ ...pick(), user: e.currentTarget.value })}>
              <option value="">Choose…</option>
              <For each={addableUsers(users(), rows())}>{(u) => <option value={u.id}>{u.display_name} ({u.email})</option>}</For>
            </select>
          </label>
          <label class="field"><span>Role</span>
            <select value={pick().role} disabled={props.scopeType === 'sandbox'} onChange={(e) => setPick({ ...pick(), role: e.currentTarget.value })}>
              <For each={roles()}>{(role) => <option value={role}>{role}</option>}</For>
            </select>
          </label>
          <button type="submit" class="primary" disabled={busy() || !pick().user}>Add person</button>
          <button type="button" class="secondary" onClick={() => setInviting(true)}>Invite by link</button>
          <small class="muted users-sentence">{roleSentence(props.scopeType === 'sandbox' ? 'member' : pick().role)}</small>
        </form>

        <Show when={invitations().length > 0}>
          <h3 class="eyebrow">Pending invitations</h3>
          <ul class="users-invites">
            <For each={invitations()}>
              {(inv) => (
                <li>
                  <span>{inv.role}</span>
                  <span class="mono small">{inv.email || 'anyone with the link'}</span>
                  <span class="muted small">expires {new Date(inv.expires_at).toLocaleDateString()}</span>
                  <button type="button" class="tiny danger" disabled={busy()} onClick={() => guard(() => tenancy.withdraw(inv.id))}>Withdraw</button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>

      <Show when={inviting()}>
        <InviteDialog scopeType={props.scopeType} scopeId={props.scopeId} onClose={() => setInviting(false)} onDone={load} />
      </Show>
    </div>
  );
}
