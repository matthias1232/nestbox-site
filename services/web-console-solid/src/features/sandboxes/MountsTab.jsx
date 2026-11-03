// Directories on the node that this sandbox sees, and moving them between sandboxes.
//
// A mount is a persistent root bound into a sandbox. Docker fixes the binds when a
// container is created, so changing them takes effect when the container is
// replaced: "Apply now" does that. What lives on a persistent root survives; what
// was written elsewhere inside the old container does not, and the button says so.
import MountEditor from './MountEditor.jsx';

export default function MountsTab(props) {
  return <MountEditor sandboxId={props.sandbox.id} />;
}
