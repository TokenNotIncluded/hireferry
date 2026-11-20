//! Public job discussion with member-only writes and private-profile attribution.
use crate::{AppError, AppResult, AppState, auth, models, now, user_id};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::HeaderMap,
    routing::{delete, get, post, put},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Deserialize;
use serde_json::{Value, json};

const PAGE_SIZE: i64 = 20;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/public/jobs/{id}/interactions", get(read))
        .route("/api/jobs/{id}/vote", put(vote))
        .route("/api/jobs/{id}/comments", post(comment))
        .route(
            "/api/jobs/{id}/comments/{comment_id}",
            delete(remove_comment),
        )
        .layer(DefaultBodyLimit::max(16 * 1024))
}

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS job_votes (
            job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            vote INTEGER NOT NULL CHECK(vote IN (-1,1)),
            PRIMARY KEY(job_id,user_id)
        );
        CREATE TABLE IF NOT EXISTS job_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
            user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            body TEXT NOT NULL CHECK(length(body) BETWEEN 1 AND 2000),
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS job_comments_job ON job_comments(job_id,id DESC);",
    )?;
    Ok(())
}

fn require_public(db: &Connection, id: &str) -> AppResult<()> {
    let visible: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs j JOIN users u ON u.id=j.owner_id
         WHERE j.id=?1 AND j.status='published' AND j.headcount>0 AND u.banned=0)",
        [id],
        |r| r.get(0),
    )?;
    if !visible {
        return Err(AppError::not_found("Public job not found"));
    }
    Ok(())
}

fn summary(db: &Connection, id: &str, viewer: Option<&str>, offset: i64) -> AppResult<Value> {
    require_public(db, id)?;
    let (upvotes, downvotes): (i64, i64) = db.query_row(
        "SELECT COALESCE(sum(v.vote=1),0),COALESCE(sum(v.vote=-1),0)
         FROM job_votes v JOIN users u ON u.id=v.user_id WHERE v.job_id=?1 AND u.banned=0",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let my_vote: i64 = db.query_row(
        "SELECT COALESCE((SELECT vote FROM job_votes WHERE job_id=?1 AND user_id=?2),0)",
        params![id, viewer],
        |r| r.get(0),
    )?;
    let total: i64 = db.query_row(
        "SELECT count(*) FROM job_comments c JOIN users u ON u.id=c.user_id WHERE c.job_id=?1 AND u.banned=0",
        [id], |r| r.get(0),
    )?;
    let mut stmt = db.prepare(
        "SELECT c.id,c.body,CASE WHEN u.public_profile=1 AND u.name<>'' THEN u.name ELSE '会员' END,
         c.created_at,c.user_id=?2 FROM job_comments c JOIN users u ON u.id=c.user_id
         WHERE c.job_id=?1 AND u.banned=0 ORDER BY c.id DESC LIMIT ?3 OFFSET ?4",
    )?;
    let comments = stmt
        .query_map(params![id, viewer, PAGE_SIZE, offset], |r| {
            Ok(json!({"id":r.get::<_,i64>(0)?,"body":r.get::<_,String>(1)?,
            "author_name":r.get::<_,String>(2)?,"created_at":r.get::<_,i64>(3)?,
            "can_delete":r.get::<_,Option<bool>>(4)?.unwrap_or(false)}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"upvotes":upvotes,"downvotes":downvotes,"my_vote":my_vote,
        "comments":comments,"total":total,"offset":offset,"limit":PAGE_SIZE}),
    )
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    #[serde(default)]
    offset: i64,
}

async fn read(
    State(s): State<AppState>,
    Path(id): Path<String>,
    h: HeaderMap,
    Query(page): Query<Page>,
) -> AppResult<Json<Value>> {
    if !(0..=100_000).contains(&page.offset) {
        return Err(AppError::bad_request("Invalid comment offset"));
    }
    let viewer = if h.contains_key("authorization") || h.contains_key("x-hireferry-ai") {
        Some(user_id(&s, &h, false)?)
    } else {
        auth::session(&s, &h)?
            .filter(|session| !session.admin && !session.bootstrap)
            .and_then(|session| session.user_id)
    };
    let db = s.conn()?;
    Ok(Json(summary(&db, &id, viewer.as_deref(), page.offset)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoteInput {
    vote: i64,
}

async fn vote(
    State(s): State<AppState>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<VoteInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    if !(-1..=1).contains(&input.vote) {
        return Err(AppError::bad_request("Vote must be -1, 0 or 1"));
    }
    mutate(&s, &uid, &id, |db| {
        auth::rate_limit(db, &format!("job-vote:{uid}"), 120, 60)?;
        if input.vote == 0 {
            db.execute(
                "DELETE FROM job_votes WHERE job_id=?1 AND user_id=?2",
                params![id, uid],
            )?;
        } else {
            db.execute(
                "INSERT INTO job_votes(job_id,user_id,vote) VALUES(?1,?2,?3)
                ON CONFLICT(job_id,user_id) DO UPDATE SET vote=excluded.vote",
                params![id, uid, input.vote],
            )?;
        }
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentInput {
    body: String,
}

async fn comment(
    State(s): State<AppState>,
    Path(id): Path<String>,
    h: HeaderMap,
    Json(input): Json<CommentInput>,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    let body = input.body.trim();
    if body.is_empty()
        || body.chars().count() > 2000
        || body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(AppError::bad_request(
            "Comment must contain 1–2000 characters without invalid control characters",
        ));
    }
    mutate(&s, &uid, &id, |db| {
        auth::rate_limit(db, &format!("job-comment:{uid}"), 10, 60)?;
        db.execute(
            "INSERT INTO job_comments(job_id,user_id,body,created_at) VALUES(?1,?2,?3,?4)",
            params![id, uid, body, now()],
        )?;
        Ok(())
    })
}

async fn remove_comment(
    State(s): State<AppState>,
    Path((id, comment_id)): Path<(String, i64)>,
    h: HeaderMap,
) -> AppResult<Json<Value>> {
    let uid = user_id(&s, &h, true)?;
    mutate(&s, &uid, &id, |db| {
        let author: Option<String> = db
            .query_row(
                "SELECT user_id FROM job_comments WHERE id=?1 AND job_id=?2",
                params![comment_id, id],
                |r| r.get(0),
            )
            .optional()?;
        let author = author.ok_or_else(|| AppError::not_found("Comment not found"))?;
        if author != uid {
            return Err(AppError::forbidden("You can only delete your own comments"));
        }
        db.execute(
            "DELETE FROM job_comments WHERE id=?1 AND job_id=?2",
            params![comment_id, id],
        )?;
        Ok(())
    })
}

fn mutate(
    s: &AppState,
    uid: &str,
    id: &str,
    action: impl FnOnce(&Connection) -> AppResult<()>,
) -> AppResult<Json<Value>> {
    let mut db = s.conn()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if models::load_user(&tx, uid)?.banned {
        return Err(AppError::forbidden("Account is suspended"));
    }
    require_public(&tx, id)?;
    action(&tx)?;
    let value = summary(&tx, id, Some(uid), 0)?;
    tx.commit()?;
    Ok(Json(value))
}
