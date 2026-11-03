import IconButton from './IconButton.jsx';
import { IconHelp } from './icons.jsx';
import { useUi } from '../state/ui.jsx';

// A small "?" that opens the help drawer at `anchor` ("docs-slug#heading").
export default function HelpTip(props) {
  const ui = useUi();
  return <IconButton icon={IconHelp} label="Help" onClick={() => ui.help.open(props.anchor)} />;
}
