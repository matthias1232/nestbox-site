# auth/ — login

## Purpose

The only screen reachable without a session. It collects an email and a
password (or offers the identity-provider links), in the website's look: form column left,
illustration or looping video right (`/illustrations/login.webp`, optional `login-loop.mp4`;
both fall back quietly when missing; the video is skipped under reduced motion).
Styles: `../../styles/login.css`, imported by `LoginPage.jsx`.

## Rules

- The password field is never mirrored into state; it is read at submit.
- The show/hide toggle uses the shared `IconEye` / `IconEyeOff` from `components/icons.jsx` (X10), not a private glyph.
- A failure shows the server's message, which is deliberately uniform so it does
  not reveal whether the account exists.
- On success the CSRF token is held in memory by the session provider only.
