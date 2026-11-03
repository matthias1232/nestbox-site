// One node's tunnel, as a badge: the state word, the age of its last handshake,
// and the colour that belongs to it.
//
// The badge is the whole answer to "is that machine reachable right now". The
// age separates a healthy tunnel from one that only looks healthy: `stale` means
// the hub has not heard from the peer for minutes. Props: `state`, `age`
// (seconds or null), optional `title`.

import { tunnelLabel, tunnelTone } from './tunnels.js';

export default function TunnelBadge(props) {
  return (
    <span
      class={`badge ${tunnelTone(props.state)}`}
      title={props.title || 'The WireGuard tunnel from this gateway to the node.'}
    >
      <i />{tunnelLabel(props.state, props.age)}
    </span>
  );
}
