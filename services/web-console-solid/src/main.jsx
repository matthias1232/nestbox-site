// Entry point: mount the app inside the session, UI and live providers.

import { render } from 'solid-js/web';
import { SessionProvider } from './state/session.jsx';
import { UiProvider } from './state/ui.jsx';
import { LiveProvider } from './state/live.jsx';
import App from './App.jsx';
import './styles/tokens.css';
import './styles/app.css';
import './styles/dlp.css';
import './styles/shell.css';

const root = document.getElementById('root');
render(
  () => (
    <SessionProvider>
      <UiProvider>
        <LiveProvider>
          <App />
        </LiveProvider>
      </UiProvider>
    </SessionProvider>
  ),
  root,
);
