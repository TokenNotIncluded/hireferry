//! Database-level regression coverage for account suspension and schema upgrades.
//! All identities and credentials are disposable fixtures in an in-memory database.
use hireferry::{
    db,
    humangrant::{self, GrantContext, Permission},
    now,
};
use rusqlite::{Connection, params};

fn database() -> Connection {
    let db = db::open(":memory:").unwrap();
    db.execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('owner','owner@example.invalid','seeker',0,0),('other','other@example.invalid','seeker',0,0)", []).unwrap();
    db
}

fn grant(db: &Connection, id: &str, user: &str, claimed: bool) -> GrantContext {
    let request = format!("{id}_request");
    let actor = format!("ed25519:{id}");
    let token_hash = format!("{id}_hash");
    let expires = now() + 3600;
    let permissions = vec![Permission {
        action: "profile.read".into(),
        resource: "self".into(),
    }];
    let encoded = serde_json::to_string(&permissions).unwrap();
    db.execute("INSERT INTO hg_requests(id,human_link,actor_id,public_key,client_id,purpose,requested_permissions,phase,created_at,status_updated_at,expires_at,requested_grant_expires_at) VALUES(?1,?1,?2,'synthetic-public-key','hireferry-cli','Synthetic migration fixture',?3,?4,?5,?5,?6,?6)", params![request,actor,encoded,if claimed { "succeeded" } else { "approved" },now(),expires]).unwrap();
    db.execute("INSERT INTO hg_grants(id,request_id,user_id,actor_id,client_id,permissions,created_at,expires_at,audit_id) VALUES(?1,?2,?3,?4,'hireferry-cli',?5,?6,?7,?1)", params![id,request,user,actor,encoded,now(),expires]).unwrap();
    if claimed {
        db.execute("INSERT INTO hg_sessions(request_id,grant_id,token_hash,actor_id,created_at) VALUES(?1,?2,?3,?4,?5)", params![request,id,token_hash,actor,now()]).unwrap();
    }
    GrantContext {
        grant_id: id.into(),
        user_id: user.into(),
        actor_id: actor,
        client_id: "hireferry-cli".into(),
        token_hash,
        permissions,
        expires_at: expires,
    }
}

fn revoked_at(db: &Connection, id: &str) -> Option<i64> {
    db.query_row(
        "SELECT revoked_at FROM hg_grants WHERE id=?1",
        [id],
        |row| row.get(0),
    )
    .unwrap()
}

fn phase(db: &Connection, grant_id: &str) -> String {
    db.query_row(
        "SELECT r.phase FROM hg_requests r JOIN hg_grants g ON g.request_id=r.id WHERE g.id=?1",
        [grant_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn suspension_revokes_claimed_and_unclaimed_grants_atomically_and_permanently() {
    let mut db = database();
    let claimed = grant(&db, "claimed", "owner", true);
    grant(&db, "unclaimed", "owner", false);
    let unaffected = grant(&db, "unaffected", "other", true);
    assert!(humangrant::validate_grant(&db, &claimed).is_ok());

    // A failed administrative transaction must not leave a half-revoked grant.
    {
        let tx = db.transaction().unwrap();
        tx.execute("UPDATE users SET banned=1 WHERE id='owner'", [])
            .unwrap();
        assert!(revoked_at(&tx, "claimed").is_some());
        assert!(revoked_at(&tx, "unclaimed").is_some());
        assert_eq!(phase(&tx, "unclaimed"), "cancelled");
        tx.rollback().unwrap();
    }
    assert_eq!(revoked_at(&db, "claimed"), None);
    assert_eq!(revoked_at(&db, "unclaimed"), None);
    assert_eq!(phase(&db, "unclaimed"), "approved");
    assert!(humangrant::validate_grant(&db, &claimed).is_ok());

    db.execute("UPDATE users SET banned=1 WHERE id='owner'", [])
        .unwrap();
    let revoked = revoked_at(&db, "claimed").unwrap();
    assert!(humangrant::validate_grant(&db, &claimed).is_err());
    assert_eq!(phase(&db, "claimed"), "succeeded");
    assert_eq!(phase(&db, "unclaimed"), "cancelled");
    assert!(humangrant::validate_grant(&db, &unaffected).is_ok());

    db.execute("UPDATE users SET banned=0 WHERE id='owner'", [])
        .unwrap();
    db::migrate(&db).unwrap();
    assert_eq!(revoked_at(&db, "claimed"), Some(revoked));
    assert!(humangrant::validate_grant(&db, &claimed).is_err());
    assert_eq!(phase(&db, "unclaimed"), "cancelled");
    assert!(humangrant::validate_grant(&db, &unaffected).is_ok());
    let fresh = grant(&db, "fresh", "owner", true);
    assert!(humangrant::validate_grant(&db, &fresh).is_ok());
}

#[test]
fn upgrade_revokes_existing_suspended_account_grants_and_is_idempotent() {
    let db = database();
    let old = grant(&db, "legacy", "owner", true);
    grant(&db, "legacy_unclaimed", "owner", false);
    let unaffected = grant(&db, "unaffected", "other", true);
    // Simulate a database predating HumanGrant's permanent suspension rule.
    db.execute_batch("DROP TRIGGER cli_account_suspended; DROP TABLE cli_refresh_tokens; DROP TABLE cli_tokens; DROP TABLE cli_grants; DROP TABLE cli_authorizations; DROP TRIGGER hg_account_suspended; PRAGMA user_version=6; UPDATE users SET banned=1 WHERE id='owner';").unwrap();
    assert_eq!(revoked_at(&db, "legacy"), None);

    db::migrate(&db).unwrap();
    let revoked = revoked_at(&db, "legacy").unwrap();
    assert_eq!(phase(&db, "legacy_unclaimed"), "cancelled");
    db::migrate(&db).unwrap();
    assert_eq!(revoked_at(&db, "legacy"), Some(revoked));
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='hg_account_suspended'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);

    db.execute("UPDATE users SET banned=0 WHERE id='owner'", [])
        .unwrap();
    assert!(humangrant::validate_grant(&db, &old).is_err());
    assert_eq!(phase(&db, "legacy_unclaimed"), "cancelled");
    assert!(humangrant::validate_grant(&db, &unaffected).is_ok());
}

#[test]
fn version_seven_humangrant_permissions_and_sessions_survive_cli_version_eight_upgrade() {
    let db = database();
    let claimed = grant(&db, "claimed_before_cli", "owner", true);
    grant(&db, "approved_before_cli", "other", false);
    db.execute_batch("DROP TRIGGER cli_account_suspended; DROP TABLE cli_refresh_tokens; DROP TABLE cli_tokens; DROP TABLE cli_grants; DROP TABLE cli_authorizations; PRAGMA user_version=7;").unwrap();
    assert!(humangrant::validate_grant(&db, &claimed).is_ok());

    db::migrate(&db).unwrap();
    let current = humangrant::validate_grant(&db, &claimed).unwrap();
    assert_eq!(current.user_id, claimed.user_id);
    assert_eq!(current.actor_id, claimed.actor_id);
    assert_eq!(current.client_id, claimed.client_id);
    assert_eq!(current.token_hash, claimed.token_hash);
    assert_eq!(current.permissions, claimed.permissions);
    assert_eq!(current.expires_at, claimed.expires_at);
    assert_eq!(phase(&db, "approved_before_cli"), "approved");
    assert_eq!(revoked_at(&db, "claimed_before_cli"), None);
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM cli_authorizations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db::migrate(&db).unwrap();
    assert!(humangrant::validate_grant(&db, &claimed).is_ok());
}
