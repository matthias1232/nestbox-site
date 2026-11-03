// Live traffic in a window of its own (/pop/traffic): the same view as the page, without the shell.
import { onMount } from 'solid-js';
import { TrafficView } from './TrafficPage.jsx';

export default function TrafficPopout() {
  onMount(() => {
    document.title = 'Live traffic';
  });
  return <TrafficView popout />;
}
