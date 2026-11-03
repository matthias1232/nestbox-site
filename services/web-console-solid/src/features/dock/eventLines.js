// Turns one live event into one ticker line (pure; the dock renders the result).

// One line for the event `name` with its payload: { tone: 'ok' | 'bad' | 'warn' | 'info', text, path }.
export function lineFor(name, payload) {
  const p = payload && typeof payload === 'object' ? payload : {};
  switch (name) {
    case 'alert':
      return {
        tone: 'bad',
        text: `Refused ${p.host || 'destination'}${p.port ? `:${p.port}` : ''}${p.sandbox_name ? ` from ${p.sandbox_name}` : ''}`,
        path: p.sandbox_id ? `/sandboxes/${p.sandbox_id}` : null,
      };
    case 'sandbox': {
      const bad = ['failed', 'error', 'unhealthy'].includes(p.status);
      return {
        tone: bad ? 'bad' : p.status === 'running' ? 'ok' : 'info',
        text: `Sandbox ${p.name || short(p.sandbox_id)} is ${p.status || 'changed'}`,
        path: p.sandbox_id ? `/sandboxes/${p.sandbox_id}` : null,
      };
    }
    case 'node': {
      const down = p.state === 'down' || p.state === 'offline';
      return {
        tone: down ? 'bad' : 'info',
        text: `Node ${p.name || short(p.node_id)} tunnel ${p.state || 'changed'}`,
        path: p.node_id ? `/node/${p.node_id}` : null,
      };
    }
    case 'job': {
      const failed = p.status === 'failed' || p.status === 'partial';
      return {
        tone: failed ? 'bad' : p.status === 'succeeded' ? 'ok' : 'info',
        text: `Job ${p.kind || short(p.job_id)} ${p.status || 'progress'} ${Number(p.completed) || 0}/${Number(p.total) || 0}`,
        path: p.job_id ? `/operations?id=${p.job_id}` : '/operations',
      };
    }
    case 'control':
      return {
        tone: 'info',
        text: `Sandbox setting changed${p.what ? `: ${p.what}` : ''}`,
        path: p.sandbox_id ? `/sandboxes/${p.sandbox_id}` : null,
      };
    case 'models':
      return { tone: 'info', text: 'Models changed', path: '/models' };
    case 'policy_rollout':
      return {
        tone: 'info',
        text: `Policy rollout ${Number(p.applied) || 0}/${Number(p.total) || 0}`,
        path: '/policies',
      };
    default:
      return { tone: 'info', text: `Event ${name}`, path: null };
  }
}

// The first eight characters of an id, or a dash.
function short(id) {
  return id ? String(id).slice(0, 8) : '—';
}

// Add a line at the top and keep at most `max` lines.
export function pushLine(lines, line, max = 200) {
  return [line, ...lines].slice(0, max);
}

// Lines of one tone, or all when `tone` is 'all'.
export function filterTone(lines, tone) {
  return tone === 'all' ? lines : lines.filter((line) => line.tone === tone);
}
