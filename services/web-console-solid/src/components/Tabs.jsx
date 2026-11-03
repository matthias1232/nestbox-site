// A tab strip.
//
// The active tab is controlled by the parent, because two tabs that both think
// they are active is a bug that is hard to find later. A tab may carry `count`:
// it is drawn after the label in muted mono, not as a badge (V08).
import { Show } from 'solid-js';
import '../styles/primitives.css';

export default function Tabs(props) {
  return (
    <div class="tabs" role="tablist">
      {props.tabs.map((tab) => (
        <button
          role="tab"
          aria-selected={props.active === tab.id}
          class={props.active === tab.id ? 'tab active' : 'tab'}
          onClick={() => props.onChange(tab.id)}
        >
          {tab.label}
          <Show when={typeof tab.count === 'number'}>
            {' '}<span class="mono muted tab-count">{tab.count}</span>
          </Show>
        </button>
      ))}
    </div>
  );
}
