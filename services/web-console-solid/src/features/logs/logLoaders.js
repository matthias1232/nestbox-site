// Pure helpers that turn a log "source" into a loader returning merged log text.
//
// A source is a plain string so it can be saved with the pane layout:
//   'all'            every running sandbox the caller may see, merged by time
//   'sandbox:<id>'   one sandbox
//   'node:<id>'      the sandboxes of one node, merged by time
//   'system:<name>'  a platform component (admins)
import * as sandboxApi from '../../api/sandboxes.js';
import * as systemApi from '../../api/system.js';
import * as nodeApi from '../../api/nodes.js';

const STAMPED = /^(\S+Z)\s+(?:\[([^\]]+)\]\s+)?(.*)$/;
const ISO_START = /^\d{4}-\d\d-\d\dT/;

// Put `tag` in front of the source of a line `<iso> [source] text`; other lines stay as they are.
export function stamp(line, tag) {
  const match = STAMPED.exec(line);
  return match ? `${match[1]} [${tag}${match[2] ? ' ' + match[2] : ''}] ${match[3]}` : line;
}

// Merge several groups of lines into one text ordered by time, keeping the last `max` lines.
export function mergeByTime(groups, max = 1500) {
  const merged = [];
  for (const group of groups) for (const line of group) merged.push(line);
  merged.sort();
  return merged.slice(-max).join('\n');
}

// A short human label for a source. `names` maps ids to names ({ sandbox: {id: name}, node: {id: name} }).
export function sourceLabel(source, names = {}) {
  if (!source || source === 'all') return 'All sandboxes';
  const index = source.indexOf(':');
  const kind = index < 0 ? source : source.slice(0, index);
  const key = index < 0 ? '' : source.slice(index + 1);
  if (kind === 'sandbox') return `Sandbox ${(names.sandbox && names.sandbox[key]) || key.slice(0, 8)}`;
  if (kind === 'node') return `Node ${(names.node && names.node[key]) || key.slice(0, 8)}`;
  if (kind === 'system') return `Platform: ${key}`;
  return source;
}

// The sandbox ids a source shows, or null when it is not sandbox-based (used by the red lane).
export function sandboxIdsOf(source, runningIds = []) {
  if (source === 'all') return runningIds;
  if (source && source.startsWith('sandbox:')) return [source.slice(8)];
  return null;
}

// Lines of one sandbox, tagged with its name.
async function sandboxLines(item, tail) {
  const result = await sandboxApi.logs(item.id, tail);
  return (result.content || '').split('\n').filter(Boolean).map((line) => stamp(line, item.name));
}

// Read several sandboxes; a sandbox that fails is skipped.
async function readMany(targets, tail) {
  const results = await Promise.allSettled(targets.map((item) => sandboxLines(item, tail)));
  return results.filter((r) => r.status === 'fulfilled').map((r) => r.value);
}

// `loaderFor(source, options)` -> () => Promise<string>. `options.sandboxes` is the list of
// running sandboxes ({ id, name }) used by 'all'.
export function loaderFor(source, options = {}) {
  const sandboxes = options.sandboxes || [];
  if (source === 'all') {
    return async () => mergeByTime(await readMany(sandboxes, 80));
  }
  if (source.startsWith('sandbox:')) {
    const id = source.slice(8);
    return async () => {
      const known = sandboxes.find((item) => item.id === id);
      return mergeByTime([await sandboxLines({ id, name: known ? known.name : id.slice(0, 8) }, 500)]);
    };
  }
  if (source.startsWith('node:')) {
    const nodeId = source.slice(5);
    return async () => {
      const answer = await nodeApi.sandboxes(nodeId);
      const rows = Array.isArray(answer) ? answer : answer.items || answer.sandboxes || [];
      const running = rows.filter((item) => !item.status || item.status === 'running');
      return mergeByTime(await readMany(running, 80));
    };
  }
  if (source.startsWith('system:')) {
    const service = source.slice(7);
    return async () => {
      const answer = await systemApi.logs(service, 500);
      const lines = (answer.content || '').split('\n').filter(Boolean)
        .map((line) => (ISO_START.test(line) ? stamp(line, service) : line));
      return mergeByTime([lines]);
    };
  }
  return async () => '';
}
