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
        Self::with_config(Config::default())
    }
    fn with_config(mut cfg: Config) -> Self {
        let dir = tempfile::tempdir().unwrap();
        cfg.development = true;
        cfg.database = dir
            .path()
            .join("chat.sqlite3")
            .to_string_lossy()
            .into_owned();
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
        salary_currency: None,
        location: "Remote".into(),
        preferences: "A small team".into(),
        status: "active".into(),
        allow_external: allowed,
    }
}

#[test]
fn mandate_salary_keeps_its_currency_and_uses_config_default_for_new_records() {
    let mut cfg = Config {
        development: true,
        database: ":memory:".into(),
        ..Default::default()
    };
    cfg.pricing.currency = "JPY".into();
    let s = AppState::new(cfg).unwrap();
    s.conn().unwrap().execute("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('seeker','yen@example.test','seeker',1,1)", []).unwrap();
    let principal = Principal {
        user_id: Some("seeker".into()),
        admin: false,
        via_ai: false,
        actor: "seeker".into(),
    };
    let first = chat::save_mandate(&s, &principal, None, mandate(false)).unwrap();
    assert_eq!(first["mandate"]["salary_currency"], "JPY");
    assert_eq!(first["mandate"]["currency_decimals"], 0);
    assert_eq!(first["mandate"]["salary_min_cents"], 300000);
    // A historical CNY mandate must not be reinterpreted as JPY when the
    // operator changes the default and a legacy client omits the field.
    s.conn()
        .unwrap()
        .execute(
            "UPDATE mandates SET salary_currency='CNY' WHERE user_id='seeker'",
            [],
        )
        .unwrap();
    let preserved = chat::save_mandate(&s, &principal, None, mandate(false)).unwrap();
    assert_eq!(preserved["mandate"]["salary_currency"], "CNY");
    let mut disallowed = mandate(false);
    disallowed.salary_currency = Some("USD".into());
    assert!(chat::save_mandate(&s, &principal, None, disallowed).is_err());
    let mut explicit = mandate(false);
    explicit.salary_currency = Some("JPY".into());
    assert_eq!(
        chat::save_mandate(&s, &principal, None, explicit).unwrap()["mandate"]["salary_currency"],
        "JPY"
    );
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
fn recent_record_capacity_returns_latest_items_without_hiding_known_conversations() {
    let mut cfg = Config::default();
    cfg.limits.recent_list_limit = 2;
    let f = Fixture::with_config(cfg);
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let mut match_ids = Vec::new();
    for n in 1..=3 {
        let (matched, job) = f.matched(&seeker, &employer);
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE matches SET updated_at=?1 WHERE id=?2",
            params![n, matched],
        )
        .unwrap();
        db.execute("INSERT INTO chat_conversations(id,kind,match_id,created_at,updated_at) VALUES(?1,'matched',?2,?3,?3)", params![format!("match_{matched}"), matched, n]).unwrap();
        db.execute("INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,headcount,status,delete_count,deleted_from_status,deleted_headcount,created_at,updated_at) VALUES(?1,?2,'Deleted role','Private JD',300000,0,'deleted',1,'published',1,?3,?3)", params![format!("deleted-{n}"), uid(&employer), n]).unwrap();
        db.execute("INSERT INTO orders(id,user_id,kind,job_id,amount_cents,provider,status,created_at,paid_at) VALUES(?1,?2,'publish',?3,30000,'stripe','paid',?4,?4)", params![format!("order-{n}"), uid(&employer), job, n]).unwrap();
        db.execute("INSERT INTO outreach(id,user_id,channel,opportunity,source_url,stage,note,created_by,updated_by,created_at,updated_at) VALUES(?1,?2,'Public careers','Private opportunity','https://example.invalid/role','identified','','admin','admin',?3,?3)", params![format!("outreach-{n}"), uid(&seeker), n]).unwrap();
        match_ids.push(matched);
    }
    // Keep the platform room older than the three synthetic matched rooms.
    f.state.conn().unwrap().execute("INSERT INTO chat_conversations(id,kind,user_id,created_at,updated_at) VALUES(?1,'platform',?2,0,0)", params![platform(&seeker), uid(&seeker)]).unwrap();
    let page_ids = |page: Value| -> Vec<String> {
        items(&page)
            .iter()
            .map(|item| item["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        page_ids(hireferry::api::deleted_jobs(&f.state, uid(&employer)).unwrap()),
        vec!["deleted-3", "deleted-2"]
    );
    assert_eq!(
        hireferry::api::user_orders(&f.state, uid(&employer))
            .unwrap()
            .into_iter()
            .map(|order| order.id)
            .collect::<Vec<_>>(),
        vec!["order-3", "order-2"]
    );
    assert_eq!(
        page_ids(hireferry::api::list_matches(&f.state, Some(uid(&employer))).unwrap()),
        vec![match_ids[2].clone(), match_ids[1].clone()]
    );
    assert_eq!(
        page_ids(chat::list_outreach(&f.state, &seeker, None).unwrap()),
        vec!["outreach-3", "outreach-2"]
    );
    assert_eq!(
        page_ids(chat::list_conversations(&f.state, &seeker).unwrap()),
        vec![
            format!("match_{}", match_ids[2]),
            format!("match_{}", match_ids[1])
        ]
    );
    let oldest = format!("match_{}", match_ids[0]);
    assert_eq!(
        chat::get_conversation(&f.state, &seeker, &oldest).unwrap()["id"],
        oldest
    );
}

#[test]
fn batched_conversations_match_single_room_metadata_for_each_reader() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let stranger = f.user("seeker");
    let admin = f.admin();
    let (matched, _) = f.matched(&seeker, &employer);
    let (other_match, _) = f.matched(&stranger, &employer);
    let room = format!("match_{matched}");
    let other_room = format!("match_{other_match}");
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE users SET name='求职者',avatar='stored-avatar' WHERE id=?1",
            [uid(&seeker)],
        )
        .unwrap();
        db.execute("UPDATE users SET name='' WHERE id=?1", [uid(&employer)])
            .unwrap();
    }
    send(&f, &seeker, &platform(&seeker), "Private question");
    send(&f, &admin, &platform(&seeker), "Private answer");
    send(&f, &seeker, &room, "Shared question");
    let reply = send(&f, &employer, &room, "Shared answer");
    let latest = send(&f, &admin, &room, &"聘渡🛶".repeat(80));
    chat::mark_read(&f.state, &seeker, &room, reply["id"].as_i64().unwrap()).unwrap();
    // Include an empty match and private room, and deterministic timestamp ties.
    for reader in [&seeker, &employer, &stranger, &admin] {
        chat::list_conversations(&f.state, reader).unwrap();
    }
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE chat_conversations SET updated_at=100", [])
        .unwrap();
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE chat_conversations SET updated_at=200 WHERE id=?1",
            [&room],
        )
        .unwrap();
    for reader in [&seeker, &employer, &stranger, &admin] {
        let page = chat::list_conversations(&f.state, reader).unwrap();
        let rows = items(&page);
        for row in rows {
            assert_eq!(
                *row,
                chat::get_conversation(&f.state, reader, row["id"].as_str().unwrap()).unwrap()
            );
        }
        let mut expected = rows
            .iter()
            .map(|row| {
                (
                    row["updated_at"].as_i64().unwrap(),
                    row["id"].as_str().unwrap(),
                )
            })
            .collect::<Vec<_>>();
        expected.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        assert_eq!(
            rows.iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected.iter().map(|(_, id)| *id).collect::<Vec<_>>()
        );
        if reader.user_id == seeker.user_id {
            assert!(!rows.iter().any(|row| row["id"] == other_room));
            let shared = rows.iter().find(|row| row["id"] == room).unwrap();
            assert_eq!(shared["unread_count"], 1);
            assert_eq!(shared["last_message_id"], latest["id"]);
            assert_eq!(
                shared["last_message_preview"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .count(),
                180
            );
            assert_eq!(
                shared["participants"][0]["avatar"],
                format!("/api/avatars/{}", uid(&seeker))
            );
        }
    }
    let empty = chat::get_conversation(&f.state, &admin, &other_room).unwrap();
    assert_eq!(empty["last_message_id"], 0);
    assert_eq!(empty["last_message_preview"], "");
    assert!(empty["last_message_at"].is_null());
    assert_eq!(empty["unread_count"], 0);

    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [uid(&seeker)])
        .unwrap();
    assert!(chat::list_conversations(&f.state, &seeker).is_err());
    let admin_page = chat::list_conversations(&f.state, &admin).unwrap();
    for id in [platform(&seeker), room] {
        let archived = items(&admin_page)
            .iter()
            .find(|row| row["id"] == id)
            .unwrap();
        assert_eq!(archived["writable"], false);
        assert_eq!(
            *archived,
            chat::get_conversation(&f.state, &admin, &id).unwrap()
        );
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

#[tokio::test]
async fn platform_ai_acknowledges_messages_asynchronously_with_workspace_links() {
    let mut cfg = Config::default();
    cfg.chat.auto_reply.delay_seconds = 0;
    let f = Fixture::with_config(cfg);
    let seeker = f.user("seeker");
    let admin = f.admin();
    let room = chat::platform_conversation(&f.state, &seeker, None).unwrap();
    let _ = send(
        &f,
        &seeker,
        room["id"].as_str().unwrap(),
        "我想找 Rust 后端岗位",
    );
    let pending: i64 = f
        .state
        .conn()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM chat_ai_jobs WHERE status='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
    assert!(hireferry::chat_ai::run_once(&f.state).await.unwrap());
    let page = chat::messages(&f.state, &admin, room["id"].as_str().unwrap(), 0, 100).unwrap();
    let reply = items(&page)
        .iter()
        .find(|item| item["via_ai"] == true)
        .expect("the async platform acknowledgement should be visible");
    assert_eq!(reply["format"], "openui");
    assert!(reply["body"].as_str().unwrap().contains("不是时刻在线"));
    assert!(reply["body"].as_str().unwrap().contains("app?tab=profile"));
    assert_eq!(
        f.state
            .conn()
            .unwrap()
            .query_row("SELECT status FROM chat_ai_jobs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "complete"
    );
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
            items(&list).iter().find(|v| v["id"] == id).unwrap(),
            &chat::get_conversation(&f.state, &admin, &id).unwrap()
        );
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
    assert_eq!(platform_reply["is_own"], true);
    assert_eq!(platform_reply["staff_name"], "超级管理员");
    assert_eq!(first["sender_id"], uid(&seeker));
    assert_eq!(first["via_ai"], false);
    for response in [&first, &second] {
        assert!(response.get("is_own").is_none());
        assert!(response.get("staff_name").is_none());
    }
    let history = chat::messages(&f.state, &seeker, &id, -1, 100).unwrap();
    for response in items(&history) {
        assert!(response.get("is_own").is_none());
        assert!(response.get("staff_name").is_none());
    }
    // Compare the complete customer-visible message, including sanitized HTML,
    // without importing metadata from the administrator's sending response.
    let mut public_reply = platform_reply.clone();
    public_reply.as_object_mut().unwrap().remove("is_own");
    public_reply.as_object_mut().unwrap().remove("staff_name");
    assert_eq!(items(&history), &vec![first.clone(), public_reply]);
    let earlier = chat::messages_before(
        &f.state,
        &admin,
        &id,
        platform_reply["id"].as_i64().unwrap(),
        100,
    )
    .unwrap();
    let mut customer_from_admin_view = first.clone();
    customer_from_admin_view["is_own"] = json!(false);
    customer_from_admin_view["staff_name"] = Value::Null;
    assert_eq!(items(&earlier), &vec![customer_from_admin_view]);
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
fn polling_reads_existing_messages_without_materializing_empty_match_rooms() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let admin = f.admin();
    let (matched, _) = f.matched(&seeker, &employer);
    for p in [&admin, &seeker, &employer] {
        assert!(items(&chat::updates(&f.state, p, 0, 100).unwrap()).is_empty());
    }
    assert_eq!(
        f.state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM chat_conversations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "Empty message polling must not materialize the matching history"
    );
    let room = format!("match_{matched}");
    let sent = send(
        &f,
        &seeker,
        &room,
        "Shared room is created when it is needed",
    );
    let updates = chat::updates(&f.state, &employer, 0, 100).unwrap();
    assert_eq!(items(&updates).len(), 1);
    assert_eq!(items(&updates)[0]["id"], sent["id"]);
    assert_eq!(items(&updates)[0]["conversation_id"], room);
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

async fn conversation_request(
    f: &Fixture,
    id: &str,
    cookie: Option<&str>,
    bearer: Option<&str>,
) -> (StatusCode, Value) {
    request(
        f,
        "GET",
        &format!("/api/chat/conversations/{id}"),
        None,
        cookie,
        None,
        bearer,
    )
    .await
}

#[tokio::test]
async fn known_conversation_metadata_remains_accessible_outside_the_recent_two_hundred() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let admin = f.admin();
    let (admin_cookie, _) = f.browser(&admin);
    let (seeker_cookie, _) = f.browser(&seeker);
    let (matched, _) = f.matched(&seeker, &employer);
    let platform_id = platform(&seeker);
    let matched_id = format!("match_{matched}");
    send(&f, &seeker, &platform_id, "Old private platform message");
    send(&f, &seeker, &matched_id, "Old shared introduction message");
    {
        let mut db = f.state.conn().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("UPDATE chat_conversations SET updated_at=1", [])
            .unwrap();
        for index in 0..205 {
            let id = format!("recent-chat-user-{index:03}");
            tx.execute(
                "INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES(?1,?2,?3,'seeker',?4,?4)",
                params![id, format!("{id}@example.invalid"), format!("Recent user {index}"), hireferry::now()],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO chat_conversations(id,kind,user_id,created_at,updated_at) VALUES(?1,'platform',?2,?3,?3)",
                params![format!("platform_{id}"), id, hireferry::now()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let recent = request(
        &f,
        "GET",
        "/api/chat/conversations",
        None,
        Some(&admin_cookie),
        None,
        None,
    )
    .await;
    assert_eq!(recent.0, StatusCode::OK);
    assert_eq!(items(&recent.1).len(), 200);
    assert!(
        items(&recent.1)
            .iter()
            .all(|c| c["id"] != platform_id && c["id"] != matched_id)
    );

    for (id, preview) in [
        (&platform_id, "Old private platform message"),
        (&matched_id, "Old shared introduction message"),
    ] {
        let direct = conversation_request(&f, id, Some(&admin_cookie), None).await;
        assert_eq!(direct.0, StatusCode::OK);
        assert_eq!(direct.1["id"], id.as_str());
        assert_eq!(direct.1["last_message_preview"], preview);
        assert_eq!(direct.1["unread_count"], 1);
        assert_eq!(direct.1["updated_at"], 1);
        assert_eq!(direct.1["writable"], true);
        assert_eq!(direct.1["participants"].as_array().unwrap().len(), 2);
        assert!(!direct.1.to_string().contains("@example.invalid"));
        assert_eq!(
            direct.1,
            chat::get_conversation(&f.state, &admin, id).unwrap(),
            "HTTP returns the existing Conversation DTO without a wrapper"
        );
        let owner = conversation_request(&f, id, Some(&seeker_cookie), None).await;
        assert_eq!(owner.0, StatusCode::OK);
        assert_eq!(owner.1["unread_count"], 0);
    }
    let db = f.state.conn().unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM chat_conversations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 207, "Reading an old room must not duplicate it");
    let read_count: i64 = db
        .query_row("SELECT count(*) FROM chat_reads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        read_count, 0,
        "Metadata reads must not mark messages as read"
    );
}

#[tokio::test]
async fn direct_conversation_metadata_rechecks_membership_bans_and_revoked_credentials() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let stranger = f.user("seeker");
    let admin = f.admin();
    let (seeker_cookie, _) = f.browser(&seeker);
    let (employer_cookie, _) = f.browser(&employer);
    let (stranger_cookie, _) = f.browser(&stranger);
    let (admin_cookie, _) = f.browser(&admin);
    let (ai, token) = f.admin_ai();
    let private_id = platform(&seeker);
    assert_eq!(
        conversation_request(&f, &private_id, None, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        conversation_request(&f, &private_id, Some(&stranger_cookie), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.state
            .conn()
            .unwrap()
            .query_row("SELECT count(*) FROM chat_conversations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "Unauthorized metadata requests must not create a room"
    );
    let own = conversation_request(&f, &private_id, Some(&seeker_cookie), None).await;
    assert_eq!(own.0, StatusCode::OK);
    assert_eq!(own.1["user_id"], uid(&seeker));
    let admin_created = conversation_request(&f, &platform(&employer), None, Some(&token)).await;
    assert_eq!(admin_created.0, StatusCode::OK);
    assert_eq!(admin_created.1["user_id"], uid(&employer));

    let (matched, job) = f.matched(&seeker, &employer);
    let matched_id = format!("match_{matched}");
    assert_eq!(
        conversation_request(&f, &matched_id, Some(&stranger_cookie), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        !f.state
            .conn()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_conversations WHERE id=?1)",
                [&matched_id],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
    );
    for cookie in [&seeker_cookie, &employer_cookie] {
        let response = conversation_request(&f, &matched_id, Some(cookie), None).await;
        assert_eq!(response.0, StatusCode::OK);
        assert_eq!(response.1["writable"], true);
    }
    for reason in ["declined", "deleted", "counterpart_banned"] {
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
                        "published"
                    }
                ],
            )
            .unwrap();
            db.execute(
                "UPDATE users SET banned=?2 WHERE id=?1",
                params![uid(&employer), reason == "counterpart_banned"],
            )
            .unwrap();
        }
        let hidden = conversation_request(&f, &matched_id, Some(&seeker_cookie), None).await;
        assert_eq!(hidden.0, StatusCode::NOT_FOUND, "{reason}");
        let archived = conversation_request(&f, &matched_id, Some(&admin_cookie), None).await;
        assert_eq!(archived.0, StatusCode::OK, "{reason}");
        assert_eq!(archived.1["writable"], false, "{reason}");
    }
    f.state
        .conn()
        .unwrap()
        .execute("UPDATE users SET banned=1 WHERE id=?1", [uid(&seeker)])
        .unwrap();
    assert_eq!(
        conversation_request(&f, &private_id, Some(&seeker_cookie), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        chat::get_conversation(&f.state, &seeker, &private_id)
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE admin_api_tokens SET revoked=1 WHERE id=?1",
            [ai.actor.strip_prefix("admin_ai:").unwrap()],
        )
        .unwrap();
    assert_eq!(
        conversation_request(&f, &private_id, None, Some(&token))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        chat::get_conversation(&f.state, &ai, &private_id)
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    f.state
        .conn()
        .unwrap()
        .execute(
            "DELETE FROM sessions WHERE token_hash=?1",
            [auth::hash_token(
                admin_cookie.strip_prefix("hf_session=").unwrap(),
            )],
        )
        .unwrap();
    assert_eq!(
        conversation_request(&f, &private_id, Some(&admin_cookie), None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let (bootstrap_cookie, _) = f.browser(&admin);
    f.state
        .conn()
        .unwrap()
        .execute(
            "UPDATE sessions SET admin=0,bootstrap=1 WHERE token_hash=?1",
            [auth::hash_token(
                bootstrap_cookie.strip_prefix("hf_session=").unwrap(),
            )],
        )
        .unwrap();
    assert_eq!(
        conversation_request(&f, &private_id, Some(&bootstrap_cookie), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[test]
fn bulk_room_materialization_keeps_current_authorization_and_existing_timestamps() {
    let f = Fixture::new();
    let seeker = f.user("seeker");
    let employer = f.user("employer");
    let stranger = f.user("seeker");
    let banned_employer = f.user("employer");
    let (existing, _) = f.matched(&seeker, &employer);
    let (missing, _) = f.matched(&seeker, &employer);
    let (foreign, _) = f.matched(&stranger, &employer);
    let (declined, _) = f.matched(&seeker, &employer);
    let (deleted, deleted_job) = f.matched(&seeker, &employer);
    let (banned, _) = f.matched(&seeker, &banned_employer);
    chat::get_conversation(&f.state, &seeker, &format!("match_{existing}")).unwrap();
    {
        let db = f.state.conn().unwrap();
        db.execute(
            "UPDATE chat_conversations SET created_at=7,updated_at=11",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE matches SET status='declined' WHERE id=?1",
            [&declined],
        )
        .unwrap();
        db.execute(
            "UPDATE jobs SET status='deleted' WHERE id=?1",
            [&deleted_job],
        )
        .unwrap();
        db.execute(
            "UPDATE users SET banned=1 WHERE id=?1",
            [uid(&banned_employer)],
        )
        .unwrap();
    }
    let visible = chat::list_conversations(&f.state, &seeker).unwrap();
    assert_eq!(items(&visible).len(), 3);
    for matched in [&existing, &missing] {
        assert!(
            items(&visible)
                .iter()
                .any(|c| c["id"] == format!("match_{matched}"))
        );
    }
    for matched in [&foreign, &declined, &deleted, &banned] {
        assert!(
            !f.state
                .conn()
                .unwrap()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chat_conversations WHERE match_id=?1)",
                    [matched],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap(),
            "Inaccessible match {matched} was materialized for a member"
        );
    }
    let all = chat::list_conversations(&f.state, &f.admin()).unwrap();
    assert_eq!(items(&all).len(), 7);
    for matched in [&declined, &deleted, &banned] {
        let id = format!("match_{matched}");
        assert_eq!(
            items(&all).iter().find(|c| c["id"] == id).unwrap()["writable"],
            false
        );
    }
    let repeated = chat::list_conversations(&f.state, &f.admin()).unwrap();
    assert_eq!(items(&repeated).len(), 7);
    let existing_id = format!("match_{existing}");
    let original = items(&repeated)
        .iter()
        .find(|c| c["id"] == existing_id)
        .unwrap();
    assert_eq!(original["created_at"], 7);
    assert_eq!(original["updated_at"], 11);
}
