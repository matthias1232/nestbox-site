// One mounted root: slug, kind, owner, node, path input, RO/RW chip, size, Remove.
// Props: row, error, onPath(path), onFlip(), onRemove().
import { Show } from 'solid-js';

function size(bytes) {
  if (bytes == null) return '';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i += 1; }
  return `${value.toFixed(i ? 1 : 0)} ${units[i]}`;
}

export default function MountRow(props) {
  const row = () => props.row;
  return (
    <tr class="mount-row">
      <td><b>{row().slug}</b> <span class="badge muted"><i />{row().kind}</span></td>
      <td class="small">{row().owner || '—'}</td>
      <td class="small">{row().node || '—'}</td>
      <td>
        <input
          class="mono mount-path"
          aria-label={`Mount path of ${row().slug}`}
          aria-invalid={props.error ? 'true' : 'false'}
          value={row().mount_path}
          onInput={(e) => props.onPath(e.currentTarget.value)}
        />
        <Show when={props.error}><div class="small mount-error">{props.error}</div></Show>
      </td>
      <td>
        <button
          type="button"
          class="mount-chip"
          classList={{ rw: !row().read_only }}
          aria-pressed={!row().read_only}
          aria-label={`${row().slug}: ${row().read_only ? 'read-only' : 'read-write'}, click to switch`}
          onClick={() => props.onFlip()}
        >
          {row().read_only ? 'RO' : 'RW'}
        </button>
      </td>
      <td class="mono small">{size(row().bytes_used)}</td>
      <td>
        <Show when={row().state === 'pending_recreate'}><span class="badge warn"><i />after recreate</span> </Show>
        <button type="button" class="tiny" aria-label={`Remove ${row().slug}`} onClick={() => props.onRemove()}>Remove</button>
      </td>
    </tr>
  );
}
