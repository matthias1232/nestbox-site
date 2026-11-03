// PaletteHost: mounted once by the Shell (shell/pages.js EXTRAS). It owns the open state in
// `ui.palette`, opens on Ctrl/Cmd K, loads the help index once, and navigates on Enter.
import { createSignal, onCleanup } from 'solid-js';
import CommandPalette from '../../components/CommandPalette.jsx';
import { menuEntries, recentRecord } from '../../components/palette.js';
import { search } from '../../api/search.js';
import { index as helpIndex } from '../../api/help.js';
import { menuFor } from '../../navigation.js';
import { navigate, paths } from '../../routing.js';
import { useSession } from '../../state/session.jsx';
import { onKey, useUi } from '../../state/ui.jsx';

export default function PaletteHost() {
  const ui = useUi();
  const session = useSession();
  const [docs, setDocs] = createSignal([]);
  let docsRequested = false;

  // The help index is a static file: load it the first time the palette opens.
  function loadDocs() {
    if (docsRequested) return;
    docsRequested = true;
    helpIndex().then(setDocs).catch(() => { docsRequested = false; });
  }

  const stopKey = onKey('mod+k', (event) => {
    event.preventDefault();
    if (ui.palette.isOpen()) {
      ui.palette.close();
    } else {
      loadDocs();
      ui.palette.open();
    }
  });
  onCleanup(stopKey);

  const entries = () => menuEntries(menuFor(session.identity()?.role), (id) => paths.menu(id));

  function open(path, item) {
    if (item) ui.palette.addRecent(recentRecord(item));
    ui.palette.close();
    navigate(path);
  }

  return (
    <CommandPalette
      open={ui.palette.isOpen()}
      onClose={ui.palette.close}
      search={search}
      docsIndex={docs()}
      environments={session.environments()}
      menu={entries()}
      recent={ui.palette.recent()}
      onOpen={open}
    />
  );
}
