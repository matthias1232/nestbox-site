// The sticky title row of a page: title, status badge, subtitle and the action bar.
// The action bar stays visible when the page scrolls; actions never hide inside a tab.
import { Show } from 'solid-js';

export default function PageHeader(props) {
  return (
    <header class="cc-page-header">
      <div class="cc-page-title">
        <h1>{props.title}</h1>
        <Show when={props.status}>{props.status}</Show>
        <Show when={props.subtitle}><p class="muted small">{props.subtitle}</p></Show>
      </div>
      <div class="cc-page-actions" data-tour="actions">{props.actions}</div>
    </header>
  );
}
