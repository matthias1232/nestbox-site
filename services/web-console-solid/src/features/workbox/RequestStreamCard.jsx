// Which models this sandbox is using right now, in the workbox.
//
// Stub: F-4 fills it.
import RequestStream from '../live/RequestStream.jsx';

export default function RequestStreamCard(props) {
  return (
    <section class="wb-card">
      <div class="wb-card-head">
        <h2>My models live</h2>
      </div>
      <RequestStream sandboxId={props.sandboxId} />
    </section>
  );
}
