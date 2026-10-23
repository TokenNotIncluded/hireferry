//! Durable service emails. Delivery means SMTP acceptance, not inbox receipt.
use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    auth,
    db::audit,
    now, safety,
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::get,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(r#"
CREATE TABLE IF NOT EXISTS email_runtime(id INTEGER PRIMARY KEY CHECK(id=1),enabled INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO email_runtime(id) VALUES(1);
CREATE TABLE IF NOT EXISTS email_preferences(user_id TEXT PRIMARY KEY REFERENCES users(id),messages INTEGER NOT NULL DEFAULT 1 CHECK(messages IN(0,1)),matches INTEGER NOT NULL DEFAULT 1 CHECK(matches IN(0,1)),orders INTEGER NOT NULL DEFAULT 1 CHECK(orders IN(0,1)),platform INTEGER NOT NULL DEFAULT 1 CHECK(platform IN(0,1)));
CREATE TABLE IF NOT EXISTS email_notifications(
 id TEXT PRIMARY KEY DEFAULT ('mail_'||lower(hex(randomblob(16)))),user_id TEXT NOT NULL REFERENCES users(id),
 category TEXT NOT NULL CHECK(category IN('messages','matches','orders','platform')),resource_id TEXT NOT NULL DEFAULT '',
 subject TEXT NOT NULL,body TEXT NOT NULL,dedupe_key TEXT NOT NULL UNIQUE,actor TEXT NOT NULL DEFAULT 'system',
 status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN('pending','processing','accepted','failed','cancelled')),
 attempts INTEGER NOT NULL DEFAULT 0,next_attempt_at INTEGER NOT NULL DEFAULT (unixepoch()),lease_until INTEGER,claim_token TEXT,
 last_error TEXT,created_at INTEGER NOT NULL DEFAULT (unixepoch()),accepted_at INTEGER);
CREATE INDEX IF NOT EXISTS email_due ON email_notifications(status,next_attempt_at);
CREATE INDEX IF NOT EXISTS email_retention ON email_notifications(status,created_at);
CREATE INDEX IF NOT EXISTS email_user_created ON email_notifications(user_id,created_at);
CREATE TRIGGER IF NOT EXISTS email_message_insert AFTER INSERT ON chat_messages WHEN NEW.status='complete' AND (SELECT enabled FROM email_runtime WHERE id=1)=1 BEGIN
 INSERT OR IGNORE INTO email_notifications(user_id,category,resource_id,subject,body,dedupe_key)
 SELECT u.id,'messages',NEW.conversation_id,'你有新的站内消息 / New message','聘渡有新的站内消息，请登录查看。 You have a new message on HireFerry. Sign in to read it.','message:'||NEW.id||':'||u.id
 FROM users u JOIN chat_conversations c ON c.id=NEW.conversation_id LEFT JOIN matches m ON m.id=c.match_id LEFT JOIN jobs j ON j.id=m.job_id
 WHERE u.banned=0 AND (NEW.sender_id IS NULL OR u.id<>NEW.sender_id) AND
 ((c.kind='platform' AND c.user_id=u.id) OR (c.kind='matched' AND m.status<>'declined' AND j.status<>'deleted' AND u.id IN(m.seeker_id,j.owner_id)))
 AND coalesce((SELECT messages FROM email_preferences WHERE user_id=u.id),1)=1
 AND (SELECT count(*) FROM email_notifications WHERE user_id=u.id AND created_at>unixepoch()-3600)<20;
END;
CREATE TRIGGER IF NOT EXISTS email_message_complete AFTER UPDATE OF status ON chat_messages WHEN NEW.status='complete' AND OLD.status<>'complete' AND (SELECT enabled FROM email_runtime WHERE id=1)=1 BEGIN
 INSERT OR IGNORE INTO email_notifications(user_id,category,resource_id,subject,body,dedupe_key)
 SELECT u.id,'messages',NEW.conversation_id,'你有新的站内消息 / New message','聘渡有新的站内消息，请登录查看。 You have a new message on HireFerry. Sign in to read it.','message:'||NEW.id||':'||u.id
 FROM users u JOIN chat_conversations c ON c.id=NEW.conversation_id LEFT JOIN matches m ON m.id=c.match_id LEFT JOIN jobs j ON j.id=m.job_id
 WHERE u.banned=0 AND (NEW.sender_id IS NULL OR u.id<>NEW.sender_id) AND
 ((c.kind='platform' AND c.user_id=u.id) OR (c.kind='matched' AND m.status<>'declined' AND j.status<>'deleted' AND u.id IN(m.seeker_id,j.owner_id)))
 AND coalesce((SELECT messages FROM email_preferences WHERE user_id=u.id),1)=1
 AND (SELECT count(*) FROM email_notifications WHERE user_id=u.id AND created_at>unixepoch()-3600)<20;
END;
"#)?;
    for (event, condition) in [
        ("INSERT", "1"),
        ("UPDATE OF status", "OLD.status IS NOT NEW.status"),
    ] {
        let suffix = if event == "INSERT" {
            "insert"
        } else {
            "update"
        };
        db.execute_batch(&format!(r#"
CREATE TRIGGER IF NOT EXISTS email_match_{suffix} AFTER {event} ON matches WHEN {condition} AND (SELECT enabled FROM email_runtime WHERE id=1)=1 BEGIN
 INSERT INTO email_notifications(user_id,category,resource_id,subject,body,dedupe_key)
 SELECT u.id,'matches',NEW.id,'匹配动态 / Introduction update','你的匹配有新动态，请登录聘渡查看。 Your introduction has an update. Sign in to HireFerry to view it.','match:'||lower(hex(randomblob(16)))
 FROM users u JOIN jobs j ON j.id=NEW.job_id WHERE u.id IN(NEW.seeker_id,j.owner_id) AND u.banned=0 AND j.status<>'deleted'
 AND coalesce((SELECT matches FROM email_preferences WHERE user_id=u.id),1)=1
 AND (SELECT count(*) FROM email_notifications WHERE user_id=u.id AND created_at>unixepoch()-3600)<20;
END;"#))?;
    }
    db.execute_batch(r#"
CREATE TRIGGER IF NOT EXISTS email_order_update AFTER UPDATE OF status,refunded_cents ON orders WHEN (OLD.status IS NOT NEW.status OR OLD.refunded_cents IS NOT NEW.refunded_cents) AND NEW.status IN('paid','partially_refunded','refunded') AND (SELECT enabled FROM email_runtime WHERE id=1)=1 BEGIN
 INSERT INTO email_notifications(user_id,category,resource_id,subject,body,dedupe_key)
 SELECT u.id,'orders',NEW.id,'订单动态 / Order update','你的订单状态已更新，请登录聘渡核对。 Your order status has changed. Sign in to HireFerry to review it.','order:'||lower(hex(randomblob(16)))
 FROM users u WHERE u.id=NEW.user_id AND u.banned=0 AND coalesce((SELECT orders FROM email_preferences WHERE user_id=u.id),1)=1
 AND (SELECT count(*) FROM email_notifications WHERE user_id=u.id AND created_at>unixepoch()-3600)<20;
END;
"#)?;
    Ok(())
}

pub fn configure(db: &Connection, enabled: bool) -> AppResult<()> {
    db.execute("UPDATE email_runtime SET enabled=?1 WHERE id=1", [enabled])?;
    Ok(())
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub messages: bool,
    pub matches: bool,
    pub orders: bool,
    pub platform: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            messages: true,
            matches: true,
            orders: true,
            platform: true,
        }
    }
}
fn preferences(db: &Connection, uid: &str) -> AppResult<Preferences> {
    Ok(db
        .query_row(
            "SELECT messages,matches,orders,platform FROM email_preferences WHERE user_id=?1",
            [uid],
            |r| {
                Ok(Preferences {
                    messages: r.get(0)?,
                    matches: r.get(1)?,
                    orders: r.get(2)?,
                    platform: r.get(3)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}
fn allowed(p: &Preferences, category: &str) -> bool {
    match category {
        "messages" => p.messages,
        "matches" => p.matches,
        "orders" => p.orders,
        "platform" => p.platform,
        _ => false,
    }
}

pub fn settings(s: &AppState, uid: &str) -> AppResult<Value> {
    Ok(json!({"enabled":s.cfg.smtp.enabled,"preferences":preferences(&*s.conn()?,uid)?}))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/notifications/email",
            get(get_preferences).put(save_preferences),
        )
        .route(
            "/api/admin/notifications/email",
            get(list_http).post(send_http),
        )
}
async fn get_preferences(State(s): State<AppState>, h: HeaderMap) -> AppResult<Json<Value>> {
    let uid = crate::user_id(&s, &h, false)?;
    Ok(Json(settings(&s, &uid)?))
}
async fn save_preferences(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<Preferences>,
) -> AppResult<Json<Value>> {
    if h.contains_key("authorization") || h.contains_key("x-hireferry-ai") {
        return Err(AppError::forbidden(
            "Change email preferences in your browser",
        ));
    }
    let uid = crate::user_id(&s, &h, true)?;
    let db = s.conn()?;
    db.execute("INSERT INTO email_preferences(user_id,messages,matches,orders,platform) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(user_id) DO UPDATE SET messages=excluded.messages,matches=excluded.matches,orders=excluded.orders,platform=excluded.platform",params![uid,input.messages,input.matches,input.orders,input.platform])?;
    audit(&db, &uid, "email.preferences", &uid, "")?;
    Ok(Json(json!({"preferences":input})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    pub user_id: String,
    pub subject: String,
    pub body: String,
    pub request_key: String,
}
pub fn enqueue(s: &AppState, p: &Principal, input: SendInput) -> AppResult<Value> {
    safety::bounded_text(&input.subject, 160, "Subject")?;
    safety::bounded_text(&input.body, 5000, "Body")?;
    if input.subject.trim().is_empty()
        || input.body.trim().is_empty()
        || input.subject.contains(['\r', '\n'])
    {
        return Err(AppError::bad_request(
            "Enter a subject and message without header line breaks",
        ));
    }
    if !(8..=128).contains(&input.request_key.len())
        || !input
            .request_key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(AppError::bad_request(
            "Use an 8–128 character request_key with letters, numbers, hyphens or underscores",
        ));
    }
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    access::ensure_admin(&tx, p)?;
    if !s.cfg.smtp.enabled {
        return Err(AppError::unavailable("Email delivery is not configured"));
    }
    let key = format!("platform:{}:{}", p.actor, input.request_key);
    let previous: Option<(String, String, String, String, String)> = tx
        .query_row(
            "SELECT id,user_id,subject,body,status FROM email_notifications WHERE dedupe_key=?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    if let Some((id, uid, subject, body, status)) = previous {
        if uid != input.user_id || subject != input.subject || body != input.body {
            return Err(AppError::conflict(
                "request_key was already used for different content",
            ));
        }
        return Ok(json!({"id":id,"status":status,"reused":true}));
    }
    let recipient:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND banned=0 AND role IN('seeker','employer'))",[&input.user_id],|r|r.get(0))?;
    if !recipient {
        return Err(AppError::not_found("Active user not found"));
    }
    if !preferences(&tx, &input.user_id)?.platform {
        return Err(AppError::conflict(
            "This user has disabled platform email notifications",
        ));
    }
    let limited:bool=tx.query_row("SELECT (SELECT count(*) FROM email_notifications WHERE category='platform' AND created_at>?1)>=100 OR (SELECT count(*) FROM email_notifications WHERE user_id=?2 AND category='platform' AND created_at>?1)>=5",params![now()-3600,input.user_id],|r|r.get(0))?;
    if limited {
        return Err(AppError::conflict(
            "Email notification limit reached; try later",
        ));
    }
    let id = format!("mail_{}", uuid::Uuid::new_v4().simple());
    tx.execute("INSERT INTO email_notifications(id,user_id,category,subject,body,dedupe_key,actor) VALUES(?1,?2,'platform',?3,?4,?5,?6)",params![id,input.user_id,input.subject,input.body,key,p.actor])?;
    audit(&tx, &p.actor, "email.queued", &id, &input.user_id)?;
    tx.commit()?;
    Ok(json!({"id":id,"status":"pending","reused":false}))
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ListInput {
    pub user_id: Option<String>,
}
pub fn list(s: &AppState, p: &Principal, input: ListInput) -> AppResult<Value> {
    let db = s.conn()?;
    access::ensure_admin(&db, p)?;
    let mut query=db.prepare("SELECT id,user_id,category,subject,status,attempts,last_error,created_at,accepted_at FROM email_notifications WHERE (?1 IS NULL OR user_id=?1) ORDER BY created_at DESC,id LIMIT 100")?;
    let items=query.query_map([input.user_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"user_id":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"subject":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"attempts":r.get::<_,i64>(5)?,"last_error":r.get::<_,Option<String>>(6)?,"created_at":r.get::<_,i64>(7)?,"accepted_at":r.get::<_,Option<i64>>(8)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"items":items,"smtp_enabled":s.cfg.smtp.enabled}))
}
async fn send_http(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(input): Json<SendInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, true)?;
    Ok(Json(enqueue(&s, &p, input)?))
}
async fn list_http(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(input): Query<ListInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &h, false)?;
    Ok(Json(list(&s, &p, input)?))
}

struct Delivery {
    id: String,
    uid: String,
    category: String,
    resource: String,
    subject: String,
    body: String,
    attempts: i64,
    token: String,
    actor: String,
}
fn eligible(db: &Connection, d: &Delivery) -> AppResult<Option<String>> {
    let email: Option<String> = db
        .query_row(
            "SELECT email FROM users WHERE id=?1 AND banned=0 AND role IN('seeker','employer')",
            [&d.uid],
            |r| r.get(0),
        )
        .optional()?;
    if email.is_none() || !allowed(&preferences(db, &d.uid)?, &d.category) {
        return Ok(None);
    }
    let visible=match d.category.as_str() {
  "messages"=>db.query_row("SELECT EXISTS(SELECT 1 FROM chat_conversations c LEFT JOIN matches m ON m.id=c.match_id LEFT JOIN jobs j ON j.id=m.job_id LEFT JOIN users seeker ON seeker.id=m.seeker_id LEFT JOIN users employer ON employer.id=j.owner_id WHERE c.id=?1 AND ((c.kind='platform' AND c.user_id=?2) OR (c.kind='matched' AND m.status<>'declined' AND j.status<>'deleted' AND seeker.banned=0 AND employer.banned=0 AND ?2 IN(m.seeker_id,j.owner_id))))",params![d.resource,d.uid],|r|r.get::<_,bool>(0))?,
  "matches"=>db.query_row("SELECT EXISTS(SELECT 1 FROM matches m JOIN jobs j ON j.id=m.job_id WHERE m.id=?1 AND j.status<>'deleted' AND ?2 IN(m.seeker_id,j.owner_id))",params![d.resource,d.uid],|r|r.get(0))?,
  "orders"=>db.query_row("SELECT EXISTS(SELECT 1 FROM orders WHERE id=?1 AND user_id=?2)",params![d.resource,d.uid],|r|r.get(0))?,
  "platform"=>true,_=>false,
 };
    Ok(if visible { email } else { None })
}
async fn dispatch_with<F, Fut>(s: &AppState, send: F) -> AppResult<bool>
where
    F: FnOnce(String, String, String, String) -> Fut,
    Fut: Future<Output = AppResult<()>>,
{
    let delivery = {
        let mut db = s.conn()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE email_notifications SET status=CASE WHEN attempts>=5 THEN 'failed' ELSE 'pending' END,claim_token=NULL,lease_until=NULL WHERE status='processing' AND lease_until<?1",[now()])?;
        tx.execute("DELETE FROM email_notifications WHERE rowid IN(SELECT rowid FROM email_notifications WHERE status IN('accepted','failed','cancelled') AND created_at<?1 LIMIT 100)",[now()-30*86400])?;
        let token = uuid::Uuid::new_v4().to_string();
        let found=tx.query_row("SELECT id,user_id,category,resource_id,subject,body,attempts,actor FROM email_notifications WHERE status='pending' AND next_attempt_at<=?1 ORDER BY created_at,id LIMIT 1",[now()],|r|Ok(Delivery{id:r.get(0)?,uid:r.get(1)?,category:r.get(2)?,resource:r.get(3)?,subject:r.get(4)?,body:r.get(5)?,attempts:r.get(6)?,token:token.clone(),actor:r.get(7)?})).optional()?;
        let Some(d) = found else { return Ok(false) };
        let Some(email) = eligible(&tx, &d)? else {
            tx.execute("UPDATE email_notifications SET status='cancelled',last_error='recipient_unavailable_or_unsubscribed' WHERE id=?1",[&d.id])?;
            tx.commit()?;
            return Ok(true);
        };
        tx.execute("UPDATE email_notifications SET status='processing',attempts=attempts+1,lease_until=?2,claim_token=?3 WHERE id=?1",params![d.id,now()+120,token])?;
        tx.commit()?;
        (d, email)
    };
    let (d, email) = delivery;
    let tab = match d.category.as_str() {
        "orders" => "orders",
        "messages" => "messages",
        "matches" => "matches",
        _ => "overview",
    };
    let ai_notice = if d.actor.contains("_ai:") || d.actor.contains("_webmcp:") {
        "平台 AI 服务通知 / Platform AI service notice\n\n"
    } else {
        ""
    };
    let body = format!(
        "{}{}\n\n{}/app?tab={}\n\n管理邮件通知 / Email preferences: {}/app?tab=profile\n{}",
        ai_notice, d.body, s.cfg.public_url, tab, s.cfg.public_url, s.cfg.site.name
    );
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        send(email, d.subject.clone(), body, d.id.clone()),
    )
    .await;
    let (status, error) = match result {
        Ok(Ok(())) => ("accepted", None),
        _ if d.attempts + 1 >= 5 => ("failed", Some("smtp_not_confirmed")),
        _ => ("pending", Some("smtp_not_confirmed")),
    };
    let db = s.conn()?;
    db.execute("UPDATE email_notifications SET status=?2,last_error=?3,next_attempt_at=?4,accepted_at=CASE WHEN ?2='accepted' THEN ?5 ELSE NULL END,claim_token=NULL,lease_until=NULL WHERE id=?1 AND claim_token=?6",params![d.id,status,error,now()+60*(1_i64<<d.attempts.min(4)),now(),d.token])?;
    Ok(true)
}
pub fn spawn_worker(s: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if s.cfg.smtp.enabled {
                let state = s.clone();
                if let Err(error) = dispatch_with(&s, move |to, subject, body, id| async move {
                    auth::send_notification_mail(&state, &to, &subject, body, &id).await
                })
                .await
                {
                    tracing::warn!(error=%error,"Email queue processing failed");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> AppState {
        let mut cfg = crate::config::Config {
            development: true,
            database: ":memory:".into(),
            ..Default::default()
        };
        cfg.smtp.enabled = true;
        cfg.smtp.host = "smtp.example.invalid".into();
        cfg.smtp.from = "notify@example.invalid".into();
        let s = AppState::new(cfg).unwrap();
        s.conn().unwrap().execute_batch("INSERT INTO users(id,email,role,created_at,updated_at) VALUES('a','a@example.invalid','seeker',1,1),('b','b@example.invalid','employer',1,1),('c','c@example.invalid','seeker',1,1); INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,status,created_at,updated_at) VALUES('j','b','Role','Body',300000,'published',1,1);").unwrap();
        s
    }
    fn admin() -> Principal {
        Principal {
            user_id: None,
            admin: true,
            via_ai: false,
            actor: "admin".into(),
        }
    }
    fn input() -> SendInput {
        SendInput {
            user_id: "a".into(),
            subject: "Follow-up".into(),
            body: "Please review your request.".into(),
            request_key: "request-123".into(),
        }
    }
    fn count(s: &AppState) -> i64 {
        s.conn()
            .unwrap()
            .query_row("SELECT count(*) FROM email_notifications", [], |r| r.get(0))
            .unwrap()
    }
    fn status(s: &AppState) -> String {
        s.conn()
            .unwrap()
            .query_row("SELECT status FROM email_notifications LIMIT 1", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    fn message(s: &AppState, phase: &str) {
        s.conn().unwrap().execute("INSERT OR IGNORE INTO chat_conversations(id,kind,user_id,created_at,updated_at) VALUES('room','platform','a',1,1)",[]).unwrap();
        s.conn().unwrap().execute("INSERT INTO chat_messages(conversation_id,sender_kind,sender_key,sender_actor,body,client_id,via_ai,created_at,status) VALUES('room','platform','platform','admin','secret content','msg-1',1,1,?1)",[phase]).unwrap();
    }
    #[tokio::test]
    async fn manual_delivery_is_authorized_idempotent_and_bounded() {
        let s = fixture();
        let p = admin();
        let first = enqueue(&s, &p, input()).unwrap();
        assert_eq!(first["status"], "pending");
        assert_eq!(enqueue(&s, &p, input()).unwrap()["id"], first["id"]);
        assert_eq!(count(&s), 1);
        let mut changed = input();
        changed.body = "changed".into();
        assert_eq!(
            enqueue(&s, &p, changed).unwrap_err().status,
            axum::http::StatusCode::CONFLICT
        );
        let member = Principal {
            user_id: Some("a".into()),
            admin: false,
            via_ai: false,
            actor: "a".into(),
        };
        assert_eq!(
            enqueue(&s, &member, input()).unwrap_err().status,
            axum::http::StatusCode::FORBIDDEN
        );
        for i in 0..4 {
            let mut x = input();
            x.request_key = format!("request-{i:08}");
            enqueue(&s, &p, x).unwrap();
        }
        let mut x = input();
        x.request_key = "limit-test".into();
        assert!(enqueue(&s, &p, x).is_err());
        assert!(list(&s, &member, ListInput::default()).is_err());
        assert_eq!(
            list(
                &s,
                &p,
                ListInput {
                    user_id: Some("b".into())
                }
            )
            .unwrap()["items"],
            json!([])
        );
    }
    #[tokio::test]
    async fn no_arbitrary_recipients_or_header_injection_or_unsubscribed_users() {
        let s = fixture();
        let mut x = input();
        x.user_id = "outside@example.invalid".into();
        assert!(enqueue(&s, &admin(), x).is_err());
        let mut x = input();
        x.subject = "subject\r\nBcc: thief@example.invalid".into();
        assert!(enqueue(&s, &admin(), x).is_err());
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO email_preferences(user_id,platform) VALUES('a',0)",
                [],
            )
            .unwrap();
        assert!(enqueue(&s, &admin(), input()).is_err());
        assert_eq!(count(&s), 0);
    }
    #[tokio::test]
    async fn streaming_mail_waits_for_completion_and_never_copies_message_content() {
        let s = fixture();
        message(&s, "streaming");
        assert_eq!(count(&s), 0);
        s.conn()
            .unwrap()
            .execute("UPDATE chat_messages SET status='complete'", [])
            .unwrap();
        assert_eq!(count(&s), 1);
        s.conn()
            .unwrap()
            .execute("UPDATE chat_messages SET status='complete'", [])
            .unwrap();
        assert_eq!(count(&s), 1);
        let db = s.conn().unwrap();
        let (uid, body): (String, String) = db
            .query_row("SELECT user_id,body FROM email_notifications", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(uid, "a");
        assert!(!body.contains("secret content"));
    }
    #[tokio::test]
    async fn disabled_runtime_does_not_backfill_and_rolled_back_events_do_not_escape() {
        let s = fixture();
        configure(&s.conn().unwrap(), false).unwrap();
        message(&s, "complete");
        assert_eq!(count(&s), 0);
        configure(&s.conn().unwrap(), true).unwrap();
        assert_eq!(count(&s), 0);
        let mut db = s.conn().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("INSERT INTO matches(id,seeker_id,job_id,created_at,updated_at) VALUES('m','a','j',1,1)",[]).unwrap();
        assert_eq!(
            tx.query_row("SELECT count(*) FROM email_notifications", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        tx.rollback().unwrap();
        drop(db);
        assert_eq!(count(&s), 0);
    }
    #[tokio::test]
    async fn match_updates_notify_only_participants_and_noop_updates_do_not_duplicate() {
        let s = fixture();
        let db = s.conn().unwrap();
        db.execute("INSERT INTO matches(id,seeker_id,job_id,created_at,updated_at) VALUES('m','a','j',1,1)",[]).unwrap();
        db.execute("UPDATE matches SET status=status", []).unwrap();
        drop(db);
        assert_eq!(count(&s), 2);
        s.conn()
            .unwrap()
            .execute("UPDATE matches SET status='interview'", [])
            .unwrap();
        assert_eq!(count(&s), 4);
        assert_eq!(
            s.conn()
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM email_notifications WHERE user_id='c'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn accepted_mail_has_a_stable_id_and_is_not_dispatched_twice() {
        let s = fixture();
        let id = enqueue(&s, &admin(), input()).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            dispatch_with(&s, |to, subject, body, message_id| async move {
                assert_eq!(to, "a@example.invalid");
                assert_eq!(subject, "Follow-up");
                assert_eq!(message_id, id);
                assert!(body.contains("/app?tab=profile"));
                Ok(())
            })
            .await
            .unwrap()
        );
        assert_eq!(status(&s), "accepted");
        assert!(
            !dispatch_with(&s, |_, _, _, _| async { panic!("must not resend") })
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn unsubscribed_or_suspended_recipient_is_cancelled_before_smtp() {
        for change in [
            "INSERT INTO email_preferences(user_id,platform) VALUES('a',0)",
            "UPDATE users SET banned=1 WHERE id='a'",
        ] {
            let s = fixture();
            enqueue(&s, &admin(), input()).unwrap();
            s.conn().unwrap().execute(change, []).unwrap();
            dispatch_with(&s, |_, _, _, _| async { panic!("must not send") })
                .await
                .unwrap();
            assert_eq!(status(&s), "cancelled");
        }
    }
    #[tokio::test]
    async fn revoked_conversation_access_cancels_pending_message_mail() {
        let s = fixture();
        s.conn().unwrap().execute("INSERT INTO matches(id,seeker_id,job_id,created_at,updated_at) VALUES('m','a','j',1,1)",[]).unwrap();
        s.conn()
            .unwrap()
            .execute("DELETE FROM email_notifications", [])
            .unwrap();
        s.conn().unwrap().execute_batch("INSERT INTO chat_conversations(id,kind,match_id,created_at,updated_at) VALUES('shared','matched','m',1,1);INSERT INTO chat_messages(conversation_id,sender_kind,sender_id,sender_key,sender_actor,body,client_id,created_at) VALUES('shared','user','a','user:a','a','Private','m1',1); UPDATE matches SET status='declined'; DELETE FROM email_notifications WHERE category<>'messages';").unwrap();
        assert_eq!(count(&s), 1);
        dispatch_with(&s, |_, _, _, _| async {
            panic!("revoked message must not send")
        })
        .await
        .unwrap();
        assert_eq!(status(&s), "cancelled");
    }
    #[tokio::test]
    async fn smtp_failure_retries_with_a_limit_and_expired_leases_recover() {
        let s = fixture();
        enqueue(&s, &admin(), input()).unwrap();
        s.conn().unwrap().execute("UPDATE email_notifications SET status='processing',attempts=1,lease_until=?1,claim_token='old'",[now()-1]).unwrap();
        for attempt in 2..=5 {
            dispatch_with(&s, |_, _, _, _| async {
                Err(AppError::unavailable("synthetic failure"))
            })
            .await
            .unwrap();
            assert_eq!(status(&s), if attempt == 5 { "failed" } else { "pending" });
            s.conn()
                .unwrap()
                .execute("UPDATE email_notifications SET next_attempt_at=0", [])
                .unwrap();
        }
        assert!(
            !dispatch_with(&s, |_, _, _, _| async { panic!("retry limit") })
                .await
                .unwrap()
        );
    }
    #[tokio::test]
    async fn migration_from_14_preserves_data_and_rolls_back_on_failure() {
        let s = fixture();
        let db = s.conn().unwrap();
        db.execute_batch("DROP TABLE email_notifications;DROP TABLE email_preferences;DROP TABLE email_runtime;PRAGMA user_version=14;").unwrap();
        crate::db::migrate(&db).unwrap();
        assert_eq!(
            db.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            16
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM users", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        db.execute_batch("DROP TABLE email_notifications;DROP TABLE email_preferences;DROP TABLE email_runtime;CREATE TABLE email_notifications(sentinel TEXT);PRAGMA user_version=14;").unwrap();
        assert!(crate::db::migrate(&db).is_err());
        assert_eq!(
            db.query_row("pragma user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            14
        );
        assert!(
            !db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='email_runtime')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
    }
}
