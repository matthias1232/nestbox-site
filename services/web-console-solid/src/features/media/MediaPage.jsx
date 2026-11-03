// Media rendering: the environment default, the allow-list, and one sandbox's
// own choice.
//
// The screen decides what the `render_image`, `render_video` and `render_music`
// tools may use. It does not render anything itself: rendering is an agent tool,
// and a prompt belongs in the conversation, not in an operator form.

import { For, Show, createEffect, createSignal, onMount } from 'solid-js';
import { useMedia } from './useMedia.js';
import * as sandboxApi from '../../api/sandboxes.js';
import StatusBadge from '../../components/StatusBadge.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

function emptyTarget() {
  return { provider_id: '', model: '' };
}

function targetFrom(value) {
  return value ? { provider_id: value.provider_id, model: value.model } : emptyTarget();
}

function toTarget(value) {
  if (!value?.provider_id || !value?.model?.trim()) return null;
  return { provider_id: value.provider_id, model: value.model.trim() };
}

export default function MediaPage(props) {
  const media = useMedia();
  const [sandboxes, setSandboxes] = createSignal([]);
  const [selectedSandbox, setSelectedSandbox] = createSignal('');
  const [draft, setDraft] = createSignal({ enabled: true, image: emptyTarget(), video: emptyTarget(), music: emptyTarget(), allowed_provider_ids: [] });
  const [sandboxDraft, setSandboxDraft] = createSignal({ image: emptyTarget(), video: emptyTarget(), music: emptyTarget() });
  const [userDrafts, setUserDrafts] = createSignal({});
  const [purgeDays, setPurgeDays] = createSignal(30);
  const isAdmin = () => props.role === 'admin';

  onMount(async () => {
    await media.load();
    if (props.role === 'admin') await media.loadUsers();
    try {
      setSandboxes((await sandboxApi.list({ limit: 200 })).items || []);
    } catch {
      // The list is a convenience for the override form; the settings above
      // still work without it, so a failure here is not a page error.
    }
  });

  createEffect(() => {
    const view = media.view();
    if (!view) return;
    setDraft({
      enabled: view.settings.enabled,
      image: targetFrom(view.settings.image),
      video: targetFrom(view.settings.video),
      music: targetFrom(view.settings.music),
      allowed_provider_ids: view.settings.allowed_provider_ids || [],
    });
  });

  createEffect(() => {
    const view = media.sandboxView();
    if (!view) {
      setSandboxDraft({ image: emptyTarget(), video: emptyTarget(), music: emptyTarget() });
      return;
    }
    setSandboxDraft({
      image: targetFrom(view.override_settings.image),
      video: targetFrom(view.override_settings.video),
      music: targetFrom(view.override_settings.music),
    });
  });

  createEffect(() => {
    const list = media.users();
    if (!list.length) return;
    const drafts = {};
    for (const user of list) {
      drafts[user.user_id] = {
        provider_ids: [...(user.grant.provider_ids || [])],
        can_configure: user.grant.can_configure,
      };
    }
    setUserDrafts(drafts);
  });

  const providers = () => media.view()?.providers || [];
  const providerName = (id) => providers().find((provider) => provider.id === id)?.name || '—';
  const allowed = (provider) => isAdmin() || provider.allowed;

  const setKind = (kind, patch) => setDraft({ ...draft(), [kind]: { ...draft()[kind], ...patch } });
  const setSandboxKind = (kind, patch) =>
    setSandboxDraft({ ...sandboxDraft(), [kind]: { ...sandboxDraft()[kind], ...patch } });

  async function chooseSandbox(id) {
    setSelectedSandbox(id);
    await media.loadSandbox(id);
  }

  return (
    <div class="page">
      <PageHeader
        title="Media rendering"
        subtitle={<>Which model answers an image, video or music request from an agent. The sandbox never receives the provider credential: the control plane calls the provider and stores the asset.</>}
        actions={<>
          <HelpTip anchor="getting-started" />
        </>}
      />

      <Show when={media.error()}><p class="error">{media.error()}</p></Show>
      <Show when={media.notice()}><p class="notice">{media.notice()}</p></Show>

      <Show when={media.view()} fallback={<p class="muted">Loading media settings…</p>}>
        <section class="editor">
          <h3>Environment default</h3>
          <p class="muted small">
            Every sandbox inherits these unless it chooses its own. A kind without a model is
            unconfigured, and a request for it fails with a message that says so.
          </p>
          <label class="field">
            <input
              type="checkbox"
              checked={draft().enabled}
              disabled={!isAdmin()}
              onChange={(event) => setDraft({ ...draft(), enabled: event.currentTarget.checked })}
            />
            Media rendering enabled for this environment
          </label>

          <For each={media.KINDS}>
            {(kind) => (
              <div class="form-grid">
                <label class="field">
                  {kind.label} provider
                  <select
                    class="input"
                    disabled={!isAdmin()}
                    value={draft()[kind.id].provider_id}
                    onChange={(event) => setKind(kind.id, { provider_id: event.currentTarget.value })}
                  >
                    <option value="">— none —</option>
                    <For each={providers().filter(allowed)}>
                      {(provider) => <option value={provider.id}>{provider.name} ({provider.kind})</option>}
                    </For>
                  </select>
                </label>
                <label class="field">
                  {kind.label} model
                  <input
                    class="input"
                    placeholder="the model id the provider expects"
                    value={draft()[kind.id].model}
                    disabled={!isAdmin()}
                    onInput={(event) => setKind(kind.id, { model: event.currentTarget.value })}
                  />
                </label>
                <span class="mono small">{kind.tool}</span>
              </div>
            )}
          </For>

          <h4>Providers a sandbox owner may choose</h4>
          <p class="muted small">
            Nothing ticked means every provider in this environment. Ticking any provider turns
            the list into an allow-list: a sandbox may then choose only from it.
          </p>
          <div class="form-grid">
            <For each={providers()}>
              {(provider) => (
                <label class="field rp-check">
                  <input
                    type="checkbox"
                    disabled={!isAdmin()}
                    checked={draft().allowed_provider_ids.includes(provider.id)}
                    onChange={(event) =>
                      setDraft({
                        ...draft(),
                        allowed_provider_ids: event.currentTarget.checked
                          ? [...draft().allowed_provider_ids, provider.id]
                          : draft().allowed_provider_ids.filter((id) => id !== provider.id),
                      })
                    }
                  />
                  {provider.name} ({provider.kind})
                </label>
              )}
            </For>
          </div>
          <Show when={isAdmin()}>
            <button
              class="primary"
              disabled={media.busy()}
              onClick={() =>
                media.save({
                  enabled: draft().enabled,
                  image: toTarget(draft().image),
                  video: toTarget(draft().video),
                  music: toTarget(draft().music),
                  allowed_provider_ids: draft().allowed_provider_ids,
                })
              }
            >
              Save environment default
            </button>
          </Show>
          <Show when={!isAdmin()}>
            <p class="muted small">Only an admin can change the environment default and its allow-list.</p>
          </Show>
        </section>

        <Show when={isAdmin()}>
          <section class="editor">
            <h3>User releases</h3>
            <p class="muted small">
              Which providers a person may see — and whether they may change their own sandbox's
              choice. No tick means every provider the environment allows; “may choose” unticked
              lets them see the models but not switch them. Admins are never restricted.
            </p>
            <div class="table-scroll">
              <table class="table">
                <thead>
                  <tr>
                    <th>Person</th>
                    <th>Providers</th>
                    <th>May choose</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  <For each={media.users()}>
                    {(user) => (
                      <tr>
                        <td>
                          <div>{user.display_name || user.email}</div>
                          <div class="muted small">{user.email}{user.explicit ? ' · released' : ' · inherits'}</div>
                        </td>
                        <td>
                          <For each={providers()}>
                            {(provider) => (
                              <label class="field rp-check">
                                <input
                                  type="checkbox"
                                  checked={(userDrafts()[user.user_id]?.provider_ids || []).includes(provider.id)}
                                  onChange={(event) =>
                                    setUserDrafts({
                                      ...userDrafts(),
                                      [user.user_id]: {
                                        ...userDrafts()[user.user_id],
                                        provider_ids: event.currentTarget.checked
                                          ? [...(userDrafts()[user.user_id]?.provider_ids || []), provider.id]
                                          : (userDrafts()[user.user_id]?.provider_ids || []).filter((id) => id !== provider.id),
                                      },
                                    })
                                  }
                                />
                                {provider.name}
                              </label>
                            )}
                          </For>
                        </td>
                        <td>
                          <label class="field rp-check">
                            <input
                              type="checkbox"
                              checked={userDrafts()[user.user_id]?.can_configure ?? true}
                              onChange={(event) =>
                                setUserDrafts({
                                  ...userDrafts(),
                                  [user.user_id]: {
                                    ...userDrafts()[user.user_id],
                                    can_configure: event.currentTarget.checked,
                                  },
                                })
                              }
                            />
                          </label>
                        </td>
                        <td>
                          <button
                            class="tiny"
                            disabled={media.busy()}
                            onClick={() =>
                              media.saveUserGrant(user.user_id, {
                                provider_ids: userDrafts()[user.user_id]?.provider_ids || [],
                                can_configure: userDrafts()[user.user_id]?.can_configure ?? true,
                              })
                            }
                          >
                            Save
                          </button>
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </section>
        </Show>

        <section class="editor">
          <h3>Sandbox choice</h3>
          <label class="field">
            Sandbox
            <select class="input" value={selectedSandbox()} onChange={(event) => chooseSandbox(event.currentTarget.value)}>
              <option value="">— choose a sandbox —</option>
              <For each={sandboxes()}>{(sandbox) => <option value={sandbox.id}>{sandbox.name}</option>}</For>
            </select>
          </label>

          <Show when={media.sandboxView()} fallback={<p class="muted small">Choose a sandbox to see what it inherits.</p>}>
            <For each={media.KINDS}>
              {(kind) => (
                <div class="form-grid">
                  <label class="field">
                    {kind.label}
                    <select
                      class="input"
                      value={sandboxDraft()[kind.id].provider_id}
                      onChange={(event) => {
                        setSandboxKind(kind.id, { provider_id: event.currentTarget.value });
                      }}
                    >
                      <option value="">
                        inherit ({providerName(media.sandboxView().effective[kind.id]?.provider_id)})
                      </option>
                      <For each={providers().filter((provider) => provider.allowed)}>
                        {(provider) => (
                          <option value={provider.id}>{provider.name}</option>
                        )}
                      </For>
                    </select>
                  </label>
                  <label class="field">
                    {kind.label} model
                    <input
                      class="input"
                      value={sandboxDraft()[kind.id].model}
                      placeholder={media.sandboxView().effective[kind.id]?.model || 'inherited'}
                      onInput={(event) => setSandboxKind(kind.id, { model: event.currentTarget.value })}
                    />
                  </label>
                  <StatusBadge
                    value={
                      (sandboxDraft()[kind.id].provider_id ? 'own choice' : 'inherited') +
                      (media.sandboxView().effective[kind.id]
                        ? ` · ${media.sandboxView().effective[kind.id].model}`
                        : ' · unconfigured')
                    }
                  />
                </div>
              )}
            </For>
            <button
              class="primary"
              disabled={media.busy()}
              onClick={() =>
                media.saveSandbox(selectedSandbox(), {
                  image: toTarget(sandboxDraft().image),
                  video: toTarget(sandboxDraft().video),
                  music: toTarget(sandboxDraft().music),
                })
              }
            >
              Save sandbox choice
            </button>

            <h4>Recent renders</h4>
            <Show when={isAdmin()}>
              <div class="form-grid">
                <label class="field">
                  Purge renders older than (days)
                  <input
                    class="input"
                    type="number"
                    min="1"
                    max="3650"
                    value={purgeDays()}
                    onInput={(event) => setPurgeDays(Number(event.currentTarget.value))}
                  />
                </label>
                <button class="tiny danger" disabled={media.busy()} onClick={() => media.purge(selectedSandbox(), purgeDays())}>
                  Purge
                </button>
              </div>
              <p class="muted small">Removes the render rows and their files. The audit entry records how many went.</p>
            </Show>
            <Show when={media.renders().length} fallback={<p class="muted small">This sandbox has not rendered anything yet.</p>}>
              <div class="table-scroll">
                <table class="table">
                  <thead>
                    <tr>
                      <th>When</th>
                      <th>Kind</th>
                      <th>Model</th>
                      <th>Status</th>
                      <th>Prompt</th>
                      <th>Asset</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={media.renders()}>
                      {(render) => (
                        <tr>
                          <td class="small">{new Date(render.created_at).toLocaleString()}</td>
                          <td>{render.kind}</td>
                          <td class="mono small">{render.model}</td>
                          <td><StatusBadge value={render.status} /></td>
                          <td class="small">{render.prompt.slice(0, 120)}</td>
                          <td>
                            <Show when={render.asset_url} fallback={<span class="muted small">{render.error || '—'}</span>}>
                              <a href={render.asset_url} target="_blank" rel="noreferrer">open</a>
                            </Show>
                          </td>
                          <td>
                            <Show when={isAdmin()}>
                              <button
                                class="tiny danger"
                                disabled={media.busy()}
                                onClick={() => media.removeRender(selectedSandbox(), render.id)}
                              >
                                delete
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
          </Show>
        </section>
      </Show>
    </div>
  );
}
