import { polyline } from './sparkline.js';

// A small trend line, no axes. Props: points (numbers), max, tone, width (120), height (28).
export default function Sparkline(props) {
  const w = () => props.width || 120;
  const h = () => props.height || 28;
  return (
    <svg class="sparkline" data-tone={props.tone} viewBox={`0 0 ${w()} ${h()}`} width={w()} height={h()} preserveAspectRatio="none" role="img" aria-label={props.label || 'Trend'}>
      <polyline points={polyline(props.points, w(), h(), props.max)} fill="none" stroke="currentColor" stroke-width="1.5" vector-effect="non-scaling-stroke" />
    </svg>
  );
}
