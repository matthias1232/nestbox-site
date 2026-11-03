// Persistent roots: the data that outlives a container.
//
// The table answers "what exists, where does it live, and what is it mounted in".
// Creating a root asks for a kind, a node, and a slug and nothing else: the host
// path is derived by the server, so a path field would be a way to ask for
// arbitrary host access.

import { For, Show, onMount } from 'solid-js';
import Empty from '../../components/Empty.jsx';
import PersistentDetail from './PersistentDetail.jsx';
import MountsMatrix from './MountsMatrix.jsx';
import { usePersistent, usageLabel } from './usePersistent.js';
import '../../styles/recipes-persist.css';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

export default function PersistentPage() {
  const state = usePersistent();

  onMount(() => state.load());

  function confirmDismount(root, mounted) {
    const message = `Dismount root "${root.slug}" from sandbox "${mounted.sandbox_name}"? `
      + `The mount at ${mounted.mount_path} is removed; the files stay in ${root.host_path}.`;
    if (window.confirm(message)) state.dismount(root, mounted);
  }

  function confirmArchive(root) {
    const message = `Archive root "${root.slug}"? It disappears from this list. `
      + `Its directory ${root.host_path} and its data are unchanged.`;
    if (window.confirm(message)) state.archive(root);
  }

  function confirmPurge(root) {
    const message = `Delete the record of root "${root.slug}"? The row is removed and cannot be `
      + `restored from the console. The directory ${root.host_path} stays on the node: the `
      + 'control plane does not delete files it cannot see.';
    if (window.confirm(message)) state.purge(root);
  }

  return (
    <section class="page">
      <PageHeader
        title="Persistent data"
        subtitle={<>{state.items().length} roots · a root is a directory on a node; a mount binds it into a sandbox</>}
        actions={<>
          <button class="secondary" onClick={() => state.load()} disabled={state.loading()}>
            {state.loading() ? 'Refreshing…' : 'Refresh'}
          </button>
          <HelpTip anchor="getting-started" />
        </>}
      />

      <Show when={!state.selected()}>
        <Show when={state.error()}><p class="error">{state.error()}</p></Show>
        <Show when={state.notice()}><p class="notice">{state.notice()}</p></Show>
      </Show>

      <p class="muted small rp-page-note">
        Data never lives in an image: rebuilding a sandbox keeps its roots, and a new sandbox from
        the same recipe sees an empty root until it is mounted. Archived roots are hidden and
        there is no restore in this phase.
      </p>

      <Show when={!state.loading()} fallback={<p class="muted">Loading roots…</p>}>
        <Show
          when={state.items().length > 0}
          fallback={
            <Empty
              title="No persistent roots yet"
              hint="Create one below, then mount it into a sandbox on the same node."
            />
          }
        >
          <div class="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Kind</th>
                  <th>Node</th>
                  <th>Slug</th>
                  <th>Host path</th>
                  <th>Used</th>
                  <th>Mounted in</th>
                </tr>
              </thead>
              <tbody>
                <For each={state.items()}>
                  {(root) => (
                    <tr
                      class={state.selected()?.id === root.id ? 'selected' : ''}
                      onClick={() => state.open(root)}
                    >
                      <td class="small">{state.kindLabel(root.kind)}</td>
                      <td class="small">{state.nodeLabel(root.node_id)}</td>
                      <td class="mono">{root.slug}</td>
                      <td class="mono small" title={root.host_path}>
                        <span class="rp-ellipsis">{root.host_path}</span>
                      </td>
                      <td class="narrow" title={root.bytes_used == null ? 'never measured' : `${root.bytes_used} bytes`}>
                        <Show when={root.measured_at} fallback={<span class="muted small">not measured</span>}>
                          {usageLabel(root)}
                        </Show>
                      </td>
                      <td class="small">
                        <Show
                          when={root.mounted_in?.length > 0}
                          fallback={<span class="muted">not mounted</span>}
                        >
                          <ul class="rp-mount-list">
                            <For each={root.mounted_in}>
                              {(mounted) => (
                                <li>
                                  <span>{mounted.sandbox_name}</span>
                                  <span class="mono small"> {mounted.mount_path}</span>
                                  <span class={mounted.read_only ? 'ro' : 'rw'}>
                                    {mounted.read_only ? ' ro' : ' rw'}
                                  </span>
                                </li>
                              )}
                            </For>
                          </ul>
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

      <Show when={!state.loading() && state.items().length > 0}>
        <MountsMatrix roots={state.items()} />
      </Show>

      <form class="editor rp-create" onSubmit={state.create}>
        <h3>Create a root</h3>
        <p class="muted small">
          The host path is derived by the server from the kind, the owner, and the slug, so this
          form has no path field. A request that names a host path is arbitrary host access, and
          the API does not offer one.
        </p>
        <div class="form-grid">
          <label class="field">
            <span>Kind</span>
            <select
              value={state.form().kind}
              onChange={(event) => state.updateForm({ kind: event.currentTarget.value })}
            >
              <option value="project">project</option>
              <option value="agent_home">agent home</option>
            </select>
          </label>
          <label class="field">
            <span>Node</span>
            <select
              value={state.form().node}
              onChange={(event) => state.updateForm({ node: event.currentTarget.value })}
            >
              <option value="">local node</option>
              <For each={state.nodes()}>
                {(node) => <option value={node.name}>{node.display_name || node.name}</option>}
              </For>
            </select>
          </label>
          <label class="field">
            <span>Slug</span>
            <input
              value={state.form().slug}
              required
              placeholder="project-alpha"
              onInput={(event) => state.updateForm({ slug: event.currentTarget.value })}
            />
          </label>
        </div>
        <p class="muted small">
          Slugs are lowercase letters, digits, and hyphens, 1 to 63 characters; the slug becomes
          part of the directory name and cannot be changed later.
        </p>
        <div class="actions">
          <button type="submit" disabled={state.busy()}>
            {state.busy() ? 'Creating…' : 'Create root'}
          </button>
        </div>
      </form>

      <Show when={state.selected()}>
        <PersistentDetail
          root={state.selected()}
          state={state}
          onDismount={confirmDismount}
          onArchive={confirmArchive}
          onPurge={confirmPurge}
        />
      </Show>
    </section>
  );
}
