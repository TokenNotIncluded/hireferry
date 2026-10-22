CREATE TABLE IF NOT EXISTS promo_applications(
    id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id),
    reason TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','approved','rejected')),
    review_note TEXT NOT NULL DEFAULT '', reviewed_by TEXT,
    created_at INTEGER NOT NULL, reviewed_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS one_pending_promo_application ON promo_applications(user_id) WHERE status='pending';
CREATE INDEX IF NOT EXISTS promo_applications_user ON promo_applications(user_id,created_at);
CREATE INDEX IF NOT EXISTS promo_applications_queue ON promo_applications(status,created_at);
CREATE TABLE IF NOT EXISTS promo_codes(
    id TEXT PRIMARY KEY, code TEXT NOT NULL UNIQUE COLLATE NOCASE, label TEXT NOT NULL DEFAULT '',
    application_id TEXT UNIQUE REFERENCES promo_applications(id),
    free_activation_limit INTEGER NOT NULL DEFAULT 1 CHECK(free_activation_limit BETWEEN 0 AND 1000000),
    publish_limit INTEGER NOT NULL DEFAULT 1 CHECK(publish_limit BETWEEN 0 AND 1000000),
    discount_percent INTEGER NOT NULL DEFAULT 100 CHECK(discount_percent BETWEEN 0 AND 100),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)), expires_at INTEGER,
    created_by TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS promo_redemptions(
    order_id TEXT PRIMARY KEY REFERENCES orders(id), code_id TEXT NOT NULL REFERENCES promo_codes(id),
    user_id TEXT NOT NULL REFERENCES users(id), purpose TEXT NOT NULL CHECK(purpose IN ('activation','publish')),
    status TEXT NOT NULL CHECK(status IN ('reserved','consumed','released')),
    request_key TEXT NOT NULL, request_hash TEXT NOT NULL, reserved_until INTEGER, created_at INTEGER NOT NULL,
    UNIQUE(user_id,purpose,request_key),
    CHECK(status<>'reserved' OR (purpose='publish' AND reserved_until IS NOT NULL))
);
CREATE INDEX IF NOT EXISTS promo_redemptions_quota ON promo_redemptions(code_id,purpose,status);
CREATE INDEX IF NOT EXISTS promo_reservations_due ON promo_redemptions(reserved_until) WHERE status='reserved';
CREATE VIEW IF NOT EXISTS promo_code_balances AS
    SELECT c.*,
        (SELECT count(*) FROM promo_redemptions r WHERE r.code_id=c.id AND r.purpose='activation' AND r.status='consumed') AS activation_used,
        (SELECT count(*) FROM promo_redemptions r WHERE r.code_id=c.id AND r.purpose='publish' AND r.status='consumed') AS publish_used,
        (SELECT count(*) FROM promo_redemptions r WHERE r.code_id=c.id AND r.purpose='publish' AND r.status='reserved' AND r.reserved_until>CAST(strftime('%s','now') AS INTEGER)) AS publish_reserved
    FROM promo_codes c;
-- All cancellation paths, including deletion elsewhere in the app,
-- release their reservation. Settled uses are never replenished by refunds.
CREATE TRIGGER IF NOT EXISTS promo_order_terminal AFTER UPDATE OF status ON orders
WHEN OLD.status='pending' AND NEW.status IN ('paid','paid_needs_review','cancelled','expired','failed')
BEGIN
    UPDATE promo_redemptions SET status=CASE WHEN NEW.status='paid' THEN 'consumed' ELSE 'released' END
    WHERE order_id=NEW.id AND status='reserved';
END;
