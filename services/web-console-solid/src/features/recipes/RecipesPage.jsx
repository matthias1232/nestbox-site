// Recipes: the templates an image is built from, and the sandboxes they start.
//
// The table answers "what exists and what came of the last build"; the drawer
// answers "what did I ask for" and holds the four actions. Writing a new recipe
// is not here: that form belongs to the builder, which validates every field
// against the baseline and the tool catalog before a row exists.

import { For, Show, onMount } from 'solid-js';
import StatusBadge from '../../components/StatusBadge.jsx';
import Empty from '../../components/Empty.jsx';
import RecipeDetail from './RecipeDetail.jsx';
import { useRecipes } from './useRecipes.js';
import '../../styles/recipes-persist.css';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

export default function RecipesPage() {
  const state = useRecipes();

  onMount(() => state.load());

  function confirmDelete(recipe) {
    const message = `Delete recipe "${recipe.name}"? The recipe and its revision history are removed. `
      + 'Sandboxes already created from it keep running, and their mounts keep their data.';
    if (window.confirm(message)) state.destroy(recipe);
  }

  return (
    <section class="page">
      <PageHeader
        title="Recipes"
        subtitle={<>{state.items().length} recipes · an image is addressed by the hash of the recipe that made it</>}
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
        Edit, build, and run a recipe here. Writing a new one is the builder's job: the same
        fields are validated against the baseline and the tool catalog when the row is created.
      </p>

      <Show when={!state.loading()} fallback={<p class="muted">Loading recipes…</p>}>
        <Show
          when={state.items().length > 0}
          fallback={
            <Empty
              title="No recipes yet"
              hint="A recipe is created through the builder or the API. This screen edits, builds, and runs one."
            />
          }
        >
          <div class="table-scroll">
            <table>
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Baseline</th>
                  <th class="narrow">Revision</th>
                  <th class="narrow">State</th>
                  <th>Image reference</th>
                  <th>Node</th>
                </tr>
              </thead>
              <tbody>
                <For each={state.items()}>
                  {(recipe) => (
                    <tr
                      class={state.selected()?.id === recipe.id ? 'selected' : ''}
                      onClick={() => state.open(recipe)}
                    >
                      <td class="mono">{recipe.name}</td>
                      <td class="mono small">{recipe.baseline_key}</td>
                      <td class="narrow mono">{recipe.revision}</td>
                      <td class="narrow"><StatusBadge status={recipe.state} /></td>
                      <td class="mono small" title={recipe.image_ref || 'not built'}>
                        <Show when={recipe.image_ref} fallback={<span class="muted">not built</span>}>
                          <span class="rp-ellipsis">{recipe.image_ref}</span>
                        </Show>
                      </td>
                      <td class="small">{state.nodeLabel(recipe.node_id)}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>

      <Show when={state.selected()}>
        <RecipeDetail
          recipe={state.selected()}
          state={state}
          onDelete={confirmDelete}
        />
      </Show>
    </section>
  );
}
