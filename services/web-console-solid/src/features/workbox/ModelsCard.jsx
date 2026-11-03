// The models this sandbox may use: where each grant comes from, whether the
// model is ready, a warm-up button, and a way to grant one more model.
//
// The list is the server's effective list (`control.models` or
// `GET /sandboxes/{id}/models`); this card never merges grants itself. Readiness
// arrives as `models_changed` events on the live stream, because a warm-up runs
// in the background and a spinner would claim a result the server has not
// answered yet.

import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import * as modelGrants from '../../api/modelGrants.js';
import * as providers from '../../api/providers.js';
import * as sandboxApi from '../../api/sandboxes.js';
import Empty from '../../components/Empty.jsx';
import { useSession } from '../../state/session.jsx';
import {
  canManageModels,
  readinessBadge,
  readinessFromEvents,
  sourceLabel,
} from './models.js';
import { notYetGranted } from './controlStrip.js';
import { onControl } from './useLive.js';

// How many readiness events are kept. Only the last one per model is shown, so
// a bounded buffer is enough and the map cannot grow without end.
const KEEP_EVENTS = 200;

// The grants endpoint answers with a bare array; tolerate an `items` envelope
// so a future paging change does not empty the table.
function asRows(answer) {
  if (Array.isArray(answer)) return answer;
  return answer?.items || [];
}

export default function ModelsCard(props) {
  const session = useSession();

  // Readiness as it arrived on the stream, kept as a list so the last event of a
  // model overwrites the earlier ones.
  const [events, setEvents] = createSignal([]);
  // The effective list after this card changed a grant; `null` means "the one
  // the page handed in".
  const [effective, setEffective] = createSignal(null);
  const [owner, setOwner] = createSignal(null);
  const [profiles, setProfiles] = createSignal([]);
  const [grants, setGrants] = createSignal([]);
  const [adding, setAdding] = createSignal(false);
  const [form, setForm] = createSignal({ profile: '', model_name: '', is_default: false });
  // Models the user may use that this sandbox does not have yet, and the chosen one
  // (`''` nothing yet, `'__other'` the free form below).
  const [candidates, setCandidates] = createSignal([]);
  const [pick, setPick] = createSignal('');
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [busy, setBusy] = createSignal(false);
  const [warmKey, setWarmKey] = createSignal(null);

  const sandboxId = () => props.sandboxId;
  const rows = () => effective() ?? (props.control?.models || []);
  const readiness = createMemo(() => readinessFromEvents(events()));
  const mayManage = createMemo(() =>
    canManageModels(session.identity()?.role, owner(), session.identity()?.user_id));

  async function loadEffective() {
    setEffective(asRows(await modelGrants.sandboxModels(sandboxId())));
  }

  async function loadGrants() {
    setGrants(asRows(await modelGrants.list({ sandbox_id: sandboxId() })));
  }

  async function loadProfiles() {
    if (profiles().length) return profiles();
    const answer = await providers.list();
    const list = asRows(answer);
    // A reachable profile first: it is the one that can answer right away.
    const sorted = [...list].sort((left, right) => {
      const rank = (profile) => (profile.status === 'reachable' ? 0 : 1);
      return rank(left) - rank(right) || String(left.name).localeCompare(String(right.name));
    });
    setProfiles(sorted);
    return sorted;
  }

  // The owner is only needed for someone who is neither operator nor admin; the
  // summary the page holds does not carry it, so ask the list once. The grants
  // are loaded in every case, because a revoke needs the id of the row.
  onMount(async () => {
    const identity = session.identity();
    setOwner(props.control?.owner_user_id ?? null);
    if (!['operator', 'admin'].includes(identity?.role)) {
      try {
        const page = await sandboxApi.list({ limit: 200 });
        const found = (page?.items || []).find((item) => item.id === sandboxId());
        setOwner(found?.owner_user_id ?? null);
      } catch {
        /* an unknown owner hides the button; the server would refuse anyway */
      }
    }
    try {
      await loadGrants();
    } catch {
      /* removal stays unavailable until the grants can be read */
    }
  });

  const stop = onControl(sandboxId, (event) => {
    if (event.type !== 'models_changed') return;
    setEvents((current) => [...current, event].slice(-KEEP_EVENTS));
  });
  onCleanup(stop);

  function profileName(profileId) {
    const found = profiles().find((profile) => profile.id === profileId);
    return found ? found.name : 'a provider profile';
  }

  function grantOf(row) {
    return grants().find((grant) => grant.scope === 'sandbox' && grant.model_name === row.model_name);
  }

  // The grants the user already holds, minus the names this sandbox has: the
  // two-click path is "pick one, grant".
  async function loadCandidates() {
    const identity = session.identity();
    try {
      const mine = asRows(await modelGrants.list({ user_id: identity?.user_id }));
      setCandidates(notYetGranted(mine, rows()));
    } catch {
      setCandidates([]);
    }
  }

  async function openAdd() {
    setError(null);
    setNotice(null);
    setAdding(true);
    setPick('');
    await loadCandidates();
    if (candidates().length) return;
    setPick('__other');
    try {
      const list = await loadProfiles();
      if (!list.length) {
        setError('No provider profile is configured, so there is no endpoint to grant a model from.');
        return;
      }
      if (!form().profile) {
        setForm({ ...form(), profile: list[0].id, model_name: list[0].model || '' });
      }
    } catch (failure) {
      setError(failure.message);
    }
  }

  // A profile switch may bring its own model name: only an untouched name
  // follows the new profile, a typed one is left alone.
  function chooseProfile(profileId) {
    const next = profiles().find((item) => item.id === profileId);
    const prior = profiles().find((item) => item.id === form().profile);
    const current = form().model_name;
    const untouched = !current || current === (prior?.model || '');
    setForm({ ...form(), profile: profileId, model_name: untouched ? (next?.model || '') : current });
  }

  // Choosing "another endpoint" needs the provider list for the free form.
  async function choosePick(value) {
    setPick(value);
    if (value !== '__other') return;
    try {
      const list = await loadProfiles();
      if (!form().profile && list.length) {
        setForm({ ...form(), profile: list[0].id, model_name: list[0].model || '' });
      }
    } catch (failure) {
      setError(failure.message);
    }
  }

  async function submitAdd(event) {
    event.preventDefault();
    const chosen = candidates().find((item) => item.model_name === pick());
    if (chosen) {
      await grantChosen(chosen);
      return;
    }
    const name = form().model_name.trim();
    if (!name) {
      setError('A model name is required; the agents ask for it by that name.');
      return;
    }
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await modelGrants.create({
        scope: 'sandbox',
        sandbox_id: sandboxId(),
        model_name: name,
        provider_profile_id: form().profile,
        is_default: form().is_default,
      });
      setNotice(`${name} is granted to this sandbox. The agents see it after their next start.`);
      setAdding(false);
      setForm({ profile: '', model_name: '', is_default: false });
      await afterChange();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function grantChosen(chosen) {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await modelGrants.create({
        scope: 'sandbox',
        sandbox_id: sandboxId(),
        model_name: chosen.model_name,
        provider_profile_id: chosen.provider_profile_id,
        is_default: false,
      });
      setNotice(`${chosen.model_name} is granted to this sandbox. The agents see it after their next start.`);
      setAdding(false);
      setPick('');
      await afterChange();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function removeGrant(row) {
    const grant = grantOf(row);
    if (!grant) {
      setError(`No sandbox grant for ${row.model_name} was found; reload the page and try again.`);
      return;
    }
    if (!window.confirm(`Revoke ${row.model_name} from this sandbox? Agents that ask for it fall back to the next grant.`)) {
      return;
    }
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await modelGrants.remove(grant.id);
      setNotice(`${row.model_name} is no longer granted to this sandbox.`);
      await afterChange();
    } catch (failure) {
      setError(failure.message);
    } finally {
      setBusy(false);
    }
  }

  async function warm(row) {
    const key = row.upstream_model || row.model_name;
    setWarmKey(key);
    setError(null);
    setNotice(null);
    try {
      await modelGrants.warm(key);
      setNotice(`Warming ${key}. The readiness appears here when the model answers.`);
    } catch (failure) {
      setError(failure.message);
    } finally {
      setWarmKey(null);
    }
  }

  // After a change the card reloads both lists, so the row it shows is the
  // server's answer and not the request it sent.
  async function afterChange() {
    try {
      await loadEffective();
    } catch (failure) {
      setError(failure.message);
    }
    try {
      await loadGrants();
    } catch {
      /* removal stays unavailable, the new list is already shown */
    }
  }

  return (
    <section>
      <div class="draft-head">
        <h2>Models</h2>
        <Show when={mayManage()}>
          <button class="tiny secondary" disabled={busy()} onClick={openAdd}>
            Add model
          </button>
        </Show>
      </div>
      <p class="muted small">
        The models the agents of this sandbox may ask for, and where each grant comes from.
      </p>

      <Show when={error()}><p class="error small">{error()}</p></Show>
      <Show when={notice()}><p class="notice small">{notice()}</p></Show>

      <Show when={adding()}>
        <form class="draft-card" onSubmit={submitAdd}>
          <Show when={candidates().length > 0}>
            <label class="field">
              <span>Model you may use</span>
              <select required value={pick()} onChange={(event) => choosePick(event.currentTarget.value)}>
                <option value="">Choose …</option>
                <For each={candidates()}>
                  {(item) => <option value={item.model_name}>{item.model_name}</option>}
                </For>
                <option value="__other">Another endpoint …</option>
              </select>
            </label>
          </Show>
          <Show when={pick() === '__other'}>
          <div class="form-grid">
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
            {' '}Make this the model the agents start with.
          </label>
          </Show>
          <div class="actions">
            <button type="submit" class="primary tiny"
                    disabled={busy() || !pick() || (pick() === '__other' && !form().profile)}>
              {busy() ? 'Granting…' : 'Grant model'}
            </button>
            <button type="button" class="tiny secondary"
                    onClick={() => { setAdding(false); setError(null); }}>
              Cancel
            </button>
          </div>
        </form>
      </Show>

      <Show when={rows().length > 0} fallback={
        <Empty title="No models granted."
               hint="The agents use the gateway's default model." />
      }>
        <div class="table-scroll">
          <table class="dense">
            <thead>
              <tr>
                <th>Model</th>
                <th>Granted to</th>
                <th>Readiness</th>
                <th />
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row) => (
                  <tr>
                    <td>
                      <strong>{row.model_name}</strong>
                      <Show when={row.is_default}>
                        <span class="badge info"><i />default</span>
                      </Show>
                      <Show when={row.keep_warm}>
                        <span class="badge muted"><i />keep warm</span>
                      </Show>
                      <div class="muted small mono">{row.upstream_model || row.model_name}</div>
                    </td>
                    <td>
                      <span class="badge muted"><i />{sourceLabel(row.source)}</span>
                      <div class="muted small">{profileName(row.provider_profile_id)}</div>
                    </td>
                    <td>
                      <Show when={readinessBadge(readiness()[row.model_name])}
                            fallback={<span class="muted small">not checked yet</span>}>
                        {(badge) => <span class={`badge ${badge().tone}`}><i />{badge().text}</span>}
                      </Show>
                    </td>
                    <td class="actions">
                      <button class="tiny"
                              disabled={warmKey() === (row.upstream_model || row.model_name)}
                              title="Load the model and ask it one tiny question"
                              onClick={() => warm(row)}>
                        {warmKey() === (row.upstream_model || row.model_name) ? 'Warming…' : 'Warm up'}
                      </button>
                      <Show when={mayManage() && row.source === 'sandbox'}>
                        <button class="tiny danger" disabled={busy()} onClick={() => removeGrant(row)}>
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
      </Show>
    </section>
  );
}
