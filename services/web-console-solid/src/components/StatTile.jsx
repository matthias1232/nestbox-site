import { Show } from 'solid-js';

// A figure with an eyebrow label, an optional unit and trend line; a button when `onClick` is set.
// Props: label, value, unit, trend, tone ('ok' | 'warn' | 'bad' | 'info'), onClick.
export default function StatTile(props) {
  const body = () => (
    <>
      <span class="eyebrow">{props.label}</span>
      <span class="figure">
        {props.value ?? '–'}
        <Show when={props.unit}>
          <small>{props.unit}</small>
        </Show>
      </span>
      <Show when={props.trend}>
        <span class="muted small">{props.trend}</span>
      </Show>
    </>
  );
  return (
    <Show
      when={props.onClick}
      fallback={<div class="stat-tile" data-tone={props.tone}>{body()}</div>}
    >
      <button type="button" class="stat-tile clickable" data-tone={props.tone} onClick={(event) => props.onClick(event)}>
        {body()}
      </button>
    </Show>
  );
}
