import { arcPath, fraction, isHot } from './gauge.js';

// A 270 degree arc for CPU, RAM, VRAM or disk: lime fill, red from 90 %.
// Props: label, value, max, unit, size (px, default 64).
export default function Gauge(props) {
  const r = 24;
  const percent = () => Math.round(fraction(props.value, props.max) * 100);
  return (
    <figure
      class="gauge"
      role="img"
      aria-label={`${props.label}: ${props.value ?? 0} of ${props.max ?? 0} ${props.unit || ''}, ${percent()} percent`}
    >
      <svg viewBox="0 0 64 64" width={props.size || 64} height={props.size || 64} aria-hidden="true">
        <path d={arcPath(1, 1, r, 32, 32)} class="gauge-track" />
        <path d={arcPath(props.value, props.max, r, 32, 32)} class="gauge-fill" classList={{ hot: isHot(props.value, props.max) }} />
        <text x="32" y="36" text-anchor="middle" class="gauge-percent">{percent()}%</text>
      </svg>
      <figcaption>
        <span class="eyebrow">{props.label}</span>
        <span class="mono small">{props.value ?? 0}/{props.max ?? 0} {props.unit}</span>
      </figcaption>
    </figure>
  );
}
