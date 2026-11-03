// Users of this environment (admins only).
//
// A user manages their own sandboxes from start to end; an admin sees and manages all of
// them and sets what everyone shares (providers and keys, policies, gateways). Roles:
//   admin    everything, including users, providers, keys and global policy;
//   operator their own sandboxes: create, run, policies on them, terminal, backups;
//   viewer   read-only, on their own sandboxes and the shared ones.

import { For, Show, createSignal, onMount } from 'solid-js';
import '../../styles/users.css';
import * as api from '../../api/users.js';
import { useSession } from '../../state/session.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import Tabs from '../../components/Tabs.jsx';
import SelectTable from '../../components/SelectTable.jsx';
import Switch from '../../components/Switch.jsx';
import Drawer from '../../components/Drawer.jsx';
import Empty from '../../components/Empty.jsx';
import AccessSection from './AccessSection.jsx';
import PermissionsTab from './PermissionsTab.jsx';
import { roleSentence } from './permissions.js';

const ROLES = [
  ['admin', 'Admin — everything, including users, providers, keys and global policy'],
  ['operator', 'Operator — manages their own sandboxes from start to end'],
  ['member', 'Member — works in the sandboxes they own or were given, nothing else'],
  ['viewer', 'Viewer — read-only on their own and shared sandboxes'],
];

const blank = { email: '', display_name: '', password: '', role: 'operator' };

export default function UsersPage(props) {
  const session = useSession();
  const [tab, setTab] = createSignal('people');
  const [drawer, setDrawer] = createSignal(false);
  const [items, setItems] = createSignal([]);
  const [form, setForm] = createSignal({ ...blank });
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [passwordFor, setPasswordFor] = createSignal(null);
  const [newPassword, setNewPassword] = createSignal('');

  async function load() {
    try {
      setItems((await api.list()).items || []);
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

  const submit = (event) => {
    event.preventDefault();
    guard(async () => {
      await api.create(form());
      setForm({ ...blank });
      setDrawer(false);
    }, 'User created. Give them the password through a channel of your own; it is not shown again.');
  };

  const changeRole = (user, role) =>
    guard(() => api.update(user.id, { role }), `${user.display_name} is now ${role}. Their sessions ended; they sign in again.`);
  const toggle = (user) =>
    guard(
      () => api.update(user.id, { status: user.status === 'active' ? 'disabled' : 'active' }),
      user.status === 'active' ? `${user.display_name} is disabled and signed out.` : `${user.display_name} is active again.`,
    );
  const remove = (user) => {
    if (!window.confirm(`Remove ${user.display_name} from this environment? Their sandboxes stay and become shared; an admin can reassign or delete them.`)) return;
    guard(() => api.remove(user.id), `${user.display_name} was removed.`);
  };
  const savePassword = (event) => {
    event.preventDefault();
    const user = passwordFor();
    guard(async () => {
      await api.setPassword(user.id, newPassword());
      setPasswordFor(null);
      setNewPassword('');
    }, `New password set for ${user.display_name}; their sessions ended.`);
  };

  const columns = [
    { key: 'display_name', label: 'Name', sortable: true, render: (u) => `${u.display_name}${u.is_you ? ' (you)' : ''}` },
    { key: 'email', label: 'Email', sortable: true, render: (u) => <span class="mono small">{u.email}</span> },
    {
      key: 'role', label: 'Role', sortable: true,
      render: (u) => (
        <select value={u.role} disabled={busy() || u.is_you} aria-label={`Role of ${u.display_name}`}
                title={u.is_you ? 'You cannot change your own role' : roleSentence(u.role)}
                onChange={(e) => changeRole(u, e.currentTarget.value)}>
          <For each={ROLES}>{([value]) => <option value={value}>{value}</option>}</For>
        </select>
      ),
    },
    {
      key: 'status', label: 'Status', sortable: true,
      render: (u) => <Switch checked={u.status === 'active'} disabled={busy() || u.is_you} label={u.status === 'active' ? 'Active' : 'Disabled'} onChange={() => toggle(u)} />,
    },
    { key: 'sandboxes', label: 'Sandboxes', sortable: true, type: 'number', align: 'right' },
    {
      key: 'actions', label: '',
      render: (u) => (
        <span class="actions">
          <button type="button" class="tiny" onClick={() => setPasswordFor(u)}>Set password…</button>
          <button type="button" class="tiny danger" disabled={busy() || u.is_you} onClick={() => remove(u)}>Remove</button>
        </span>
      ),
    },
  ];

  return (
    <Show when={props.role === 'admin'} fallback={<section class="page"><Empty title="Admins only" hint="Ask an admin of this data center to change users and permissions." /></section>}>
      <section class="page">
        <Breadcrumb items={[{ label: 'Administration' }, { label: 'Users & permissions' }]} />
        <PageHeader
          title="Users & permissions"
          subtitle={`${items().length} user${items().length === 1 ? '' : 's'} in this data center. Each user manages their own sandboxes; admins see all of them.`}
          actions={<button type="button" class="primary" onClick={() => setDrawer(true)}>New user</button>}
        />
        <Tabs tabs={[{ id: 'people', label: 'People' }, { id: 'environment', label: 'This data center' }]} active={tab()} onChange={setTab} />

        <Show when={error()}><p class="error">{error()}</p></Show>
        <Show when={notice()}><p class="notice">{notice()}</p></Show>

        <Show when={tab() === 'people'}>
          <SelectTable columns={columns} rows={items()} rowKey="id"
                       empty={{ title: 'No users yet', hint: 'Create the first user.', action: <button type="button" class="primary" onClick={() => setDrawer(true)}>New user</button> }} />

          <Show when={passwordFor()}>
            <form class="users-password" onSubmit={savePassword}>
              <h3>New password for {passwordFor().display_name}</h3>
              <label class="field">
                <span>Password</span>
                <input type="password" autocomplete="new-password" minlength="12" required
                       value={newPassword()} onInput={(e) => setNewPassword(e.currentTarget.value)} />
                <small class="muted">At least 12 characters. Their current sessions end, so they sign in again.</small>
              </label>
              <div class="actions">
                <button type="submit" class="primary" disabled={busy()}>Set password</button>
                <button type="button" class="secondary" onClick={() => setPasswordFor(null)}>Cancel</button>
              </div>
            </form>
          </Show>

          <AccessSection users={items()} />
        </Show>

        <Show when={tab() === 'environment'}>
          <Show when={session.activeEnvironment()} fallback={<Empty title="No data center selected" hint="Pick one in the context bar." />}>
            <PermissionsTab scopeType="environment" scopeId={session.activeEnvironment()} />
          </Show>
        </Show>

        <Drawer open={drawer()} title="New user" subtitle="They sign in with the email and password below." onClose={() => setDrawer(false)}>
          <form class="users-drawer-form" onSubmit={submit}>
            <label class="field"><span>Email</span>
              <input type="email" required value={form().email} placeholder="name@example.com"
                     onInput={(e) => setForm({ ...form(), email: e.currentTarget.value })} />
            </label>
            <label class="field"><span>Name</span>
              <input required maxlength="80" value={form().display_name} placeholder="Ada Lovelace"
                     onInput={(e) => setForm({ ...form(), display_name: e.currentTarget.value })} />
            </label>
            <label class="field"><span>Role</span>
              <select value={form().role} onChange={(e) => setForm({ ...form(), role: e.currentTarget.value })}>
                <For each={ROLES}>{([value, text]) => <option value={value}>{text}</option>}</For>
              </select>
            </label>
            <label class="field"><span>Initial password</span>
              <input type="password" autocomplete="new-password" required minlength="12" value={form().password}
                     onInput={(e) => setForm({ ...form(), password: e.currentTarget.value })} />
              <small class="muted">At least 12 characters. Hand it over once.</small>
            </label>
            <div class="actions"><button type="submit" class="primary" disabled={busy()}>Create user</button></div>
          </form>
        </Drawer>
      </section>
    </Show>
  );
}
