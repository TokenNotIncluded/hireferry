//! Authorization, isolation and transactional behavior of private mediation.
//! Fixtures never send mail, charge a payment provider or contact an external site.
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use hireferry::{
    AppState,
    access::Principal,
    auth,
    chat::{self, MandateInput, MessageInput, OutreachInput, SearchInput},
    config::Config,
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

struct Fixture {
    _dir: TempDir,
    state: AppState,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            development: true,
            database: dir
                .path()
                .join("chat.sqlite3")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        let state = AppState::new(cfg).unwrap();
        Self { _dir: dir, state }
    }
    fn user(&self, role: &str) -> Principal {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute("INSERT INTO users(id,email,name,role,employer_paid,visible,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,0,?6,?6)",params![id,format!("private-{id}@example.invalid"),format!("Member {id}"),role,role=="employer",hireferry::now()]).unwrap();
        Principal {
            user_id: Some(id.clone()),
            admin: false,
            via_ai: false,
            actor: id,
        }
    }
    fn admin(&self) -> Principal {
        Principal {
            user_id: None,
            admin: true,
            via_ai: false,
            actor: "admin".into(),
        }
    }
    fn admin_ai(&self) -> (Principal, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let raw = format!(
            "hf_admin_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.state.conn().unwrap().execute("INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES(?1,'chat test',?2,?3,?4)",params![id,auth::hash_token(&raw),hireferry::now(),hireferry::now()+3600]).unwrap();
        (
            Principal {
                user_id: None,
                admin: true,
                via_ai: true,
                actor: format!("admin_ai:{id}"),
            },
            raw,
        )
    }
    fn matched(&self, seeker: &Principal, employer: &Principal) -> (String, String) {
        let job = uuid::Uuid::new_v4().to_string();
        let matched = uuid::Uuid::new_v4().to_string();
        let db = self.state.conn().unwrap();
        db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES(?1,?2,'Private role','Confidential JD',300000,'published',?3,?3)",params![job,employer.user_id,hireferry::now()]).unwrap();
        db.execute("INSERT INTO matches(id,seeker_id,job_id,status,created_at,updated_at) VALUES(?1,?2,?3,'introduced',?4,?4)",params![matched,seeker.user_id,job,hireferry::now()]).unwrap();
        (matched, job)
    }
    fn browser(&self, p: &Principal) -> (String, String) {
        let raw = uuid::Uuid::new_v4().to_string();
        let csrf = uuid::Uuid::new_v4().to_string();
        self.state.conn().unwrap().execute("INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?1,?2,?3,0,?4,?5,?6)",params![auth::hash_token(&raw),p.user_id,p.admin,csrf,hireferry::now()+3600,hireferry::now()]).unwrap();
        (format!("hf_session={raw}"), csrf)
    }
}
fn uid(p: &Principal) -> &str {
    p.user_id.as_deref().unwrap()
}
fn platform(p: &Principal) -> String {
    format!("platform_{}", uid(p))
}
fn message(body: &str) -> MessageInput {
    MessageInput {
        body: body.into(),
        client_id: uuid::Uuid::new_v4().to_string(),
    }
}
fn send(f: &Fixture, p: &Principal, id: &str, body: &str) -> Value {
    chat::send_message(&f.state, p, id, message(body)).unwrap()
}
fn items(v: &Value) -> &Vec<Value> {
    v["items"].as_array().unwrap()
}
fn mandate(allowed: bool) -> MandateInput {
    MandateInput {
        target_industry: "Technology".into(),
        target_roles: "Rust developer".into(),
        salary_min_cents: Some(300_000),
        salary_max_cents: Some(600_000),
        location: "Remote".into(),
        preferences: "A small team".into(),
        status: "active".into(),
        allow_external: allowed,
    }
}
fn outreach(stage: &str) -> OutreachInput {
    OutreachInput {
        channel: "Public careers page".into(),
        opportunity: "A possible Rust role".into(),
        source_url: "https://example.invalid/careers/rust".into(),
        stage: stage.into(),
        note: "Manually recorded; no request was sent by this application.".into(),
    }
}

#[test]
fn platform_threads_are_unique_private_and_lazily_created_only_after_authorization() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let admin = f.admin();
    let first = chat::platform_conversation(&f.state, &seeker, None).unwrap();
    assert_eq!(first["id"], platform(&seeker));
    let again = chat::platform_conversation(&f.state, &admin, Some(uid(&seeker))).unwrap();
    assert_eq!(again["id"], first["id"]);
    let secret = send(
        &f,
        &seeker,
        &platform(&seeker),
        "Only my platform adviser may see this.",
    );
    let guessed = platform(&employer);
    assert_eq!(
        chat::messages(&f.state, &seeker, &guessed, 0, 100)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        chat::send_message(
            &f.state,
            &employer,
            &platform(&seeker),
            message("injection")
        )
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        chat::platform_conversation(&f.state, &seeker, Some(uid(&employer)))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let count: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row("SELECT count(*) FROM chat_conversations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "Unauthorized guessed IDs must not create rows");
    let employer_list = chat::list_conversations(&f.state, &employer).unwrap();
    assert_eq!(items(&employer_list).len(), 1);
    assert_eq!(employer_list["items"][0]["id"], guessed);
    assert!(items(&chat::updates(&f.state, &employer, 0, 100).unwrap()).is_empty());
    let admin_page = chat::messages(&f.state, &admin, &platform(&seeker), 0, 100).unwrap();
    assert_eq!(admin_page["items"][0]["id"], secret["id"]);
    assert!(!admin_page.to_string().contains("private-"));
    assert!(!first.to_string().contains("example.invalid"));
}

#[test]
fn matching_creates_a_separate_room_without_revealing_either_platform_history() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let stranger = f.user("seeker");
    send(
        &f,
        &seeker,
        &platform(&seeker),
        "seeker-platform-private-sentinel",
    );
    send(
        &f,
        &employer,
        &platform(&employer),
        "employer-platform-private-sentinel",
    );
    let (matched, _) = f.matched(&seeker, &employer);
    let id = format!("match_{matched}");
    let fresh = chat::messages(&f.state, &seeker, &id, -1, 100).unwrap();
    assert!(items(&fresh).is_empty());
    let intro = send(
        &f,
        &employer,
        &id,
        "Hello, let us discuss the introduced role.",
    );
    let response = chat::messages(&f.state, &seeker, &id, 0, 100).unwrap();
    assert_eq!(items(&response).len(), 1);
    assert_eq!(response["items"][0]["id"], intro["id"]);
    assert!(!response.to_string().contains("platform-private-sentinel"));
    assert_eq!(
        chat::messages(&f.state, &stranger, &id, 0, 100)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let updates = chat::updates(&f.state, &seeker, 0, 100).unwrap();
    assert!(
        !updates
            .to_string()
            .contains("employer-platform-private-sentinel")
    );
    assert!(
        updates
            .to_string()
            .contains("seeker-platform-private-sentinel")
    );
    let listed = chat::list_conversations(&f.state, &seeker).unwrap();
    let room = items(&listed).iter().find(|v| v["id"] == id).unwrap();
    assert_eq!(room["writable"], true);
    assert_eq!(room["participants"].as_array().unwrap().len(), 2);
}

#[test]
fn revoked_matches_disappear_for_members_but_administrators_keep_read_only_history() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let admin = f.admin();
    let (matched, job) = f.matched(&seeker, &employer);
    let id = format!("match_{matched}");
    let original = send(&f, &seeker, &id, "Historical introduction");
    for status in ["introduced", "interview", "hired"] {
        f.state
            .conn()
            .unwrap()
            .execute(
                "UPDATE matches SET status=?2 WHERE id=?1",
                params![matched, status],
            )
            .unwrap();
        f.state
            .conn()
            .unwrap()
            .execute("UPDATE jobs SET status='closed' WHERE id=?1", [&job])
            .unwrap();
        assert!(chat::messages(&f.state, &employer, &id, 0, 100).is_ok());
        send(
            &f,
            &employer,
            &id,
            "A closed advertisement does not end an existing conversation.",
        );
    }
    for reason in ["declined", "deleted", "banned"] {
        {
            let db = f.state.conn().unwrap();
            db.execute(
                "UPDATE matches SET status=?2 WHERE id=?1",
                params![
                    matched,
                    if reason == "declined" {
                        "declined"
                    } else {
                        "introduced"
                    }
                ],
            )
            .unwrap();
            db.execute(
                "UPDATE jobs SET status=?2 WHERE id=?1",
                params![
                    job,
                    if reason == "deleted" {
                        "deleted"
                    } else {
                        "closed"
                    }
                ],
            )
            .unwrap();
            db.execute(
                "UPDATE users SET banned=?2 WHERE id=?1",
                params![uid(&employer), reason == "banned"],
            )
            .unwrap();
        }
        assert_eq!(
            chat::messages(&f.state, &seeker, &id, 0, 100)
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            chat::send_message(&f.state, &seeker, &id, message("late message"))
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            chat::mark_read(&f.state, &seeker, &id, original["id"].as_i64().unwrap())
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert!(items(&chat::updates(&f.state, &seeker, 0, 100).unwrap()).is_empty());
        assert!(
            items(&chat::list_conversations(&f.state, &seeker).unwrap())
                .iter()
                .all(|v| v["id"] != id)
        );
        assert!(!items(&chat::messages(&f.state, &admin, &id, 0, 100).unwrap()).is_empty());
        let list = chat::list_conversations(&f.state, &admin).unwrap();
        assert_eq!(
            items(&list).iter().find(|v| v["id"] == id).unwrap()["writable"],
            false
        );
        assert_eq!(
            chat::send_message(&f.state, &admin, &id, message("not allowed on archive"))
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
    }
}

#[test]
fn messages_are_idempotent_per_sender_and_body_rendering_rejects_active_markup() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let admin = f.admin();
    let id = platform(&seeker);
    let input=MessageInput{body:"# Hello\n<script>alert(1)</script>\n![tracker](https://example.invalid/track)\n[unsafe](javascript:alert(1))".into(),client_id:"same-client-id".into()};
    let first = chat::send_message(&f.state, &seeker, &id, input.clone()).unwrap();
    let second = chat::send_message(&f.state, &seeker, &id, input.clone()).unwrap();
    assert_eq!(first["id"], second["id"]);
    let html = first["body_html"].as_str().unwrap();
    assert!(html.contains("<h1>"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("<img"));
    assert!(!html.contains("javascript:"));
    let changed = MessageInput {
        body: "A different payload".into(),
        client_id: input.client_id.clone(),
    };
    assert_eq!(
        chat::send_message(&f.state, &seeker, &id, changed)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    let platform_reply = chat::send_message(&f.state, &admin, &id, input).unwrap();
    assert_ne!(platform_reply["id"], first["id"]);
    assert_eq!(platform_reply["sender_kind"], "platform");
    assert!(platform_reply["sender_id"].is_null());
    assert_eq!(first["sender_id"], uid(&seeker));
    assert_eq!(first["via_ai"], false);
    let unicode = "聘".repeat(10000);
    assert!(chat::send_message(&f.state, &seeker, &id, message(&unicode)).is_ok());
    assert_eq!(
        chat::send_message(&f.state, &seeker, &id, message(&(unicode + "渡")))
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert!(chat::send_message(&f.state, &seeker, &id, message(" \n\t ")).is_err());
    assert!(
        chat::send_message(
            &f.state,
            &seeker,
            &id,
            MessageInput {
                body: "x".into(),
                client_id: "x".repeat(81)
            }
        )
        .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_retries_across_connections_write_one_message_and_one_audit_event() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let id = platform(&seeker);
    chat::platform_conversation(&f.state, &seeker, None).unwrap();
    let states = (0..4)
        .map(|_| AppState::new((*f.state.cfg).clone()).unwrap())
        .collect::<Vec<_>>();
    let barrier = Arc::new(tokio::sync::Barrier::new(states.len()));
    let mut tasks = Vec::new();
    for state in states {
        let barrier = barrier.clone();
        let p = seeker.clone();
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            chat::send_message(
                &state,
                &p,
                &id,
                MessageInput {
                    body: "One delivery".into(),
                    client_id: "concurrent-attempt".into(),
                },
            )
            .unwrap()
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap()["id"].as_i64().unwrap());
    }
    assert!(ids.iter().all(|id| id == &ids[0]));
    let db = f.state.conn().unwrap();
    let messages: i64 = db
        .query_row("SELECT count(*) FROM chat_messages", [], |r| r.get(0))
        .unwrap();
    let audit: i64 = db
        .query_row(
            "SELECT count(*) FROM audit WHERE action='chat_message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(messages, 1);
    assert_eq!(audit, 1);
}

#[test]
fn cursors_are_incremental_scoped_and_bounded_while_latest_and_older_history_remain_accessible() {
    let f = Fixture::new();
    let owner = f.user("seeker");
    let outsider = f.user("seeker");
    let id = platform(&owner);
    let mut expected = Vec::new();
    for n in 0..105 {
        expected.push(
            send(&f, &owner, &id, &format!("Visible {n}"))["id"]
                .as_i64()
                .unwrap(),
        );
        if n % 10 == 0 {
            send(
                &f,
                &outsider,
                &platform(&outsider),
                "foreign-private-sentinel",
            );
        }
    }
    let latest = chat::messages(&f.state, &owner, &id, -1, 1000).unwrap();
    assert_eq!(items(&latest).len(), 100);
    assert_eq!(latest["has_older"], true);
    assert_eq!(latest["has_more"], false);
    assert_eq!(latest["first_cursor"], expected[5]);
    assert_eq!(latest["next_cursor"], expected[104]);
    let older = chat::messages_before(&f.state, &owner, &id, expected[5], 100).unwrap();
    assert_eq!(items(&older).len(), 5);
    assert_eq!(older["has_older"], false);
    let first = chat::messages(&f.state, &owner, &id, 0, 100).unwrap();
    assert_eq!(first["has_more"], true);
    assert_eq!(first["next_cursor"], expected[99]);
    let final_page = chat::messages(&f.state, &owner, &id, expected[99], 100).unwrap();
    assert_eq!(items(&final_page).len(), 5);
    let changes = chat::updates(&f.state, &owner, 0, 1000).unwrap();
    assert_eq!(items(&changes).len(), 100);
    assert!(!changes.to_string().contains("foreign-private-sentinel"));
    assert_eq!(changes["next_cursor"], expected[99]);
    let tail = chat::updates(&f.state, &owner, expected[99], 100).unwrap();
    assert_eq!(items(&tail).len(), 5);
    assert_eq!(tail["has_more"], false);
    let empty = chat::updates(&f.state, &owner, expected[104], 100).unwrap();
    assert!(items(&empty).is_empty());
    assert_eq!(empty["next_cursor"], expected[104]);
    assert!(chat::updates(&f.state, &owner, -1, 100).is_err());
    assert!(chat::messages(&f.state, &owner, &id, 0, 0).is_err());
}

#[test]
fn read_markers_are_monotonic_thread_scoped_and_shared_by_platform_ai_identities() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let other = f.user("seeker");
    let admin = f.admin();
    let id = platform(&seeker);
    let (ai, _) = f.admin_ai();
    let (second_ai, _) = f.admin_ai();
    let first = send(&f, &seeker, &id, "First")["id"].as_i64().unwrap();
    let second = send(&f, &seeker, &id, "Second")["id"].as_i64().unwrap();
    assert_eq!(
        chat::platform_conversation(&f.state, &admin, Some(uid(&seeker))).unwrap()["unread_count"],
        2
    );
    chat::mark_read(&f.state, &ai, &id, first).unwrap();
    assert_eq!(
        chat::platform_conversation(&f.state, &second_ai, Some(uid(&seeker))).unwrap()["unread_count"],
        1
    );
    chat::mark_read(&f.state, &admin, &id, second).unwrap();
    let backwards = chat::mark_read(&f.state, &second_ai, &id, first).unwrap();
    assert_eq!(backwards["message_id"], second);
    assert_eq!(backwards["unread_count"], 0);
    let reply = send(
        &f,
        &ai,
        &id,
        "Platform AI draft delivered under delegated access",
    );
    assert_eq!(reply["sender_kind"], "platform");
    assert_eq!(reply["via_ai"], true);
    assert_eq!(
        chat::platform_conversation(&f.state, &seeker, None).unwrap()["unread_count"],
        1
    );
    let foreign = send(&f, &other, &platform(&other), "Foreign marker")["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        chat::mark_read(&f.state, &seeker, &id, foreign)
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    chat::mark_read(&f.state, &seeker, &id, reply["id"].as_i64().unwrap()).unwrap();
    assert_eq!(
        chat::platform_conversation(&f.state, &seeker, None).unwrap()["unread_count"],
        0
    );
    let token_id = ai.actor.strip_prefix("admin_ai:").unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE admin_api_tokens SET revoked=1 WHERE id=?1",
            [token_id],
        )
        .unwrap();
    assert!(chat::send_message(&f.state, &ai, &id, message("Revoked token")).is_err());
    assert!(chat::messages(&f.state, &ai, &id, 0, 100).is_err());
}

#[test]
fn mandates_require_an_explicit_seeker_action_and_admin_cannot_expand_consent_or_resume() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let other = f.user("seeker");
    let admin = f.admin();
    assert!(chat::get_mandate(&f.state, &seeker, None).unwrap()["mandate"].is_null());
    assert_eq!(
        chat::save_mandate(&f.state, &admin, Some(uid(&seeker)), mandate(false))
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        chat::get_mandate(&f.state, &other, Some(uid(&seeker)))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let initial = chat::save_mandate(&f.state, &seeker, None, mandate(false)).unwrap();
    assert_eq!(initial["mandate"]["allow_external"], false);
    assert!(initial["mandate"]["consented_at"].is_null());
    assert_eq!(
        chat::save_mandate(&f.state, &admin, Some(uid(&seeker)), mandate(true))
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    let mut paused = mandate(false);
    paused.status = "paused".into();
    chat::save_mandate(&f.state, &admin, Some(uid(&seeker)), paused).unwrap();
    assert_eq!(
        chat::save_mandate(&f.state, &admin, Some(uid(&seeker)), mandate(false))
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    let mut seeker_ai = seeker.clone();
    seeker_ai.via_ai = true;
    let authorized = chat::save_mandate(&f.state, &seeker_ai, None, mandate(true)).unwrap();
    assert_eq!(authorized["mandate"]["status"], "active");
    assert_eq!(authorized["mandate"]["allow_external"], true);
    assert_eq!(authorized["mandate"]["via_ai"], true);
    assert_eq!(authorized["mandate"]["consented_via_ai"], true);
    assert!(authorized["mandate"]["consented_at"].as_i64().unwrap() > 0);
    let mut bad = mandate(true);
    bad.salary_min_cents = Some(700_000);
    bad.salary_max_cents = Some(600_000);
    assert_eq!(
        chat::save_mandate(&f.state, &seeker, None, bad)
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    let revoked = chat::save_mandate(&f.state, &seeker, None, mandate(false)).unwrap();
    assert!(revoked["mandate"]["consented_at"].is_null());
    assert_eq!(
        chat::list_mandates(&f.state, &seeker, SearchInput::default())
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    let found = chat::list_mandates(
        &f.state,
        &admin,
        SearchInput {
            q: Some("Rust".into()),
            status: Some("active".into()),
            page: Some(1),
        },
    )
    .unwrap();
    assert_eq!(found["total"], 1);
    assert!(!found.to_string().contains("private-"));
}

#[test]
fn external_progress_is_manual_scoped_and_requires_live_consent_with_retained_admin_history() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let other = f.user("seeker");
    let admin = f.admin();
    assert!(items(&chat::list_outreach(&f.state, &seeker, None).unwrap()).is_empty());
    assert_eq!(
        chat::save_outreach(&f.state, &admin, uid(&seeker), None, outreach("identified"))
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    chat::save_mandate(&f.state, &seeker, None, mandate(true)).unwrap();
    assert_eq!(
        chat::save_outreach(
            &f.state,
            &seeker,
            uid(&seeker),
            None,
            outreach("identified")
        )
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
    let record =
        chat::save_outreach(&f.state, &admin, uid(&seeker), None, outreach("identified")).unwrap();
    let id = record["id"].as_str().unwrap();
    assert_eq!(record["stage"], "identified");
    assert_eq!(
        chat::list_outreach(&f.state, &seeker, None).unwrap()["items"][0]["id"],
        id
    );
    assert_eq!(
        chat::list_outreach(&f.state, &other, Some(uid(&seeker)))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let mut unsafe_url = outreach("contacted");
    unsafe_url.source_url = "javascript:alert(1)".into();
    assert_eq!(
        chat::save_outreach(&f.state, &admin, uid(&seeker), Some(id), unsafe_url)
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        chat::save_outreach(&f.state, &admin, uid(&other), Some(id), outreach("closed"))
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let mut paused = mandate(true);
    paused.status = "paused".into();
    chat::save_mandate(&f.state, &seeker, None, paused).unwrap();
    assert_eq!(
        chat::save_outreach(&f.state, &admin, uid(&seeker), None, outreach("identified"))
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        chat::save_outreach(
            &f.state,
            &admin,
            uid(&seeker),
            Some(id),
            outreach("contacted")
        )
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN
    );
    chat::save_outreach(&f.state, &admin, uid(&seeker), Some(id), outreach("closed")).unwrap();
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [uid(&seeker)])
        .unwrap();
    assert_eq!(
        chat::get_mandate(&f.state, &admin, Some(uid(&seeker))).unwrap()["mandate"]["banned"],
        true
    );
    assert_eq!(
        chat::list_mandates(&f.state, &admin, SearchInput::default()).unwrap()["items"][0]["banned"],
        true
    );
    assert_eq!(
        chat::list_outreach(&f.state, &admin, Some(uid(&seeker))).unwrap()["items"][0]["banned"],
        true
    );
    assert!(chat::list_outreach(&f.state, &seeker, None).is_err());
    assert!(chat::save_mandate(&f.state, &admin, Some(uid(&seeker)), mandate(true)).is_err());
    assert!(
        chat::save_outreach(&f.state, &admin, uid(&seeker), Some(id), outreach("closed")).is_ok()
    );
}

async fn request(
    f: &Fixture,
    method: &str,
    path: &str,
    payload: Option<Value>,
    cookie: Option<&str>,
    csrf: Option<&str>,
    bearer: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("origin", "http://localhost:3000");
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(csrf) = csrf {
        builder = builder.header("x-csrf-token", csrf);
    }
    if let Some(bearer) = bearer {
        builder = builder.header("authorization", format!("Bearer {bearer}"));
    }
    let body = if let Some(payload) = payload {
        builder = builder.header("content-type", "application/json");
        Body::from(payload.to_string())
    } else {
        Body::empty()
    };
    let response = hireferry::app(f.state.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn http_writes_require_csrf_and_never_accept_forged_sender_or_ai_fields() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let id = platform(&seeker);
    let (cookie, csrf) = f.browser(&seeker);
    let (_, admin_token) = f.admin_ai();
    let path = format!("/api/chat/conversations/{id}/messages");
    assert_eq!(
        request(&f, "GET", "/api/chat/conversations", None, None, None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &f,
            "POST",
            &path,
            Some(json!({"body":"Hello","client_id":"browser-id"})),
            Some(&cookie),
            None,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let spoof=request(&f,"POST",&path,Some(json!({"body":"Forged platform sender","client_id":"spoof","sender_kind":"platform","via_ai":true})),Some(&cookie),Some(&csrf),None).await;
    assert!(spoof.0.is_client_error());
    let user = request(
        &f,
        "POST",
        &path,
        Some(json!({"body":"Hello","client_id":"browser-id"})),
        Some(&cookie),
        Some(&csrf),
        None,
    )
    .await;
    assert_eq!(user.0, StatusCode::OK);
    assert_eq!(user.1["sender_kind"], "user");
    assert_eq!(user.1["via_ai"], false);
    let ai = request(
        &f,
        "POST",
        &path,
        Some(json!({"body":"Platform reply","client_id":"admin-ai-id"})),
        None,
        None,
        Some(&admin_token),
    )
    .await;
    assert_eq!(ai.0, StatusCode::OK);
    assert_eq!(ai.1["sender_kind"], "platform");
    assert_eq!(ai.1["via_ai"], true);
    let latest = request(
        &f,
        "GET",
        &format!("{path}?limit=1"),
        None,
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(latest.0, StatusCode::OK);
    assert_eq!(latest.1["items"][0]["id"], ai.1["id"]);
    assert_eq!(latest.1["has_older"], true);
    let older = request(
        &f,
        "GET",
        &format!("{path}?before={}&limit=1", ai.1["id"]),
        None,
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(older.0, StatusCode::OK);
    assert_eq!(older.1["items"][0]["id"], user.1["id"]);
    let bad_cursor = request(
        &f,
        "GET",
        &format!("{path}?after=0&before=5"),
        None,
        Some(&cookie),
        None,
        None,
    )
    .await;
    assert_eq!(bad_cursor.0, StatusCode::BAD_REQUEST);
    let read_path = format!("/api/chat/conversations/{id}/read");
    assert_eq!(
        request(
            &f,
            "POST",
            &read_path,
            Some(json!({"message_id":ai.1["id"]})),
            Some(&cookie),
            None,
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &f,
            "POST",
            &read_path,
            Some(json!({"message_id":ai.1["id"]})),
            Some(&cookie),
            Some(&csrf),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn webmcp_cookie_messages_attribute_ai_and_recheck_revoked_admin_sessions() {
    fn webmcp_headers(cookie: &str, csrf: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("cookie", cookie.parse().unwrap());
        headers.insert("x-csrf-token", csrf.parse().unwrap());
        headers.insert("x-hireferry-ai", "webmcp".parse().unwrap());
        headers.insert("origin", "http://localhost:3000".parse().unwrap());
        headers
    }

    async fn post(
        f: &Fixture,
        path: &str,
        headers: &axum::http::HeaderMap,
        payload: Value,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap();
        request.headers_mut().extend(headers.clone());
        let response = hireferry::app(f.state.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    let f = Fixture::new();
    let seeker = f.user("seeker");
    let stranger = f.user("seeker");
    let id = platform(&seeker);
    let path = format!("/api/chat/conversations/{id}/messages");
    let (seeker_cookie, seeker_csrf) = f.browser(&seeker);
    let seeker_headers = webmcp_headers(&seeker_cookie, &seeker_csrf);
    let user_message = post(
        &f,
        &path,
        &seeker_headers,
        json!({"body":"My browser assistant asks about this role.","client_id":"webmcp-seeker"}),
    )
    .await;
    assert_eq!(user_message.0, StatusCode::OK);
    assert_eq!(user_message.1["via_ai"], true);
    assert_eq!(user_message.1["sender_kind"], "user");
    assert_eq!(user_message.1["sender_id"], uid(&seeker));

    for (field, value) in [
        ("sender_kind", json!("platform")),
        ("sender_id", json!(uid(&stranger))),
        ("sender_actor", json!("admin")),
        ("via_ai", json!(false)),
    ] {
        let mut forged = json!({"body":"Forged identity","client_id":format!("spoof-{field}")});
        forged[field] = value;
        let rejected = post(&f, &path, &seeker_headers, forged).await;
        assert!(rejected.0.is_client_error(), "Accepted forged {field}");
    }

    let (admin_cookie, admin_csrf) = f.browser(&f.admin());
    let admin_headers = webmcp_headers(&admin_cookie, &admin_csrf);
    let captured = hireferry::access::principal(&f.state, &admin_headers, true).unwrap();
    let session_hash = auth::hash_token(admin_cookie.strip_prefix("hf_session=").unwrap());
    assert!(captured.admin && captured.via_ai);
    assert!(captured.user_id.is_none());
    assert_eq!(captured.actor, format!("admin_webmcp:{session_hash}"));

    let admin_message = post(
        &f,
        &path,
        &admin_headers,
        json!({"body":"Platform assistant reply.","client_id":"webmcp-admin"}),
    )
    .await;
    assert_eq!(admin_message.0, StatusCode::OK);
    assert_eq!(admin_message.1["via_ai"], true);
    assert_eq!(admin_message.1["sender_kind"], "platform");
    assert!(admin_message.1["sender_id"].is_null());
    {
        let db = f.state.conn().unwrap();
        let (actor, detail): (String, String) = db
            .query_row(
                "SELECT actor,detail FROM audit WHERE action='chat_message' AND target=?1 ORDER BY id DESC LIMIT 1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(actor, captured.actor);
        let detail: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(detail["message_id"], admin_message.1["id"]);
        assert_eq!(detail["via_ai"], true);
        assert_eq!(
            db.execute("DELETE FROM sessions WHERE token_hash=?1", [&session_hash])
                .unwrap(),
            1
        );
        assert_eq!(
            hireferry::access::ensure_admin(&db, &captured)
                .unwrap_err()
                .status,
            StatusCode::UNAUTHORIZED
        );
    }

    // A previously authenticated AI principal must not outlive its browser session.
    let stale = chat::send_message(&f.state, &captured, &id, message("After revocation"));
    assert_eq!(stale.unwrap_err().status, StatusCode::UNAUTHORIZED);
    let stale_http = post(
        &f,
        &path,
        &admin_headers,
        json!({"body":"After revocation via HTTP","client_id":"revoked-webmcp"}),
    )
    .await;
    assert_eq!(stale_http.0, StatusCode::UNAUTHORIZED);
    let db = f.state.conn().unwrap();
    let message_count: i64 = db
        .query_row(
            "SELECT count(*) FROM chat_messages WHERE conversation_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let audit_count: i64 = db
        .query_row(
            "SELECT count(*) FROM audit WHERE action='chat_message' AND target=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        message_count, 2,
        "Rejected requests must not persist messages"
    );
    assert_eq!(
        audit_count, 2,
        "Rejected requests must not persist delivery audits"
    );
}
