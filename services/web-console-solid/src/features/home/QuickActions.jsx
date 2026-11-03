// The things people do most, as buttons. Members only get "My sandboxes".
// Props: member (boolean), onNavigate(path).
import { Show } from 'solid-js';

export default function QuickActions(props) {
  return (
    <div class="home-quick">
      <Show when={!props.member}>
        <button type="button" class="primary" onClick={() => props.onNavigate('/sandboxes?new=1')}>New sandbox</button>
        <button type="button" class="secondary" onClick={() => props.onNavigate('/nodes?add=1')}>Add node</button>
        <button type="button" class="secondary" onClick={() => props.onNavigate('/models?deploy=1')}>Deploy model</button>
      </Show>
      <button type="button" class={props.member ? 'primary' : 'secondary'} onClick={() => props.onNavigate('/my')}>My sandboxes</button>
    </div>
  );
}
