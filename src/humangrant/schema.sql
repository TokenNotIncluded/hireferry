CREATE TABLE IF NOT EXISTS hg_requests (
    id TEXT PRIMARY KEY,
    human_link TEXT NOT NULL UNIQUE,
    actor_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    client_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    requested_permissions TEXT NOT NULL CHECK(json_valid(requested_permissions)),
    expected_human_id TEXT,
    phase TEXT NOT NULL CHECK(phase IN ('waiting_human','authenticating_human','awaiting_consent','approved','succeeded','rejected','cancelled','expired')),
    created_at INTEGER NOT NULL,
    status_updated_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    requested_grant_expires_at INTEGER NOT NULL,
    poll_interval INTEGER NOT NULL DEFAULT 5,
    next_poll_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS hg_requests_expiry ON hg_requests(expires_at);
CREATE TABLE IF NOT EXISTS hg_grants (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE REFERENCES hg_requests(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    actor_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    permissions TEXT NOT NULL CHECK(json_valid(permissions)),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    audit_id TEXT NOT NULL UNIQUE
);
CREATE INDEX IF NOT EXISTS hg_grants_user ON hg_grants(user_id,created_at);
CREATE TABLE IF NOT EXISTS hg_sessions (
    request_id TEXT PRIMARY KEY REFERENCES hg_requests(id),
    grant_id TEXT NOT NULL UNIQUE REFERENCES hg_grants(id),
    token_hash TEXT NOT NULL UNIQUE,
    actor_id TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS hg_nonces (
    actor_id TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY(actor_id,nonce)
);
CREATE INDEX IF NOT EXISTS hg_nonces_expiry ON hg_nonces(expires_at);
CREATE TABLE IF NOT EXISTS hg_limits (
    actor_id TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS hg_limits_actor ON hg_limits(actor_id,created_at);
CREATE INDEX IF NOT EXISTS hg_limits_time ON hg_limits(created_at);

-- Suspension is permanent credential revocation, including an approval that
-- has not yet been claimed. Keep this separate from the account auth trigger
-- so auth's own migration does not depend on HumanGrant tables being present.
CREATE TRIGGER IF NOT EXISTS hg_account_suspended AFTER UPDATE OF banned ON users
WHEN NEW.banned=1 BEGIN
    UPDATE hg_grants SET revoked_at=unixepoch()
    WHERE user_id=NEW.id AND revoked_at IS NULL;
    UPDATE hg_requests SET phase='cancelled',status_updated_at=unixepoch()
    WHERE phase='approved' AND id IN (SELECT request_id FROM hg_grants WHERE user_id=NEW.id);
END;

-- Also invalidate any grants belonging to accounts already suspended when an
-- existing database first receives this migration. Reopening is idempotent.
UPDATE hg_grants SET revoked_at=unixepoch()
WHERE revoked_at IS NULL AND user_id IN (SELECT id FROM users WHERE banned=1);
UPDATE hg_requests SET phase='cancelled',status_updated_at=unixepoch()
WHERE phase='approved' AND id IN (
    SELECT g.request_id FROM hg_grants g JOIN users u ON u.id=g.user_id WHERE u.banned=1
);
