# auth/ — login

## Purpose

The only screen reachable without a session. It collects an email and a
password (or offers the identity-provider links), in the website's look: form column left,
illustration or looping video right (`/illustrations/login.webp`, optional `login-loop.mp4`;
both fall back quietly when missing).
Styles: `../../styles/login.css`, imported by `LoginPage.jsx`.

## Rules

- This page states nothing it cannot know. Its two corner labels are static brand text
  ("OPEN SOURCE", "SELF-HOSTED"), not live state: before sign-in there is no data center,
  no gateway and no workload count to report (P13, review F14).
- The looping video is decoration and is optional: the `prefersReducedMotion` flag computed
  from `prefers-reduced-motion` gates it, so the `<video preload="none">` exists only when
  that flag is false and the 775 KB clip is not fetched for a visitor who asked for less
  motion. The poster image always renders behind it, and both media fall back quietly via
  `onError`.
- The password field is never mirrored into state; it is read at submit.
- The show/hide toggle uses the shared `IconEye` / `IconEyeOff` from `components/icons.jsx` (X10), not a private glyph.
- A failure shows the server's message, which is deliberately uniform so it does
  not reveal whether the account exists.
- On success the CSRF token is held in memory by the session provider only.
