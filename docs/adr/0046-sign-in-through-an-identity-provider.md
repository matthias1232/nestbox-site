# ADR 0046 — Sign-in through an identity provider, and who may register

Status: Accepted
Date: 2026-10-31

## Context

Providers' customers expect "sign in with Google". Self-hosters expect a password and no outside dependency.

## Decision

1. **OpenID Connect** (Google, GitHub, Microsoft, any generic issuer) replaces only the password check. Sessions, CSRF and revocation stay as in ADR 0004.
2. **`user_identities(provider, subject, user_id)`** links an external identity to a user. Matching is by `(provider, subject)`, never by email alone, because an email can be reused.
3. **Two modes**, `NESTBOX_MODE=saas | selfhosted`. In `saas` a first sign-in starts an onboarding that creates organisation, first data center and first site. In `selfhosted` there is no registration; a setup wizard creates the first administrator, and the password login stays.
4. **Invitations** are single-use links carrying a role and a scope, so a person can be invited to one sandbox.
5. Provider client secrets are stored as vault locators (ADR 0037), never in the database in the clear.

## Alternatives rejected

* **Passwords only.** Every customer would hold another credential and the provider would own reset flows.
* **A hosted identity service (Auth0 and similar).** It would make a self-hosted install depend on someone else's cloud.
* **Matching by email.** An unverified or recycled address would take over an account.

## Invalidation conditions

Revisited when an enterprise customer needs SAML or SCIM.
