// The trail at the top of every page: `items` is [{ label, path? }]; the last one is the page itself.
import { For, Show } from 'solid-js';
import { navigate } from '../../routing.js';

export default function Breadcrumb(props) {
  return (
    <nav class="cc-breadcrumb" aria-label="Breadcrumb">
      <For each={props.items || []}>
        {(item, index) => (
          <>
            <Show when={index() > 0}><span class="cc-crumb-sep" aria-hidden="true">/</span></Show>
            <Show
              when={item.path && index() < (props.items || []).length - 1}
              fallback={<span aria-current={index() === (props.items || []).length - 1 ? 'page' : undefined}>{item.label}</span>}
            >
              <a href={item.path} onClick={(event) => { event.preventDefault(); navigate(item.path); }}>{item.label}</a>
            </Show>
          </>
        )}
      </For>
    </nav>
  );
}
