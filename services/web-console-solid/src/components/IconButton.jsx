import '../styles/primitives.css';
import { Icon } from './icons.jsx';

// A 28x28 square icon-only button; `label` is both the tooltip and the accessible name.
// Props: icon (name or component), label, onClick, tone ('danger'), disabled, active.
export default function IconButton(props) {
  return (
    <button
      type="button"
      class="icon-button"
      classList={{ danger: props.tone === 'danger', active: !!props.active }}
      aria-label={props.label}
      title={props.label}
      aria-pressed={props.active === undefined ? undefined : !!props.active}
      disabled={props.disabled}
      onClick={(event) => props.onClick && props.onClick(event)}
    >
      {typeof props.icon === 'function' ? props.icon({}) : <Icon name={props.icon} />}
    </button>
  );
}
