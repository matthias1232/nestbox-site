// Live logs in their own window (/pop/logs/<sandbox id or all>), without the shell.
// The pane layout lives in this window only, so it does not disturb the opener's saved panes.
import { createSignal, onMount } from 'solid-js';
import '../../styles/logs.css';
import { LogsBoard, addPane } from './LogsPage.jsx';
import { sourceLabel } from './logLoaders.js';

export default function LogsPopout(props) {
  const start = props.id && props.id !== 'all' ? `sandbox:${props.id}` : 'all';
  const [panes, setPanes] = createSignal([start]);
  onMount(() => { document.title = `Logs · ${sourceLabel(start)}`; });

  return (
    <main class="logs-popout">
      <header class="logs-popout-head">
        <strong class="brand-mark">nestbox<i>_</i></strong>
        <h1>Live logs</h1>
        <button class="secondary tiny" disabled={panes().length >= 4} onClick={() => addPane(panes, setPanes)}>Add pane</button>
      </header>
      <LogsBoard panes={panes} setPanes={setPanes} runAlerts inWindow />
    </main>
  );
}
