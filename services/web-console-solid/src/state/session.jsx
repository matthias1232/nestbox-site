// Session and environment state.
//
// Small and explicit: one provider holding the identity, the CSRF token, and the
// active environment. Everything else is local to a feature.

import { createContext, createSignal, useContext } from 'solid-js';
import * as auth from '../api/auth.js';
import { setCsrfMissingHandler, setCsrfToken, setEnvironment } from '../api/client.js';

const SessionContext = createContext();

export function SessionProvider(props) {
  const [identity, setIdentity] = createSignal(null);
  const [environments, setEnvironments] = createSignal([]);
  const [activeEnvironment, setActiveEnvironment] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [busy, setBusy] = createSignal(false);

  // After a reload the security token is gone; go back to the sign-in form
  // with the reason instead of failing every action with a 403.
  setCsrfMissingHandler(() => {
    setIdentity(null);
    setCsrfToken(null);
    setError('Please sign in again: a page reload ends the security token that changes need.');
  });

  async function signIn(email, password) {
    setBusy(true);
    setError(null);
    try {
      const result = await auth.login(email, password);
      // The CSRF token lives in memory only. Putting it in storage would make it
      // readable by any script on the page, which is the thing it protects.
      setCsrfToken(result.csrf_token);
      setIdentity({
        display_name: result.display_name,
        role: result.role,
        environment_id: result.environment_id,
        user_id: result.user_id ?? null,
      });
      setEnvironments(result.environments || []);
      setActiveEnvironment(result.environment_id);
      setEnvironment(result.environment_id);
      return true;
    } catch (failure) {
      setError(failure.message);
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function signOut() {
    try {
      await auth.logout();
    } catch {
      // A failed logout still clears the local state; the cookie is cleared by
      // the response when it succeeds, and the session expires regardless.
    }
    setIdentity(null);
    setCsrfToken(null);
    setEnvironment(null);
    setActiveEnvironment(null);
  }

  // Restore the identity after a page reload, without a CSRF token: the token
  // is deliberately memory-only, so a reload requires a fresh sign-in before any
  // mutating action. Read-only views keep working.
  function restore(identityPayload) {
    setIdentity({
      display_name: identityPayload.display_name,
      role: identityPayload.role,
      environment_id: identityPayload.environment_id,
      // The owner of a sandbox is matched by id; a reload must not lose it.
      user_id: identityPayload.user_id ?? null,
    });
    setActiveEnvironment(identityPayload.environment_id);
    setEnvironment(identityPayload.environment_id);
  }

  // After a reload: a fresh token for the live session. If the session itself is gone the
  // 401 sends the user to the sign-in form, which is the honest answer.
  async function refreshCsrf() {
    try {
      const result = await auth.csrf();
      setCsrfToken(result.csrf_token);
    } catch {
      /* read-only views still work; the first change asks for a sign-in */
    }
  }

  function switchEnvironment(id) {
    setActiveEnvironment(id);
    setEnvironment(id);
  }

  return (
    <SessionContext.Provider
      value={{
        identity,
        environments,
        activeEnvironment,
        error,
        busy,
        signIn,
        signOut,
        restore,
        refreshCsrf,
        switchEnvironment,
      }}
    >
      {props.children}
    </SessionContext.Provider>
  );
}

export function useSession() {
  const context = useContext(SessionContext);
  if (!context) throw new Error('useSession must be used inside SessionProvider');
  return context;
}
