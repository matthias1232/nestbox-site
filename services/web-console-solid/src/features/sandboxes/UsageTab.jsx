// What this sandbox's agents used: models, tokens, cache, cost.
import UsageView from '../usage/UsageView.jsx';

export default function UsageTab(props) {
  return <UsageView sandboxId={props.sandbox.id} defaultGroup="model" />;
}
