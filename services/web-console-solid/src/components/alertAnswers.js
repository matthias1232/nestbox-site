// The answers offered for one alert and the request bodies they send (contract C9).

// The buttons of an alert card, in display order.
export function answers(alert) {
  return [
    { id: 'now', label: 'Allow now' },
    { id: '15', label: '15 min' },
    { id: '60', label: '1 h' },
    { id: 'draft', label: 'Draft rule' },
    { id: 'ignore', label: 'Ignore' },
    { id: 'open', label: alert?.sandbox_name ? `Open ${alert.sandbox_name}` : 'Open sandbox' },
  ];
}

// Body of the permanent one-step allow: POST /sandboxes/{id}/allow.
export function allowBody(alert) {
  return { host: alert.host, port: alert.port, protocol: alert.protocol };
}

// Arguments of a bounded allow: a bypass window on the alert's channel.
export function windowBody(alert, minutes) {
  const channel = ['proxy', 'dns', 'mcp'].includes(alert.channel) ? alert.channel : 'all';
  return {
    sandbox_id: alert.sandbox_id,
    minutes: Number(minutes),
    unlimited: false,
    mode: 'bypass',
    channel,
    reason: `alert ${alert.host}`,
  };
}

// The network event a rule draft is made from.
export function draftEvent(alert) {
  return {
    host: alert.host,
    port: alert.port,
    protocol: alert.protocol,
    channel: alert.channel,
    decision: alert.decision,
    reason: alert.reason,
    count: alert.count,
    first_at: alert.first_at,
    last_at: alert.last_at,
  };
}

// "3 min ago" style age of an ISO time, relative to `now` (ms).
export function ageOf(iso, now = Date.now()) {
  const then = Date.parse(iso || '');
  if (Number.isNaN(then)) return '';
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 60) return `${seconds} s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
  return `${Math.floor(seconds / 3600)} h ago`;
}
