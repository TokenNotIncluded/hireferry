//! Durable early-stage platform AI acknowledgements.
//!
//! The service deliberately does not pretend to be an always-online model.
//! Incoming platform messages are queued, acknowledged asynchronously, and
//! given clear links back to the user's workspace. A future MCP-connected
//! agent can replace `deterministic_reply` without changing the queue or chat
//! permissions.

use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{AppError, AppResult, AppState, config::ChatAutoReply, now};

#[derive(Debug)]
struct Job {
    id: i64,
    conversation_id: String,
    trigger_message_id: i64,
    attempts: u32,
    claim_token: String,
}

pub(crate) fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS chat_ai_jobs(
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id),
    trigger_message_id INTEGER NOT NULL UNIQUE REFERENCES chat_messages(id),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','complete','failed','cancelled')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts>=0),
    next_attempt_at INTEGER NOT NULL,
    lease_until INTEGER,
    claim_token TEXT,
    last_error TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS chat_ai_jobs_due ON chat_ai_jobs(status,next_attempt_at,id);
CREATE INDEX IF NOT EXISTS chat_ai_jobs_conversation ON chat_ai_jobs(conversation_id,status,id);
"#,
    )?;
    Ok(())
}

/// Called inside the message transaction. Only user-authored platform chats
/// enter the queue; matched rooms and platform-authored messages are never
/// answered by this fallback agent.
pub(crate) fn enqueue_tx(
    db: &Connection,
    conversation_id: &str,
    trigger_message_id: i64,
    user_message: bool,
    auto_reply: &ChatAutoReply,
) -> AppResult<()> {
    if !user_message || !auto_reply.enabled {
        return Ok(());
    }
    let is_platform: bool = db.query_row(
        "SELECT kind='platform' FROM chat_conversations WHERE id=?1",
        [conversation_id],
        |row| row.get(0),
    )?;
    if !is_platform {
        return Ok(());
    }
    let at = now();
    db.execute(
        "INSERT OR IGNORE INTO chat_ai_jobs(conversation_id,trigger_message_id,next_attempt_at,created_at,updated_at) VALUES(?1,?2,?3,?4,?4)",
        params![conversation_id, trigger_message_id, at + auto_reply.delay_seconds as i64, at],
    )?;
    Ok(())
}

fn claim(s: &AppState) -> AppResult<Option<Job>> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let at = now();
    tx.execute(
        "UPDATE chat_ai_jobs SET status='pending',lease_until=NULL,claim_token=NULL,updated_at=?1 WHERE status='processing' AND lease_until IS NOT NULL AND lease_until<=?1",
        [at],
    )?;
    let Some((id, conversation_id, trigger_message_id, attempts)) = tx
        .query_row(
            "SELECT id,conversation_id,trigger_message_id,attempts FROM chat_ai_jobs WHERE status='pending' AND next_attempt_at<=?1 ORDER BY created_at,id LIMIT 1",
            [at],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
        )
        .optional()?
    else {
        tx.commit()?;
        return Ok(None);
    };
    let claim_token = uuid::Uuid::new_v4().simple().to_string();
    tx.execute(
        "UPDATE chat_ai_jobs SET status='processing',attempts=attempts+1,lease_until=?2,claim_token=?3,updated_at=?4 WHERE id=?1 AND status='pending'",
        params![id, at + 120, claim_token, at],
    )?;
    // If several messages arrived together, one acknowledgement is enough.
    // Keep the newest trigger and mark older pending work as coalesced.
    tx.execute(
        "UPDATE chat_ai_jobs SET status='cancelled',last_error='coalesced into a newer acknowledgement',updated_at=?2 WHERE conversation_id=?1 AND status='pending' AND id<>?3",
        params![conversation_id, at, id],
    )?;
    tx.commit()?;
    Ok(Some(Job {
        id,
        conversation_id,
        trigger_message_id,
        attempts: attempts.saturating_add(1) as u32,
        claim_token,
    }))
}

fn mark_complete(s: &AppState, job: &Job) -> AppResult<()> {
    let db = s.conn()?;
    db.execute(
        "UPDATE chat_ai_jobs SET status='complete',lease_until=NULL,claim_token=NULL,updated_at=?2 WHERE id=?1 AND status='processing' AND claim_token=?3",
        params![job.id, now(), job.claim_token],
    )?;
    Ok(())
}

fn mark_failed(s: &AppState, job: &Job, error: &str, max_attempts: u32) -> AppResult<()> {
    let db = s.conn()?;
    let status = if job.attempts >= max_attempts {
        "failed"
    } else {
        "pending"
    };
    let delay = 2_i64.saturating_pow(job.attempts.min(8));
    db.execute(
        "UPDATE chat_ai_jobs SET status=?2,next_attempt_at=?3,lease_until=NULL,claim_token=NULL,last_error=?4,updated_at=?5 WHERE id=?1 AND status='processing' AND claim_token=?6",
        params![job.id, status, now() + delay, error.chars().take(500).collect::<String>(), now(), job.claim_token],
    )?;
    Ok(())
}

fn quote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn deterministic_reply(s: &AppState, conversation_id: &str) -> AppResult<String> {
    let db = s.conn()?;
    let role: String = db
        .query_row(
            "SELECT u.role FROM chat_conversations c JOIN users u ON u.id=c.user_id WHERE c.id=?1 AND c.kind='platform' AND u.banned=0",
            [conversation_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("Platform conversation is no longer available"))?;
    let origin = s.cfg.public_url.trim_end_matches('/');
    let (first_label, first_icon, first_url, second_label, second_icon, second_url) =
        if role == "employer" {
            (
                "查看我的岗位",
                "briefcase",
                format!("{origin}/app?tab=jobs"),
                "查看匹配候选人",
                "link",
                format!("{origin}/app?tab=matches"),
            )
        } else {
            (
                "查看我的简历",
                "file",
                format!("{origin}/app?tab=profile"),
                "查看推荐岗位",
                "briefcase",
                format!("{origin}/app?tab=matches"),
            )
        };
    let title = quote(&s.cfg.chat.auto_reply.title);
    let body = quote(&s.cfg.chat.auto_reply.body_md);
    Ok(format!(
        "root = Col([intro, actions])\nintro = Card([title, copy], \"soft\", 5, 4)\ntitle = Title(\"{title}\", \"lg\")\ncopy = Markdown(\"{body}\")\nactions = Row([first, second], 3, \"start\", \"start\", true)\nfirst = Button(\"{}\", @OpenUrl(\"{}\"), \"outline\", \"primary\", \"md\", \"{}\")\nsecond = Button(\"{}\", @OpenUrl(\"{}\"), \"solid\", \"primary\", \"md\", \"{}\")",
        quote(first_label),
        quote(&first_url),
        first_icon,
        quote(second_label),
        quote(&second_url),
        second_icon
    ))
}

pub async fn run_once(s: &AppState) -> AppResult<bool> {
    if !s.cfg.chat.auto_reply.enabled {
        return Ok(false);
    }
    let Some(job) = claim(s)? else {
        return Ok(false);
    };
    let result = deterministic_reply(s, &job.conversation_id).and_then(|body| {
        crate::chat::send_system_ai(
            s,
            &job.conversation_id,
            &format!("auto-reply:{}", job.trigger_message_id),
            &body,
            "openui",
        )
    });
    match result {
        Ok(_) => mark_complete(s, &job)?,
        Err(error) => mark_failed(s, &job, &error.message, s.cfg.chat.auto_reply.max_attempts)?,
    }
    Ok(true)
}

pub fn spawn_worker(s: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let did_work = run_once(&s).await.unwrap_or(false);
            let seconds = if did_work {
                1
            } else {
                s.cfg.chat.auto_reply.poll_seconds.max(1)
            };
            tokio::time::sleep(Duration::from_secs(seconds)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn openui_quotes_cannot_escape_a_string() {
        assert_eq!(quote("a\\\"\n"), "a\\\\\\\"\\n");
    }
}
