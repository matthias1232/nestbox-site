-- 0061_tenant_tree - organisations form a tree, a data center has sites, and a sandbox can be
-- handed to one person (ADR 0044, 0045, 0046).
--
--   * organizations get a parent and a kind (platform / partner / customer);
--   * a new role `member` sits between viewer and operator: it works inside the sandboxes it
--     owns or was granted, and nowhere else (row-level security does the "nowhere else");
--   * `sandbox_grants` hand one sandbox to one user;
--   * `sites` are the places an environment (a data center) runs nodes;
--   * nodes may belong to one environment and one site;
--   * `user_identities` link an external sign-in to a user, `invitations` carry a role and a
--     scope to a person, `oidc_states` hold a sign-in in flight.

-- ---------------------------------------------------------------------------
-- Organisation tree
-- ---------------------------------------------------------------------------

ALTER TABLE organizations
    ADD COLUMN parent_id uuid REFERENCES organizations(id) ON DELETE RESTRICT,
    ADD COLUMN kind text NOT NULL DEFAULT 'customer',
    ADD CONSTRAINT organizations_kind_valid CHECK (kind IN ('platform', 'partner', 'customer')),
    ADD CONSTRAINT organizations_not_own_parent CHECK (parent_id IS NULL OR parent_id <> id);

CREATE INDEX organizations_parent_idx ON organizations (parent_id);

-- The organisation that existed before the tree is the provider's own.
UPDATE organizations SET kind = 'platform'
 WHERE id = (SELECT id FROM organizations ORDER BY created_at LIMIT 1);

-- One person, one account: the sign-in is matched by (provider, subject) and the email must not
-- belong to two accounts, or a login by email could not say which one is meant.
CREATE UNIQUE INDEX users_email_global ON users (lower(email));

-- ---------------------------------------------------------------------------
-- The member role
-- ---------------------------------------------------------------------------

ALTER TABLE users DROP CONSTRAINT users_role_valid;
ALTER TABLE users ADD CONSTRAINT users_role_valid
    CHECK (role IN ('admin', 'operator', 'member', 'viewer'));
ALTER TABLE user_scopes DROP CONSTRAINT user_scopes_role_valid;
ALTER TABLE user_scopes ADD CONSTRAINT user_scopes_role_valid
    CHECK (role IN ('admin', 'operator', 'member', 'viewer'));

-- ---------------------------------------------------------------------------
-- Which environments a user reaches, and with which role.
--
-- Direct membership (user_scopes) plus the downward reach of a provider: an admin of a
-- platform or partner organisation is admin of every environment of that organisation and of
-- the organisations below it. A customer's admin reaches only the customer's own.
-- If two rows name the same environment the highest role wins.
-- ---------------------------------------------------------------------------

CREATE FUNCTION nb_role_rank(p_role text) RETURNS integer
    LANGUAGE sql IMMUTABLE AS
$$ SELECT CASE p_role WHEN 'admin' THEN 4 WHEN 'operator' THEN 3 WHEN 'member' THEN 2 ELSE 1 END $$;

CREATE FUNCTION nb_user_environments(p_user uuid)
    RETURNS TABLE (environment_id uuid, role text)
    LANGUAGE sql STABLE AS
$$
    WITH RECURSIVE reach AS (
        SELECT o.id FROM users u
          JOIN organizations o ON o.id = u.organization_id
         WHERE u.id = p_user AND u.role = 'admin' AND u.status = 'active'
           AND o.kind IN ('platform', 'partner')
        UNION
        SELECT c.id FROM organizations c JOIN reach r ON c.parent_id = r.id
    ),
    every AS (
        SELECT s.environment_id, s.role FROM user_scopes s WHERE s.user_id = p_user
        UNION ALL
        SELECT e.id, 'admin' FROM environments e JOIN reach r ON r.id = e.organization_id
    )
    SELECT DISTINCT ON (every.environment_id) every.environment_id, every.role
      FROM every
     ORDER BY every.environment_id, nb_role_rank(every.role) DESC
$$;

-- ---------------------------------------------------------------------------
-- Sites
-- ---------------------------------------------------------------------------

CREATE TABLE sites (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name           text NOT NULL,
    slug           text NOT NULL,
    kind           text NOT NULL DEFAULT 'home',
    provider       text,
    region         text,
    -- The WireGuard overlay of the site's data center (ADR 0047), filled when nodes join.
    overlay_cidr   text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT sites_slug_unique UNIQUE (environment_id, slug),
    CONSTRAINT sites_slug_format CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
    CONSTRAINT sites_kind_valid CHECK (kind IN ('home', 'cloud', 'edge'))
);

ALTER TABLE sites ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON sites
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

INSERT INTO sites (environment_id, name, slug, kind)
     SELECT id, 'Main site', 'main', 'home' FROM environments;

ALTER TABLE nodes
    ADD COLUMN environment_id uuid REFERENCES environments(id) ON DELETE SET NULL,
    ADD COLUMN site_id uuid REFERENCES sites(id) ON DELETE SET NULL;
CREATE INDEX nodes_environment_idx ON nodes (environment_id);

ALTER TABLE gateways ADD COLUMN site_id uuid REFERENCES sites(id) ON DELETE SET NULL;

-- ---------------------------------------------------------------------------
-- Sandbox grants: one sandbox, one person.
-- ---------------------------------------------------------------------------

CREATE TABLE sandbox_grants (
    sandbox_id     uuid NOT NULL REFERENCES sandboxes(id) ON DELETE CASCADE,
    user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    environment_id uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    granted_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (sandbox_id, user_id)
);
CREATE INDEX sandbox_grants_user_idx ON sandbox_grants (user_id);

ALTER TABLE sandbox_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON sandbox_grants
    USING (environment_id = current_setting('app.environment_id', true)::uuid)
    WITH CHECK (environment_id = current_setting('app.environment_id', true)::uuid);

-- A granted sandbox is visible and writable like an owned one. Everything that follows the
-- sandbox (telemetry, rules, supervisors) asks `sandboxes`, so it follows the grant too.
DROP POLICY IF EXISTS tenant_isolation ON sandboxes;
CREATE POLICY tenant_isolation ON sandboxes
    USING (
        environment_id = current_setting('app.environment_id', true)::uuid
        AND (
            coalesce(current_setting('app.actor_role', true), '') IN ('', 'admin')
            OR owner_user_id IS NULL
            OR owner_user_id::text = coalesce(current_setting('app.actor_id', true), '')
            OR EXISTS (SELECT 1 FROM sandbox_grants g
                        WHERE g.sandbox_id = sandboxes.id
                          AND g.user_id::text = coalesce(current_setting('app.actor_id', true), ''))
        )
    )
    WITH CHECK (
        environment_id = current_setting('app.environment_id', true)::uuid
        AND (
            coalesce(current_setting('app.actor_role', true), '') IN ('', 'admin')
            OR owner_user_id IS NULL
            OR owner_user_id::text = coalesce(current_setting('app.actor_id', true), '')
            OR EXISTS (SELECT 1 FROM sandbox_grants g
                        WHERE g.sandbox_id = sandboxes.id
                          AND g.user_id::text = coalesce(current_setting('app.actor_id', true), ''))
        )
    );

-- ---------------------------------------------------------------------------
-- Sign-in through an identity provider, invitations
-- ---------------------------------------------------------------------------

CREATE TABLE user_identities (
    provider   text NOT NULL,
    subject    text NOT NULL,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    email      text,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX user_identities_user_idx ON user_identities (user_id);

CREATE TABLE invitations (
    id              uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    -- Only the hash of the link token is stored, like a session.
    token_hash      text NOT NULL UNIQUE,
    organization_id uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    environment_id  uuid NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    role            text NOT NULL,
    -- When set, the person is handed this one sandbox and nothing else.
    sandbox_id      uuid REFERENCES sandboxes(id) ON DELETE CASCADE,
    email           text,
    created_by      uuid REFERENCES users(id) ON DELETE SET NULL,
    expires_at      timestamptz NOT NULL,
    accepted_at     timestamptz,
    accepted_by     uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT invitations_role_valid CHECK (role IN ('admin', 'operator', 'member', 'viewer'))
);

-- Not row-level secured, like `users` and `sessions`: a person follows an invitation link
-- before any tenant is known. The listing for admins filters by environment explicitly.

-- A sign-in in flight: the state and the PKCE verifier, kept server-side so the browser
-- carries nothing it could tamper with. Rows are short-lived and purged on use.
CREATE TABLE oidc_states (
    state         text PRIMARY KEY,
    provider      text NOT NULL,
    code_verifier text NOT NULL,
    nonce         text NOT NULL,
    invitation_id uuid REFERENCES invitations(id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now()
);
