// The five steps a connection takes, written for someone who has never seen the system.
//
// Pure: it turns one recorded event into the steps, so what the screen says is tested.

const short = (path) => (path ? path.split('/').filter(Boolean).slice(-2).join('/') : null);

export function journeySteps(event) {
  const allowed = event.decision === 'allow';
  const audit = event.decision === 'audit';
  const target = `${event.host}${event.port ? `:${event.port}` : ''}`;
  const program = event.binary ? short(event.binary) : null;
  const policy = (event.reason_text || '').match(/policy ([A-Za-z0-9_]+)/)?.[1] || null;
  const inspected = event.inspected || ['rest', 'mcp'].includes(event.protocol) || event.mcp_method || event.method;
  const what = event.mcp_tool
    ? `the MCP tool “${event.mcp_tool}”`
    : event.method
      ? `${event.method} ${event.path || ''}`.trim()
      : null;

  return [
    {
      n: 1, who: 'The agent', icon: '🤖',
      title: program ? `${program} opens a connection to ${target}` : `A program opens a connection to ${target}`,
      says: 'The agent runs inside the sandbox. It asks for a connection like on any computer, and it does not know what is on the other side of the boundary.',
    },
    {
      n: 2, who: 'The sandbox', icon: '📦',
      title: program ? `It notes the program: ${event.binary}` : 'It notes which program asked',
      says: 'The sandbox has no network of its own. It looks up which executable made the request (from the kernel, not from what the program claims) and hands the request to the supervisor.',
    },
    {
      n: 3, who: 'The supervisor', icon: '🛡️',
      title: policy ? `It checks the policy and finds “${policy}”` : allowed ? 'It checks the policy and finds a rule' : 'It checks the policy and finds no rule',
      says: 'The supervisor sits outside the boundary and is the only one that decides. It compares the program and the destination with the rules of this sandbox.',
    },
    {
      n: 4, who: 'The decision', icon: allowed ? '✅' : audit ? '👁️' : '⛔',
      title: allowed ? 'Allowed' : audit ? 'Let through, and recorded' : 'Denied',
      says: allowed
        ? `A rule names this program and this destination.${what && inspected ? ` The supervisor also read the request (${what}) and it was within what the rule permits.` : ''}`
        : audit
          ? 'The rule is in audit mode: it lets the connection through but records it, so you can see what enforcing it would block.'
          : `Nothing allows this. Everything that is not allowed is refused, so the connection stops here${event.reason_text ? ` (${event.reason_text})` : ''}.`,
    },
    {
      n: 5, who: 'The destination', icon: allowed || audit ? '🌐' : '🚫',
      title: allowed || audit ? `${target} is reached${event.status ? ` (answered ${event.status})` : ''}` : `${target} is never contacted`,
      says: allowed || audit
        ? 'Only now does the supervisor open the real connection and relay the traffic. Credentials are added here when policy allows, so the agent never holds them.'
        : 'The program gets a refusal. If this should have worked, press “Allow” on the request in the Traffic tab; the rule is created and active in one step.',
    },
  ];
}
