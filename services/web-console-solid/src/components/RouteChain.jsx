// The way one request took, or would have taken: sandbox, proxy, check, upstream,
// router, model.
import { For, Show } from 'solid-js';
import '../styles/dlp.css';

const STATE_PREFIX = { would: 'would: ' };

/**
 * Draw a route as pills joined by arrows.
 *
 * `state` is `passed` (normal), `blocked` (stopped here, red and crossed) or
 * `would` (dashed and grey, for a path a policy would have allowed). `detail`
 * becomes the tooltip, and `compact` shrinks the chain for a chat bubble.
 */
export default function RouteChain(props) {
  const hops = () => props.route || [];
  return (
    <Show when={hops().length}>
      <span class={`route-chain ${props.compact ? 'compact' : ''}`}>
        <For each={hops()}>
          {(hop, index) => (
            <>
              <Show when={index() > 0}>
                <span class="route-arrow" aria-hidden="true">
                  →
                </span>
              </Show>
              <span
                class={`route-pill ${hop.state || 'passed'}`}
                title={hop.detail || hop.label || ''}
                data-state={hop.state || 'passed'}
              >
                {STATE_PREFIX[hop.state] || ''}
                {hop.label}
                <Show when={hop.state === 'blocked'}>
                  <span class="route-x" aria-label="blocked">
                    {' '}
                    ✕
                  </span>
                </Show>
              </span>
            </>
          )}
        </For>
      </span>
    </Show>
  );
}
