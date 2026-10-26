use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{AppState, auth, commerce, config::Config, db};
use http_body_util::BodyExt;
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _dir: TempDir,
    state: AppState,
    app: Router,
}
struct Browser {
    cookie: String,
    csrf: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::new(Config {
            development: true,
            database: dir
                .path()
                .join("interactions.sqlite3")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        })
        .unwrap();
        {
            let db = state.conn().unwrap();
            for (id, role) in [
                ("owner", "employer"),
                ("alice", "seeker"),
                ("bob", "seeker"),
            ] {
                db.execute("INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES(?1,?2,?3,?4,1,1)", params![id,format!("{id}@example.invalid"),format!("Private {id}"),role]).unwrap();
            }
            for id in ["job", "other"] {
                db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES(?1,'owner','Example role','Description',300000,'published',1,1)", [id]).unwrap();
            }
        }
        let app = hireferry::app(state.clone());
        Self {
            _dir: dir,
            state,
            app,
        }
    }
    fn browser(&self, uid: &str) -> Browser {
        let raw = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,?4,?5)", params![auth::hash_token(&raw),uid,csrf,hireferry::now()+3600,hireferry::now()]).unwrap();
        Browser {
            cookie: format!("hf_session={raw}"),
            csrf,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        browser: Option<&Browser>,
        csrf: bool,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("origin", "http://localhost:3000");
        if let Some(browser) = browser {
            req = req.header("cookie", &browser.cookie);
            if csrf {
                req = req.header("x-csrf-token", &browser.csrf);
            }
        }
        let body = if let Some(value) = body {
            req = req.header("content-type", "application/json");
            Body::from(value.to_string())
        } else {
            Body::empty()
        };
        let res = self
            .app
            .clone()
            .oneshot(req.body(body).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn get(&self, browser: Option<&Browser>) -> Value {
        let (status, data) = self
            .request(
                "GET",
                "/api/public/jobs/job/interactions",
                browser,
                false,
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{data}");
        data
    }
}

#[tokio::test]
async fn votes_set_switch_cancel_and_personalize_without_duplicate_rows() {
    let f = Fixture::new();
    let alice = f.browser("alice");
    let bob = f.browser("bob");
    for (vote, up, down) in [(1, 1, 0), (1, 1, 0), (-1, 0, 1), (0, 0, 0)] {
        let (status, value) = f
            .request(
                "PUT",
                "/api/jobs/job/vote",
                Some(&alice),
                true,
                Some(json!({"vote":vote})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(
            (
                value["upvotes"].as_i64(),
                value["downvotes"].as_i64(),
                value["my_vote"].as_i64()
            ),
            (Some(up), Some(down), Some(vote))
        );
    }
    f.request(
        "PUT",
        "/api/jobs/job/vote",
        Some(&alice),
        true,
        Some(json!({"vote":1})),
    )
    .await;
    f.request(
        "PUT",
        "/api/jobs/job/vote",
        Some(&bob),
        true,
        Some(json!({"vote":-1})),
    )
    .await;
    let public = f.get(None).await;
    assert_eq!(public["upvotes"], 1);
    assert_eq!(public["downvotes"], 1);
    assert_eq!(public["my_vote"], 0);
    assert_eq!(f.get(Some(&bob)).await["my_vote"], -1);
    for invalid in [
        json!({"vote":2}),
        json!({"vote":0.5}),
        json!({"vote":1,"user_id":"bob"}),
    ] {
        assert!(
            !f.request(
                "PUT",
                "/api/jobs/job/vote",
                Some(&alice),
                true,
                Some(invalid)
            )
            .await
            .0
            .is_success()
        );
    }
}

#[tokio::test]
async fn comments_are_plain_text_private_by_default_and_author_deletable() {
    let f = Fixture::new();
    let alice = f.browser("alice");
    let bob = f.browser("bob");
    let (status, data) = f
        .request(
            "POST",
            "/api/jobs/job/comments",
            Some(&alice),
            true,
            Some(json!({"body":"  <script>alert('x')</script>\n讨论  "})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{data}");
    let comment = &data["comments"][0];
    let id = comment["id"].as_i64().unwrap();
    assert_eq!(comment["body"], "<script>alert('x')</script>\n讨论");
    assert_eq!(comment["author_name"], "会员");
    assert_eq!(comment["can_delete"], true);
    assert_eq!(f.get(None).await["comments"][0]["can_delete"], false);
    assert!(!data.to_string().contains("Private alice"));
    assert!(!data.to_string().contains("example.invalid"));
    let path = format!("/api/jobs/job/comments/{id}");
    assert_eq!(
        f.request("DELETE", &path, Some(&bob), true, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.request(
            "DELETE",
            &format!("/api/jobs/other/comments/{id}"),
            Some(&alice),
            true,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE users SET public_profile=1,name='Published name' WHERE id='alice'",
            [],
        )
        .unwrap();
    assert_eq!(
        f.get(None).await["comments"][0]["author_name"],
        "Published name"
    );
    assert_eq!(
        f.request("DELETE", &path, Some(&alice), true, None).await.0,
        StatusCode::OK
    );
    assert_eq!(f.get(None).await["total"], 0);
}

#[tokio::test]
async fn mutations_require_member_auth_csrf_and_visible_jobs() {
    let f = Fixture::new();
    let alice = f.browser("alice");
    for (method, path, body) in [
        ("PUT", "/api/jobs/job/vote", Some(json!({"vote":1}))),
        (
            "POST",
            "/api/jobs/job/comments",
            Some(json!({"body":"hello"})),
        ),
        ("DELETE", "/api/jobs/job/comments/1", None),
    ] {
        assert_eq!(
            f.request(method, path, None, true, body.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            f.request(method, path, Some(&alice), false, body).await.0,
            StatusCode::FORBIDDEN
        );
    }
    for (status, count) in [
        ("draft", 1),
        ("closed", 1),
        ("deleted", 0),
        ("published", 0),
    ] {
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE jobs SET status=?1,headcount=?2 WHERE id='job'",
                params![status, count],
            )
            .unwrap();
        for (method, path, body) in [
            ("GET", "/api/public/jobs/job/interactions", None),
            ("PUT", "/api/jobs/job/vote", Some(json!({"vote":1}))),
            (
                "POST",
                "/api/jobs/job/comments",
                Some(json!({"body":"hello"})),
            ),
            ("DELETE", "/api/jobs/job/comments/1", None),
        ] {
            assert_eq!(
                f.request(method, path, Some(&alice), true, body).await.0,
                StatusCode::NOT_FOUND
            );
        }
    }
    f.state.conn().unwrap().execute_batch("UPDATE jobs SET status='published',headcount=1 WHERE id='job'; UPDATE users SET banned=1 WHERE id='owner';").unwrap();
    assert_eq!(
        f.request(
            "GET",
            "/api/public/jobs/job/interactions",
            None,
            false,
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.request(
            "POST",
            "/api/jobs/job/comments",
            Some(&alice),
            true,
            Some(json!({"body":"hello"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    f.state.conn().unwrap().execute_batch("UPDATE users SET banned=0 WHERE id='owner'; UPDATE users SET banned=1 WHERE id='alice';").unwrap();
    assert_eq!(
        f.request(
            "PUT",
            "/api/jobs/job/vote",
            Some(&alice),
            true,
            Some(json!({"vote":1}))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn comment_validation_unicode_pagination_bans_and_rate_limits() {
    let f = Fixture::new();
    let alice = f.browser("alice");
    for body in [" \n ".to_string(), "a\0b".to_string(), "界".repeat(2001)] {
        assert_eq!(
            f.request(
                "POST",
                "/api/jobs/job/comments",
                Some(&alice),
                true,
                Some(json!({"body":body}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        f.request(
            "POST",
            "/api/jobs/job/comments",
            Some(&alice),
            true,
            Some(json!({"body":"界".repeat(2000)}))
        )
        .await
        .0,
        StatusCode::OK
    );
    for _ in 1..10 {
        assert_eq!(
            f.request(
                "POST",
                "/api/jobs/job/comments",
                Some(&alice),
                true,
                Some(json!({"body":"hello"}))
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        f.request(
            "POST",
            "/api/jobs/job/comments",
            Some(&alice),
            true,
            Some(json!({"body":"limited"}))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    {
        let db = f.state.conn().unwrap();
        for n in 0..15 {
            db.execute(
                "INSERT INTO job_comments(job_id,user_id,body,created_at) VALUES('job','bob',?1,1)",
                [format!("comment {n}")],
            )
            .unwrap();
        }
        db.execute("INSERT INTO job_votes VALUES('job','alice',1)", [])
            .unwrap();
    }
    let first = f.get(None).await;
    assert_eq!(first["total"], 25);
    assert_eq!(first["comments"].as_array().unwrap().len(), 20);
    let (status, last) = f
        .request(
            "GET",
            "/api/public/jobs/job/interactions?offset=20",
            None,
            false,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(last["comments"].as_array().unwrap().len(), 5);
    assert_ne!(first["comments"][19]["id"], last["comments"][0]["id"]);
    for offset in ["-1", "100001", "no"] {
        assert_eq!(
            f.request(
                "GET",
                &format!("/api/public/jobs/job/interactions?offset={offset}"),
                None,
                false,
                None
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id='alice'", [])
        .unwrap();
    let public = f.get(None).await;
    assert_eq!(public["total"], 15);
    assert_eq!(public["upvotes"], 0);
}

#[tokio::test]
async fn soft_delete_retains_interactions_and_permanent_delete_erases_them() {
    let f = Fixture::new();
    let alice = f.browser("alice");
    f.request(
        "PUT",
        "/api/jobs/job/vote",
        Some(&alice),
        true,
        Some(json!({"vote":1})),
    )
    .await;
    f.request(
        "POST",
        "/api/jobs/job/comments",
        Some(&alice),
        true,
        Some(json!({"body":"Discussion"})),
    )
    .await;
    let mut db = f.state.conn().unwrap();
    db.execute_batch("UPDATE users SET employer_paid=1 WHERE id='owner'; INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,created_at,paid_at) VALUES('publication','owner','publish','job',0,'free','paid',1,1);").unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    commerce::delete_job_record(&tx, "job", 0, false, "owner").unwrap();
    assert_eq!(
        tx.query_row("SELECT count(*) FROM job_comments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM job_votes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let restored = commerce::restore_job_record(&tx, "job", 1, "owner").unwrap();
    assert_eq!(restored.status, "published");
    assert_eq!(restored.headcount, 1);
    tx.commit().unwrap();
    drop(db);
    let restored = f.get(Some(&alice)).await;
    assert_eq!(restored["upvotes"], 1);
    assert_eq!(restored["my_vote"], 1);
    assert_eq!(restored["comments"][0]["body"], "Discussion");
    let mut db = f.state.conn().unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    commerce::delete_job_record(&tx, "job", 2, true, "owner").unwrap();
    assert_eq!(
        tx.query_row("SELECT count(*) FROM job_comments", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM job_votes", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    tx.commit().unwrap();
}

#[test]
fn version_eighteen_upgrade_is_atomic_idempotent_and_preserves_jobs() {
    let db = Connection::open_in_memory().unwrap();
    db::migrate(&db).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; INSERT INTO users(id,email,role,created_at,updated_at) VALUES('legacy-owner','legacy@example.invalid','employer',1,1); INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES('legacy-job','legacy-owner','Existing role','Existing description',300000,'published',1,1);").unwrap();
    db.execute_batch("DROP TABLE job_votes; DROP TABLE job_comments; PRAGMA user_version=18; CREATE TABLE job_comments(sentinel TEXT);").unwrap();
    assert!(db::migrate(&db).is_err());
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        18
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='job_votes'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TABLE job_comments;").unwrap();
    db::migrate(&db).unwrap();
    db::migrate(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        19
    );
    let job: (String, String, String) = db
        .query_row(
            "SELECT title,jd_md,status FROM jobs WHERE id='legacy-job'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        job,
        (
            "Existing role".into(),
            "Existing description".into(),
            "published".into()
        )
    );
    assert!(
        db.execute(
            "INSERT INTO job_votes VALUES('missing-job','legacy-owner',1)",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "INSERT INTO job_votes VALUES('legacy-job','legacy-owner',0)",
            []
        )
        .is_err()
    );
    assert!(db.execute("INSERT INTO job_comments(job_id,user_id,body,created_at) VALUES('legacy-job','legacy-owner','',1)", []).is_err());
    db.execute(
        "INSERT INTO job_votes VALUES('legacy-job','legacy-owner',1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO job_comments(job_id,user_id,body,created_at) VALUES('legacy-job','legacy-owner','Preserved discussion',1)", []).unwrap();
    db.execute("DELETE FROM users WHERE id='legacy-owner'", [])
        .unwrap_err();
    db.execute("DELETE FROM jobs WHERE id='legacy-job'", [])
        .unwrap();
    for table in ["job_votes", "job_comments"] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
