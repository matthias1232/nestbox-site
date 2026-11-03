// The bar that appears above the dock while rows are selected (contract C15, package F21).
// Props in, callbacks out: `{ count, kind, actions: [{ id, label, danger?, needs? }], onAction(id), onClear }`.
// The dialog for actions that need input lives in `BulkHost`; this component only shows the bar.
import { For, Show } from 'solid-js';
import '../styles/bulk.css';

export default function BulkBar(props) {
  const noun = () => (props.kind === 'node' ? 'node' : 'sandbox');
  return (
    <Show when={props.count > 0}>
      <div class="bulk-bar" role="toolbar" aria-label="Actions for the selection">
        <span class="bulk-count mono">
          {props.count} {props.count === 1 ? noun() : noun() === 'node' ? 'nodes' : 'sandboxes'} selected
        </span>
        <div class="bulk-actions">
          <For each={props.actions || []}>
            {(action) => (
              <button
                type="button"
                class={action.danger ? 'danger tiny' : 'secondary tiny'}
                onClick={() => props.onAction?.(action.id)}
              >
                {action.label}
              </button>
            )}
          </For>
        </div>
        <button type="button" class="ghost tiny" onClick={() => props.onClear?.()}>
          Clear
        </button>
      </div>
    </Show>
  );
}
