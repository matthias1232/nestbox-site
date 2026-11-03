// A status label with a colour that means the same thing everywhere.
//
// The colour is derived from the status string, so a new status that nobody
// classified shows as neutral instead of accidentally looking healthy.

const TONES = {
  running: 'ok', healthy: 'ok', succeeded: 'ok', valid: 'ok', installed: 'ok',
  built: 'ok',
  starting: 'warn', downloading: 'warn', provisioning: 'warn', degraded: 'warn', pending: 'warn',
  updating: 'warn', expiring: 'warn', partial: 'warn', building: 'warn',
  stopped: 'muted', unknown: 'muted', cancelled: 'muted', not_installed: 'muted',
  draft: 'muted',
  failed: 'bad', unhealthy: 'bad', offline: 'bad', expired: 'bad', denied: 'bad',
  denied_by_policy: 'bad',
  // A provider answers or it does not. `pending` stays amber on purpose: an
  // install created a profile whose server is not started yet, which is not the
  // same as an endpoint that failed (ADR 0018).
  reachable: 'ok', unreachable: 'bad',
  // A caller may already hold a tone (the health rows of the home page do, X13); naming a
  // tone as the status keeps such a row honest instead of printing the word "bad".
  ok: 'ok', warn: 'warn', bad: 'bad', info: 'info', muted: 'muted',
};

export default function StatusBadge(props) {
  const tone = () => TONES[props.status] || 'muted';
  return (
    <span class={`badge ${tone()}`} title={props.title || props.status}>
      <i />{props.label || props.status}
    </span>
  );
}
