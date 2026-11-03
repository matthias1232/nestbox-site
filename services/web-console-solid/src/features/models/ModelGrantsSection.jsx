// Model grants for the whole environment and for one person (admins only).
//
// A grant names a provider profile and the name an agent uses for it. The list
// here is the grants themselves, not the effective list of a sandbox: that one
// belongs to the Workbox, where the owner may add a grant to their own sandbox.

import { For, Show, createSignal, onMount } from 'solid-js';
import * as modelGrants from '../../api/modelGrants.js';
import * as providers from '../../api/providers.js';
import * as users from '../../api/users.js';
import { useSession } from '../../state/session.jsx';

// The grants endpoint answers with a bare array; tolerate an `items` envelope.
function asRows(answer) {
  if (Array.isArray(answer)) return answer;
  return answer?.items || [];
}

const BLANK = { scope: 'environment', user_id: '', profile: '', model_name: '', is_default: false, keep_warm: false };

export default function ModelGrantsSection() {
  const session = useSession();
  const [grants, setGrants] = createSignal([]);
  const [profiles, setProfiles] = createSignal([]);
  const [people, setPeople] = createSignal([]);
  const [form, setForm] = createSignal({ ...BLANK });
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  const isAdmin = () => session.identity()?.role === 'admin';

  async function load() {
    try {
      setGrants(asRows(await modelGrants.list({})));
    } catch (failure) {
      setError(failure.message);
    }
  }

  onMount(async () => {
    if (!isAdmin()) return;
    await load();
    try {
      const list = asRows(await providers.list());
      // A reachable profile first: it is the one that can answer right away.
      setProfiles([...list].sort((left, right) => {
        const rank = (profile) => (profile.status === 'reachable' ? 0 : 1);
        return rank(left) - rank(right) || String(left.name).localeCompare(String(right.name));
      }));
    } catch (failure) {
      setError(failure.message);
    }
    try {
      setPeople(asRows(await users.list()));
    } catch {
      /* a user grant then has no name to show, which the row says in words */
    }
  });

  const profileName = (id) => profiles().find((profile) => profile.id === id)?.name || 'a provider profile';
  const personName = (id) => people().find((person) => person.id === id)?.display_name || 'a user';

  function appliesTo(grant) {
    if (grant.scope === 'user') return personName(grant.user_id);
    if (grant.scope === 'sandbox') return 'one sandbox';
    return 'This environment';
  }

  function chooseProfile(profileId) {
    const next = profiles().find((item) => item.id === profileId);
    const prior = profiles().find((item) => item.id === form().profile);
    const current = form().model_name;
    // Only an untouched name follows the new profile; a typed one is kept.
    const untouched = !current || current === (prior?.model || '');
    setForm({ ...form(), profile: profileId, model_name: untouched ? (next?.model || '') : current });
  }

  async function submit(event) {
    event.preventDefault();
    const name = form().model_name.trim();
    if (!name) {
      setError('A model name is required; the agents ask for it by that name.');
      return;
    }
    if (form().scope === 'user' && !form().user_id) {
      setError('A user grant needs the person it applies to.');
      return;
    }
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const body = {
        scope: form().scope,
        model_name: name,
        provider_profile_id: form().profile,
        is_default: form().is_default,
        keep_warm: form().keep_warm,
      };
      if (form().scope === 'user') body.user_id = form().user_id;
      await modelGrants.create(body);
      setNotice(`${name} granted to ${form().scope === 'user' ? personName(form().user_id) : 'the whole environment'}.`);
      setForm({ ...BLANK });
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function revoke(grant) {
    if (!window.confirm(`Revoke ${grant.model_name} from ${appliesTo(grant)}?`)) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await modelGrants.remove(grant.id);
      setNotice(`${grant.model_name} revoked.`);
      await load();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  // Everyone but an admin sees nothing: the section is an admin tool, not a
  // disabled form.
  if (!isAdmin()) return null;

  return (
    <div>
      <h2>Model grants</h2>
      <p class="muted">
        Which models the agents may use. A grant to this environment reaches every sandbox here;
        a grant to a person reaches the sandboxes that person owns. A sandbox's own grant is made
        in its Workbox and beats both.
      </p>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <form class="editor" onSubmit={submit}>
        <div class="form-grid">
          <label class="field">
            <span>Scope</span>
            <select value={form().scope}
                    onChange={(event) => setForm({ ...form(), scope: event.currentTarget.value, user_id: '' })}>
              <option value="environment">The whole environment</option>
              <option value="user">One person</option>
            </select>
          </label>
          <Show when={form().scope === 'user'}>
            <label class="field">
              <span>Person</span>
              <select required value={form().user_id}
                      onChange={(event) => setForm({ ...form(), user_id: event.currentTarget.value })}>
                <option value="">Choose …</option>
                <For each={people()}>
                  {(person) => <option value={person.id}>{person.display_name} ({person.role})</option>}
                </For>
              </select>
              <small class="muted">Reaches the sandboxes this person owns, not the shared ones.</small>
            </label>
          </Show>
          <label class="field">
            <span>Provider profile</span>
            <select required value={form().profile}
                    onChange={(event) => chooseProfile(event.currentTarget.value)}>
              <option value="">Choose …</option>
              <For each={profiles()}>
                {(profile) => (
                  <option value={profile.id}>
                    {profile.name} · {profile.kind || 'endpoint'}
                    {profile.status === 'reachable' ? '' : ` (${profile.status || 'unprobed'})`}
                  </option>
                )}
              </For>
            </select>
          </label>
          <label class="field">
            <span>Model name</span>
            <input required value={form().model_name}
                   placeholder="the name the agents use"
                   onInput={(event) => setForm({ ...form(), model_name: event.currentTarget.value })} />
            <small class="muted">Defaults to the profile's model; the agents ask for it by this name.</small>
          </label>
        </div>
        <label class="muted small">
          <input type="checkbox" checked={form().is_default}
                 onChange={(event) => setForm({ ...form(), is_default: event.currentTarget.checked })} />
          {' '}Default model
        </label>
        <label class="muted small">
          <input type="checkbox" checked={form().keep_warm}
                 onChange={(event) => setForm({ ...form(), keep_warm: event.currentTarget.checked })} />
          {' '}Keep it loaded when no sandbox runs
        </label>
        <div class="actions">
          <button type="submit" class="primary" disabled={busy() || !form().profile}>
            {busy() ? 'Granting…' : 'Grant model'}
          </button>
        </div>
      </form>

      <Show when={grants().length} fallback={<p class="muted">No model is granted yet. Agents use the gateway's default model.</p>}>
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th>Scope</th>
                <th>Applies to</th>
                <th>Model</th>
                <th>Provider profile</th>
                <th>Default</th>
                <th>Keep warm</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <For each={grants()}>
                {(grant) => (
                  <tr>
                    <td><span class="badge muted"><i />{grant.scope}</span></td>
                    <td>{appliesTo(grant)}</td>
                    <td>
                      <strong>{grant.model_name}</strong>
                      <Show when={grant.upstream_model && grant.upstream_model !== grant.model_name}>
                        <div class="muted small mono">{grant.upstream_model}</div>
                      </Show>
                    </td>
                    <td class="small">{profileName(grant.provider_profile_id)}</td>
                    <td>{grant.is_default ? 'yes' : <span class="muted">no</span>}</td>
                    <td>{grant.keep_warm ? 'yes' : <span class="muted">no</span>}</td>
                    <td class="actions">
                      <button class="tiny danger" disabled={busy()} onClick={() => revoke(grant)}>
                        Revoke
                      </button>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}
