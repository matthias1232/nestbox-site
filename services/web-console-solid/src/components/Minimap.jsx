// The whole topology drawing scaled to 160 px, with the visible area as a rectangle; a click moves the view.
import { For } from 'solid-js';

export default function Minimap(props) {
  const WIDTH = 160;
  const scale = () => WIDTH / Math.max(props.width, 1);
  const height = () => Math.max(24, Math.min(120, props.height * scale()));

  function move(event) {
    const box = event.currentTarget.getBoundingClientRect();
    const x = ((event.clientX - box.left) / box.width) * props.width;
    const y = ((event.clientY - box.top) / box.height) * props.height;
    props.onMove?.(x, y);
  }

  return (
    <svg
      class="topo-minimap"
      width={WIDTH}
      height={height()}
      viewBox={`0 0 ${props.width} ${props.height}`}
      preserveAspectRatio="none"
      role="img"
      aria-label="Minimap of the topology; click to move the view"
      onClick={move}
    >
      <For each={props.cards}>
        {(card) => <rect class={`mini-card ${card.tone}`} x={card.x} y={card.y} width={props.card.w} height={props.card.h} />}
      </For>
      <rect class="mini-view" x={props.view.x} y={props.view.y} width={props.view.w} height={props.view.h} />
    </svg>
  );
}
