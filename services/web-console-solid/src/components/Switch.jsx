// A two-state switch.
//
// The hidden checkbox is the real control, so the keyboard and the screen reader
// get native behaviour; the track and thumb are only how it looks.

import '../styles/controls.css';

export default function Switch(props) {
  return (
    <label class="nb-switch" title={props.title}>
      <input
        type="checkbox"
        checked={props.checked ?? false}
        disabled={props.disabled}
        onChange={(event) => props.onChange?.(event.currentTarget.checked)}
      />
      <span class="nb-switch-track">
        <span class="nb-switch-thumb" />
      </span>
      <span class="nb-switch-label">{props.label}</span>
    </label>
  );
}
