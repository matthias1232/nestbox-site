// The app: sign-in gate, the shell, and the route switch.
//
// The address bar is the single source of truth (routing.js, contract C1). This file maps what
// `parsePath` returns to one page component and wraps it in the shell, except for the pop-out
// windows, which own their whole window. Menu pages come from `features/shell/pages.js`.

import { Show, createSignal, onMount, onCleanup } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import { useSession } from './state/session.jsx';
import * as auth from './api/auth.js';
import LoginPage from './features/auth/LoginPage.jsx';
import AdminCenter from './features/sandboxes/AdminCenter.jsx';
import WorkboxPage from './features/workbox/WorkboxPage.jsx';
import MyBoxesPage from './features/workbox/MyBoxesPage.jsx';
import TerminalPopout from './features/workbox/TerminalPopout.jsx';
import DesktopPopout from './features/workbox/DesktopPopout.jsx';
import HomePage from './features/home/HomePage.jsx';
import DataCenterPage from './features/inventory/DataCenterPage.jsx';
import SitePage from './features/inventory/SitePage.jsx';
import NodePage from './features/inventory/NodePage.jsx';
import ModelPage from './features/models/ModelPage.jsx';
import ClusterDetailPage from './features/clusters/ClusterDetailPage.jsx';
import DocsPage from './features/help/DocsPage.jsx';
import LogsPopout from './features/logs/LogsPopout.jsx';
import TrafficPopout from './features/traffic/TrafficPopout.jsx';
import Shell from './features/shell/Shell.jsx';
import { PAGES } from './features/shell/pages.js';
import { SHELLLESS, navigate, parsePath, paths } from './routing.js';
import { adoptOpenerCsrf } from './api/client.js';

const readPlace = () => parsePath(window.location.pathname, window.location.search);

export default function App() {
  const session = useSession();
  const [checked, setChecked] = createSignal(false);
  const [place, setPlace] = createSignal(readPlace());

  const role = () => session.identity()?.role;
  const openAdmin = (id) => navigate(paths.sandbox(id));
  const closeAdmin = () => navigate(paths.menu('sandboxes'));

  // The back button must move the screen, otherwise the address bar and the screen disagree.
  const onPop = () => setPlace(readPlace());
  window.addEventListener('popstate', onPop);
  onCleanup(() => window.removeEventListener('popstate', onPop));

  // A reload must not show the login form to someone who is still signed in. The session
  // cookie is HttpOnly, so the only way to know is to ask the server; a 401 here is the
  // normal "not signed in" answer, not a failure.
  onMount(async () => {
    try {
      const data = await auth.me();
      // The CSRF token lives in memory only; a reload loses it, so ask for a new one. A pop-out
      // window shares its opener's token instead: asking for a new one would invalidate the
      // token the main window holds.
      session.restore(data);
      if (!(SHELLLESS.has(place().kind) && adoptOpenerCsrf())) {
        await session.refreshCsrf();
      }
      // A member's day starts with their own sandboxes, not with the operator's overview.
      if (session.identity()?.role === 'member' && window.location.pathname === '/') {
        navigate(paths.my());
      }
    } catch {
      /* not signed in */
    } finally {
      setChecked(true);
    }
  });

  // The page for a place that lives inside the shell.
  function Page() {
    const p = place();
    switch (p.kind) {
      case 'dc': return <DataCenterPage id={p.id} />;
      case 'site': return <SitePage id={p.id} />;
      case 'node': return <NodePage id={p.id} />;
      case 'admin': return <AdminCenter sandboxId={p.id} initialTab={p.tab || undefined} onClose={closeAdmin} onFollow={openAdmin} />;
      case 'box': return <WorkboxPage sandboxId={p.id} />;
      case 'my': return <MyBoxesPage />;
      case 'model': return <ModelPage id={p.id} />;
      case 'cluster': return <ClusterDetailPage id={p.id} />;
      case 'docs': return <DocsPage slug={p.slug} />;
      case 'menu': {
        const component = PAGES[p.id];
        return component
          ? <Dynamic component={component} role={role()} onOpenAdmin={openAdmin} onOpenSandbox={openAdmin} onOpenNode={() => navigate(paths.menu('nodes'))} />
          : <HomePage />;
      }
      default: return <HomePage />;
    }
  }

  return (
    <Show when={checked()} fallback={<div class="login"><p class="muted">Loading…</p></div>}>
      <Show when={session.identity()} fallback={<LoginPage />}>
        {/* Pop-out windows own their window: no sidebar, no menu. */}
        <Show when={place().kind === 'terminal'}><TerminalPopout sandboxId={place().id} tab={place().tab} /></Show>
        <Show when={place().kind === 'desktop'}><DesktopPopout sandboxId={place().id} /></Show>
        <Show when={place().kind === 'poplogs'}><LogsPopout id={place().id} /></Show>
        <Show when={place().kind === 'poptraffic'}><TrafficPopout /></Show>
        <Show when={!SHELLLESS.has(place().kind)}>
          <Shell place={place()}>
            {/* Keyed: choosing another data center remounts the page, so every screen reloads its data. */}
            <Show when={session.activeEnvironment()} keyed>
              {() => <Page />}
            </Show>
          </Shell>
        </Show>
      </Show>
    </Show>
  );
}
