// A list of things that need attention, each row a link with a status badge and one line why.
// Props: rows [{ kind, id, name, status, why }], onOpen(row).
import { For, Show } from 'solid-js';
import StatusBadge from '../../components/StatusBadge.jsx';

export default function HealthList(props) {
  return (
    <Show when={(props.rows || []).length > 0} fallback={<p class="muted small">Everything is healthy.</p>}>
      <ul class="home-health">
        <For each={(props.rows || []).slice(0, 12)}>
          {(row) => (
            <li>
              <a href="#" onClick={(event) => { event.preventDefault(); props.onOpen?.(row); }}>{row.name}</a>
              <StatusBadge status={row.status} />
              <span class="muted small">{row.why}</span>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}
