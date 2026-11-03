// A row of mutually exclusive buttons.
//
// It is a radio group, not a set of toggle buttons: exactly one option carries the
// active look, so "which mode am I in" is answered by the colour alone. The group takes
// its accessible name from `labelledby` (the id of a visible label beside it); it never
// sits inside a <label>, because that label would forward a click on any chip to the
// group's first button (U06-03).

import { For } from 'solid-js';

import '../styles/controls.css';

export default function Segmented(props) {
  return (
    <div class="nb-segmented" role="radiogroup" aria-labelledby={props.labelledby} aria-label={props.label}>
      <For each={props.options || []}>
        {(option) => {
          const active = () => props.value === option.value;
          return (
            <button
              type="button"
              role="radio"
              aria-checked={active()}
              title={option.title}
              class={'nb-seg' + (active() ? ' active' : '') + (option.tone ? ' tone-' + option.tone : '')}
              disabled={props.disabled}
              onClick={() => props.onChange?.(option.value)}
            >
              {option.label}
            </button>
          );
        }}
      </For>
    </div>
  );
}
