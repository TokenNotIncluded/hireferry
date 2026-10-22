//! Operator-owned presentation state shared by HTTP, MCP and the browser.
//! Graph result handles retain a query and revision, not historical personal data.
use crate::{
    AppError, AppResult, AppState,
    access::{self, Principal},
    relations::{self, Filters, Focus, Grouping, Mode, Query as GraphQuery, Side},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const RESULT_TTL: i64 = 900;
const OP_TTL: i64 = 86_400;
const UI_TTL: i64 = 45;
const MAX_STATE_BYTES: usize = 32_768;

macro_rules! choice {
    ($name:ident, $default:ident, $($variant:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { #[default] $default, $($variant),+ }
    };
}
choice!(ViewMode, Overview, Focus, Matrix, List);
choice!(Ordering, Stable, Name, Degree);
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDisplay {
    // Early checkpoint clients used these names as automatic mode preferences.
    // Keep saved views readable; the advertised controls use the actual modes.
    #[default]
    #[serde(alias = "focused", alias = "bundled")]
    Auto,
    Hidden,
}
choice!(Presentation, Standard, Immersive);
choice!(OpenMode, Active, Create, Resume, Activate);
choice!(ResponseMode, Summary, Rows, State);
choice!(Fullscreen, Windowed, Active, RequiresUserGesture);
choice!(VisibilitySide, Left, Right, Edges);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hidden {
    pub left: Vec<String>,
    pub right: Vec<String>,
    pub edges: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewState {
    pub mode: ViewMode,
    pub filters: Filters,
    pub grouping: Grouping,
    pub ordering: Ordering,
    pub edge_display: EdgeDisplay,
    pub focus: Option<Focus>,
    pub hidden: Hidden,
    pub presentation: Presentation,
    pub fullscreen_requested: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    SetFilters {
        filters: Filters,
    },
    SetGrouping {
        grouping: Grouping,
    },
    SetMode {
        mode: ViewMode,
    },
    SetOrdering {
        ordering: Ordering,
    },
    SetEdgeDisplay {
        edge_display: EdgeDisplay,
    },
    SetFocus {
        focus: Option<Focus>,
    },
    SetVisibility {
        side: VisibilitySide,
        ids: Vec<String>,
        visible: bool,
    },
    SetPresentation {
        presentation: Presentation,
    },
    RequestFullscreen,
    Undo,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpenInput {
    pub mode: OpenMode,
    pub view_id: Option<String>,
    pub preset_id: Option<String>,
    pub name: Option<String>,
    pub operation_id: Option<String>,
    pub response: ResponseMode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PatchInput {
    pub if_view_rev: i64,
    pub operation_id: String,
    pub commands: Vec<Command>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AckInput {
    pub view_rev: i64,
    pub client_id: String,
    #[serde(default)]
    pub fullscreen: Fullscreen,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveInput {
    pub view_id: String,
    pub if_view_rev: i64,
    pub name: String,
    pub operation_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct GraphInput {
    pub view_id: Option<String>,
    pub query: Option<GraphQuery>,
    pub response: ResponseMode,
    pub fields: Vec<String>,
    pub max_bytes: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {
    pub result_id: String,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub fields: Vec<String>,
    pub max_bytes: Option<usize>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PollInput {
    view_id: Option<String>,
    after_rev: Option<i64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ListInput {
    limit: Option<i64>,
}

struct StoredView {
    id: String,
    name: String,
    rev: i64,
    state: ViewState,
    previous: Option<String>,
    ack_rev: Option<i64>,
    seen_at: i64,
    client_id: Option<String>,
    fullscreen: String,
}

pub fn migrate(db: &Connection) -> AppResult<()> {
    db.execute_batch("SAVEPOINT relation_views_v1;")?;
    let result = db.execute_batch(
        "CREATE TABLE IF NOT EXISTS relation_views(
           id TEXT PRIMARY KEY,owner_key TEXT NOT NULL,name TEXT NOT NULL DEFAULT '',
           view_rev INTEGER NOT NULL CHECK(view_rev>0),state_json TEXT NOT NULL,
           previous_json TEXT,ack_rev INTEGER,seen_at INTEGER NOT NULL DEFAULT 0,
           client_id TEXT,fullscreen TEXT NOT NULL DEFAULT 'windowed',
           created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS relation_views_owner ON relation_views(owner_key,updated_at DESC);
         CREATE TABLE IF NOT EXISTS relation_active_views(
           owner_key TEXT PRIMARY KEY,view_id TEXT NOT NULL REFERENCES relation_views(id) ON DELETE CASCADE);
         CREATE TABLE IF NOT EXISTS relation_view_operations(
           owner_key TEXT NOT NULL,operation_id TEXT NOT NULL,request_hash TEXT NOT NULL,
           response_json TEXT NOT NULL,created_at INTEGER NOT NULL,PRIMARY KEY(owner_key,operation_id));
         CREATE INDEX IF NOT EXISTS relation_operations_time ON relation_view_operations(created_at);
         CREATE TABLE IF NOT EXISTS relation_results(
           id TEXT PRIMARY KEY,owner_key TEXT NOT NULL,query_json TEXT NOT NULL,
           data_rev INTEGER NOT NULL,created_at INTEGER NOT NULL,expires_at INTEGER NOT NULL);
         CREATE INDEX IF NOT EXISTS relation_results_owner ON relation_results(owner_key,expires_at);",
    );
    match result {
        Ok(()) => {
            db.execute_batch("RELEASE relation_views_v1;")?;
            Ok(())
        }
        Err(error) => {
            let _ = db.execute_batch("ROLLBACK TO relation_views_v1; RELEASE relation_views_v1;");
            Err(error.into())
        }
    }
}

// The platform has one root administrator identity. Its browser and root AI
// tokens share views; distinct team members never do. Credentials are checked
// on every operation, including reads of idempotency receipts and handles.
fn owner(db: &Connection, p: &Principal) -> AppResult<String> {
    access::ensure_operator(db, p)?;
    if let Some(uid) = &p.user_id {
        return Ok(format!("manager:{uid}"));
    }
    if let Some(token) = p.actor.strip_prefix("staff_ai:") {
        let member: String = db.query_row(
            "SELECT member_id FROM team_ai_tokens WHERE id=?1",
            [token],
            |r| r.get(0),
        )?;
        return Ok(format!("team:{member}"));
    }
    if let Some(identity) = p
        .actor
        .strip_prefix("staff:")
        .or_else(|| p.actor.strip_prefix("staff_webmcp:"))
    {
        let (member, _) = identity
            .split_once(':')
            .ok_or_else(|| AppError::unauthorized("Invalid staff identity"))?;
        return Ok(format!("team:{member}"));
    }
    Ok("platform_admin".into())
}

fn text(value: &str, maximum: usize, label: &str) -> AppResult<()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(AppError::bad_request(format!(
            "{label} must be 1–{maximum} bytes without control characters"
        )));
    }
    Ok(())
}

fn load(db: &Connection, who: &str, id: &str) -> AppResult<StoredView> {
    text(id, 80, "view_id")?;
    let raw = db
        .query_row(
            "SELECT id,name,view_rev,state_json,previous_json,ack_rev,seen_at,client_id,fullscreen
         FROM relation_views WHERE id=?1 AND owner_key=?2",
            params![id, who],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| AppError::not_found("View unavailable"))?;
    Ok(StoredView {
        id: raw.0,
        name: raw.1,
        rev: raw.2,
        state: serde_json::from_str(&raw.3).map_err(AppError::internal)?,
        previous: raw.4,
        ack_rev: raw.5,
        seen_at: raw.6,
        client_id: raw.7,
        fullscreen: raw.8,
    })
}

fn active_id(db: &Connection, who: &str) -> AppResult<Option<String>> {
    Ok(db
        .query_row(
            "SELECT view_id FROM relation_active_views WHERE owner_key=?1",
            [who],
            |r| r.get(0),
        )
        .optional()?)
}

fn receipt(db: &Connection, who: &str, v: &StoredView, include_state: bool) -> AppResult<Value> {
    let active = active_id(db, who)?.as_deref() == Some(v.id.as_str());
    let status = if !active || v.seen_at < crate::now() - UI_TTL {
        "stored"
    } else if v.ack_rev == Some(v.rev) {
        "applied"
    } else {
        "queued"
    };
    let mut out = json!({
        "view_id":v.id,"view_rev":v.rev,"data_rev":relations::revision(db)?,"name":v.name,
        "active":active,
        "summary":{"mode":v.state.mode,"focus":v.state.focus,"edge_display":v.state.edge_display,
            "hidden_nodes":v.state.hidden.left.len()+v.state.hidden.right.len(),
            "hidden_edges":v.state.hidden.edges.len(),"presentation":v.state.presentation},
        "ui":{"status":status,"ack_view_rev":v.ack_rev,"client_id":v.client_id,
            "fullscreen":if v.state.fullscreen_requested && v.fullscreen!="active" {"requires_user_gesture"} else {v.fullscreen.as_str()}}
    });
    if include_state {
        out["state"] = serde_json::to_value(&v.state)?;
    }
    Ok(out)
}

fn fingerprint(kind: &str, payload: &impl Serialize) -> AppResult<String> {
    Ok(hex::encode(Sha256::digest(format!(
        "{kind}:{}",
        serde_json::to_string(payload)?
    ))))
}

fn replay(db: &Connection, who: &str, operation: &str, hash: &str) -> AppResult<Option<Value>> {
    text(operation, 128, "operation_id")?;
    db.execute(
        "DELETE FROM relation_view_operations WHERE created_at<?1",
        [crate::now() - OP_TTL],
    )?;
    let existing: Option<(String,String)> = db.query_row(
        "SELECT request_hash,response_json FROM relation_view_operations WHERE owner_key=?1 AND operation_id=?2",
        params![who,operation], |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional()?;
    match existing {
        Some((old, response)) if old == hash => Ok(Some(
            serde_json::from_str(&response).map_err(AppError::internal)?,
        )),
        Some(_) => Err(AppError::conflict(
            "idempotency_conflict: operation_id was used for a different request",
        )),
        None => Ok(None),
    }
}

fn remember(
    db: &Connection,
    who: &str,
    operation: &str,
    hash: &str,
    value: &Value,
) -> AppResult<()> {
    let count: i64 = db.query_row(
        "SELECT count(*) FROM relation_view_operations WHERE owner_key=?1",
        [who],
        |r| r.get(0),
    )?;
    if count >= 10_000 {
        return Err(AppError::unavailable(
            "View operation limit reached; retry after existing receipts expire",
        ));
    }
    db.execute(
        "INSERT INTO relation_view_operations VALUES(?1,?2,?3,?4,?5)",
        params![who, operation, hash, value.to_string(), crate::now()],
    )?;
    Ok(())
}

fn create(
    db: &Connection,
    who: &str,
    name: &str,
    state: &ViewState,
    activate: bool,
) -> AppResult<String> {
    let count: i64 = db.query_row(
        "SELECT count(*) FROM relation_views WHERE owner_key=?1",
        [who],
        |r| r.get(0),
    )?;
    if count >= 64 {
        return Err(AppError::bad_request(
            "At most 64 relation views can be saved per operator",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO relation_views(id,owner_key,name,view_rev,state_json,created_at,updated_at) VALUES(?1,?2,?3,1,?4,?5,?5)",
        params![id,who,name,serde_json::to_string(state)?,crate::now()])?;
    if activate {
        db.execute("INSERT INTO relation_active_views(owner_key,view_id) VALUES(?1,?2) ON CONFLICT(owner_key) DO UPDATE SET view_id=excluded.view_id", params![who,id])?;
    }
    Ok(id)
}

pub fn open(s: &AppState, p: &Principal, input: &OpenInput) -> AppResult<Value> {
    if input.response == ResponseMode::Rows {
        return Err(AppError::bad_request("Use response summary or state"));
    }
    let mut db = s.conn()?;
    let who = owner(&db, p)?;
    if input.mode == OpenMode::Resume {
        if input.preset_id.is_some() || input.name.is_some() || input.operation_id.is_some() {
            return Err(AppError::bad_request(
                "resume only reads view_id and response",
            ));
        }
        let id = input
            .view_id
            .as_deref()
            .ok_or_else(|| AppError::bad_request("resume requires view_id"))?;
        return receipt(
            &db,
            &who,
            &load(&db, &who, id)?,
            input.response == ResponseMode::State,
        );
    }
    if input.mode == OpenMode::Activate {
        if input.preset_id.is_some() || input.name.is_some() {
            return Err(AppError::bad_request(
                "activate only accepts view_id, operation_id and response",
            ));
        }
        let id = input
            .view_id
            .as_deref()
            .ok_or_else(|| AppError::bad_request("activate requires view_id"))?;
        let op = input
            .operation_id
            .as_deref()
            .ok_or_else(|| AppError::bad_request("activate requires operation_id"))?;
        let hash = fingerprint("open", input)?;
        let tx = db.transaction()?;
        let view = load(&tx, &who, id)?;
        if let Some(value) = replay(&tx, &who, op, &hash)? {
            tx.commit()?;
            return Ok(value);
        }
        relations::validate_focus(&tx, &query_from_view(&view.state))?;
        if active_id(&tx, &who)?.as_deref() != Some(id) {
            tx.execute("INSERT INTO relation_active_views(owner_key,view_id) VALUES(?1,?2) ON CONFLICT(owner_key) DO UPDATE SET view_id=excluded.view_id",params![who,id])?;
            // A render from an earlier activation cannot acknowledge this switch.
            tx.execute(
                "UPDATE relation_views SET ack_rev=NULL WHERE id=?1 AND owner_key=?2",
                params![id, who],
            )?;
        }
        let out = receipt(
            &tx,
            &who,
            &load(&tx, &who, id)?,
            input.response == ResponseMode::State,
        )?;
        remember(&tx, &who, op, &hash, &out)?;
        tx.commit()?;
        return Ok(out);
    }
    if input.view_id.is_some() {
        return Err(AppError::bad_request(
            "view_id is only used by resume or activate",
        ));
    }
    if input.mode == OpenMode::Active
        && (input.preset_id.is_some() || input.name.is_some() || input.operation_id.is_some())
    {
        return Err(AppError::bad_request(
            "active only accepts mode and response",
        ));
    }
    if let Some(name) = &input.name {
        text(name, 80, "name")?;
    }
    let hash = fingerprint("open", input)?;
    let tx = db.transaction()?;
    if input.mode == OpenMode::Create {
        let op = input
            .operation_id
            .as_deref()
            .ok_or_else(|| AppError::bad_request("create requires operation_id"))?;
        if let Some(value) = replay(&tx, &who, op, &hash)? {
            tx.commit()?;
            return Ok(value);
        }
    } else if let Some(id) = active_id(&tx, &who)? {
        let value = receipt(
            &tx,
            &who,
            &load(&tx, &who, &id)?,
            input.response == ResponseMode::State,
        )?;
        tx.commit()?;
        return Ok(value);
    }
    let state = match &input.preset_id {
        Some(id) => load(&tx, &who, id)?.state,
        None => ViewState::default(),
    };
    relations::validate_focus(&tx, &query_from_view(&state))?;
    let id = create(&tx, &who, input.name.as_deref().unwrap_or(""), &state, true)?;
    let out = receipt(
        &tx,
        &who,
        &load(&tx, &who, &id)?,
        input.response == ResponseMode::State,
    )?;
    if let Some(op) = &input.operation_id {
        remember(&tx, &who, op, &hash, &out)?;
    }
    tx.commit()?;
    Ok(out)
}

pub fn get_view(s: &AppState, p: &Principal, id: Option<&str>) -> AppResult<Value> {
    let db = s.conn()?;
    let who = owner(&db, p)?;
    let id = id
        .map(str::to_owned)
        .or(active_id(&db, &who)?)
        .ok_or_else(|| AppError::not_found("No active relation view"))?;
    receipt(&db, &who, &load(&db, &who, &id)?, true)
}

fn validated_state(state: &mut ViewState) -> AppResult<()> {
    let q = relations::normalized(&GraphQuery {
        filters: state.filters.clone(),
        grouping: state.grouping.clone(),
        focus: state.focus.clone(),
        ..Default::default()
    })?;
    state.filters = q.filters;
    state.focus = q.focus;
    for list in [
        &mut state.hidden.left,
        &mut state.hidden.right,
        &mut state.hidden.edges,
    ] {
        if list.len() > 200 {
            return Err(AppError::bad_request(
                "At most 200 hidden IDs per side; filter a group for a larger exclusion",
            ));
        }
        for id in list.iter() {
            text(id, 120, "entity ID")?;
        }
        list.sort();
        list.dedup();
    }
    if serde_json::to_vec(state)?.len() > MAX_STATE_BYTES {
        return Err(AppError::bad_request("View state exceeds 32 KiB"));
    }
    Ok(())
}

pub fn patch(s: &AppState, p: &Principal, id: &str, input: &PatchInput) -> AppResult<Value> {
    if input.if_view_rev < 1 || input.commands.is_empty() || input.commands.len() > 20 {
        return Err(AppError::bad_request(
            "Use a positive if_view_rev and 1–20 commands",
        ));
    }
    if input.commands.len() > 1 && input.commands.iter().any(|v| matches!(v, Command::Undo)) {
        return Err(AppError::bad_request("undo must be the sole command"));
    }
    let hash = fingerprint(&format!("patch:{id}"), input)?;
    let mut db = s.conn()?;
    let who = owner(&db, p)?;
    let tx = db.transaction()?;
    // Ownership is rechecked even when an idempotent result already exists.
    let current = load(&tx, &who, id)?;
    if let Some(value) = replay(&tx, &who, &input.operation_id, &hash)? {
        tx.commit()?;
        return Ok(value);
    }
    if current.rev != input.if_view_rev {
        return Err(AppError::conflict(format!(
            "view_revision_conflict: current view_rev is {}",
            current.rev
        )));
    }
    let old = serde_json::to_string(&current.state)?;
    let mut state = current.state;
    let is_undo = matches!(input.commands.first(), Some(Command::Undo));
    for command in &input.commands {
        match command {
            Command::SetFilters { filters } => state.filters = filters.clone(),
            Command::SetGrouping { grouping } => state.grouping = grouping.clone(),
            Command::SetMode { mode } => state.mode = *mode,
            Command::SetOrdering { ordering } => state.ordering = *ordering,
            Command::SetEdgeDisplay { edge_display } => state.edge_display = *edge_display,
            Command::SetFocus { focus } => state.focus = focus.clone(),
            Command::SetPresentation { presentation } => {
                state.presentation = *presentation;
                if *presentation == Presentation::Standard {
                    state.fullscreen_requested = false;
                }
            }
            Command::RequestFullscreen => state.fullscreen_requested = true,
            Command::SetVisibility { side, ids, visible } => {
                if ids.is_empty() || ids.len() > 100 {
                    return Err(AppError::bad_request(
                        "Use 1–100 IDs per visibility command",
                    ));
                }
                let bucket = match side {
                    VisibilitySide::Left => &mut state.hidden.left,
                    VisibilitySide::Right => &mut state.hidden.right,
                    VisibilitySide::Edges => &mut state.hidden.edges,
                };
                for id in ids {
                    text(id, 120, "entity ID")?;
                    if *visible {
                        bucket.retain(|v| v != id);
                    } else if !bucket.contains(id) {
                        bucket.push(id.clone());
                    }
                }
            }
            Command::Undo => {
                let previous = current
                    .previous
                    .as_deref()
                    .ok_or_else(|| AppError::conflict("No previous view change to undo"))?;
                state = serde_json::from_str(previous).map_err(AppError::internal)?;
            }
        }
    }
    validated_state(&mut state)?;
    // Validate the final batch, including an existing focus after filters change,
    // inside this same authorized transaction. No partially stored commands.
    relations::validate_focus(&tx, &query_from_view(&state))?;
    let serialized = serde_json::to_string(&state)?;
    let changed = old != serialized;
    if changed {
        tx.execute("UPDATE relation_views SET state_json=?1,previous_json=?2,view_rev=view_rev+1,updated_at=?3 WHERE id=?4 AND owner_key=?5 AND view_rev=?6",
            params![serialized,if is_undo {None}else{Some(old)},crate::now(),id,who,input.if_view_rev])?;
    }
    let mut out = receipt(&tx, &who, &load(&tx, &who, id)?, false)?;
    out["changed"] = json!(changed);
    out["operation_id"] = json!(input.operation_id);
    remember(&tx, &who, &input.operation_id, &hash, &out)?;
    tx.commit()?;
    Ok(out)
}

pub fn acknowledge(s: &AppState, p: &Principal, id: &str, input: &AckInput) -> AppResult<Value> {
    if p.via_ai {
        return Err(AppError::forbidden(
            "A browser render acknowledgement cannot be supplied by an AI tool",
        ));
    }
    text(&input.client_id, 80, "client_id")?;
    let db = s.conn()?;
    let who = owner(&db, p)?;
    let v = load(&db, &who, id)?;
    if input.view_rev != v.rev {
        return Err(AppError::conflict(
            "view_revision_conflict: acknowledge only the current rendered revision",
        ));
    }
    let fs = serde_json::to_value(input.fullscreen)?
        .as_str()
        .unwrap_or("windowed")
        .to_owned();
    db.execute("UPDATE relation_views SET ack_rev=?1,seen_at=?2,client_id=?3,fullscreen=?4 WHERE id=?5 AND owner_key=?6",
        params![input.view_rev,crate::now(),input.client_id,fs,id,who])?;
    receipt(&db, &who, &load(&db, &who, id)?, false)
}

pub fn save(s: &AppState, p: &Principal, input: &SaveInput) -> AppResult<Value> {
    text(&input.name, 80, "name")?;
    let hash = fingerprint("save", input)?;
    let mut db = s.conn()?;
    let who = owner(&db, p)?;
    let tx = db.transaction()?;
    let v = load(&tx, &who, &input.view_id)?;
    if let Some(value) = replay(&tx, &who, &input.operation_id, &hash)? {
        tx.commit()?;
        return Ok(value);
    }
    if v.rev != input.if_view_rev {
        return Err(AppError::conflict(
            "view_revision_conflict: resume before saving",
        ));
    }
    let id = create(&tx, &who, &input.name, &v.state, false)?;
    let out = json!({"preset_id":id,"name":input.name,"source_view_id":v.id,"source_view_rev":v.rev,"operation_id":input.operation_id});
    remember(&tx, &who, &input.operation_id, &hash, &out)?;
    tx.commit()?;
    Ok(out)
}

pub fn list(s: &AppState, p: &Principal, limit: i64) -> AppResult<Value> {
    if !(1..=64).contains(&limit) {
        return Err(AppError::bad_request("limit must be 1–64"));
    }
    let db = s.conn()?;
    let who = owner(&db, p)?;
    let active = active_id(&db, &who)?;
    let total: i64 = db.query_row(
        "SELECT count(*) FROM relation_views WHERE owner_key=?1",
        [&who],
        |r| r.get(0),
    )?;
    let items=db.prepare("SELECT id,name,view_rev,updated_at FROM relation_views WHERE owner_key=?1 ORDER BY updated_at DESC,id LIMIT ?2")?
        .query_map(params![who,limit],|r|Ok(json!({"view_id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"view_rev":r.get::<_,i64>(2)?,"updated_at":r.get::<_,i64>(3)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"active_view_id":active,"items":items,"total":total,"returned":items.len(),"remaining":total-items.len() as i64}),
    )
}

fn query_from_view(v: &ViewState) -> GraphQuery {
    GraphQuery {
        filters: v.filters.clone(),
        grouping: v.grouping.clone(),
        focus: v.focus.clone(),
        mode: if v.mode == ViewMode::Focus && v.focus.is_some() {
            Mode::Neighbors
        } else if v.mode == ViewMode::List {
            Mode::Nodes
        } else {
            Mode::Overview
        },
        side: if v.mode == ViewMode::List {
            Some(Side::Left)
        } else {
            None
        },
        ..Default::default()
    }
}

fn budget(value: Option<usize>) -> AppResult<usize> {
    let n = value.unwrap_or(8192);
    if !(512..=32768).contains(&n) {
        return Err(AppError::bad_request("max_bytes must be 512–32768"));
    }
    Ok(n)
}

const FIELDS: &[&str] = &[
    "id",
    "kind",
    "label",
    "side",
    "group_id",
    "role",
    "status",
    "relation_count",
    "headcount",
    "salary_cents",
    "currency",
    "owner_id",
    "owner_name",
    "seeker_id",
    "job_id",
    "edge_id",
    "match_status",
    "left_group_id",
    "right_group_id",
    "count",
    "seekers",
    "jobs",
    "possible_pairs",
    "state_counts",
    "created_at",
    "updated_at",
];
const DEFAULT_FIELDS: &[&str] = &[
    "id",
    "kind",
    "label",
    "side",
    "group_id",
    "relation_count",
    "status",
    "edge_id",
    "seeker_id",
    "job_id",
    "match_status",
    "left_group_id",
    "right_group_id",
    "count",
];

fn fields(input: &[String]) -> AppResult<Vec<String>> {
    if input.len() > 16 || input.iter().any(|v| !FIELDS.contains(&v.as_str())) {
        return Err(AppError::bad_request(
            "Use at most 16 advertised summary fields",
        ));
    }
    let mut out = if input.is_empty() {
        DEFAULT_FIELDS.iter().map(|v| (*v).to_owned()).collect()
    } else {
        input.to_vec()
    };
    out.extend(["id".into(), "kind".into()]);
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn graph_query(s: &AppState, p: &Principal, input: &GraphInput) -> AppResult<Value> {
    let max = budget(input.max_bytes)?;
    fields(&input.fields)?;
    if input.response == ResponseMode::State {
        return Err(AppError::bad_request(
            "Graph response must be summary or rows",
        ));
    }
    if input.response == ResponseMode::Summary && !input.fields.is_empty() {
        return Err(AppError::bad_request(
            "Fields require response rows or hf_graph_read",
        ));
    }
    let mut q = {
        let db = s.conn()?;
        let who = owner(&db, p)?;
        let view = input
            .view_id
            .as_deref()
            .map(|id| load(&db, &who, id))
            .transpose()?;
        input
            .query
            .clone()
            .unwrap_or_else(|| view.map(|v| query_from_view(&v.state)).unwrap_or_default())
    };
    if q.cursor.is_some() {
        return Err(AppError::bad_request(
            "Start a query without cursor; use hf_graph_read to page its result",
        ));
    }
    q = relations::normalized(&q)?;
    let original_limit = q.limit;
    let mut brief = q.clone();
    brief.limit = Some(1);
    let graph = relations::query(s, p, &brief)?;
    let rev = graph["data_rev"]
        .as_i64()
        .ok_or_else(|| AppError::internal("Missing graph revision"))?;
    q.data_rev = Some(rev);
    let result_id = uuid::Uuid::new_v4().to_string();
    let mut out = json!({"result_id":result_id,"data_rev":rev,"mode":q.mode,"expires_at":crate::now()+RESULT_TTL,
        "summary":graph.get("summary").cloned().unwrap_or_else(||json!({"total":graph["total"],"state_counts":graph["state_counts"]}))});
    if serde_json::to_vec(&out)?.len() > max {
        return Err(AppError::bad_request(
            "response_budget_exceeded: increase max_bytes",
        ));
    }
    {
        let db = s.conn()?;
        let who = owner(&db, p)?;
        if relations::revision(&db)? != rev {
            return Err(AppError::conflict(
                "resync_required: graph changed during query",
            ));
        }
        db.execute(
            "DELETE FROM relation_results WHERE expires_at<=?1",
            [crate::now()],
        )?;
        let count: i64 = db.query_row(
            "SELECT count(*) FROM relation_results WHERE owner_key=?1",
            [&who],
            |r| r.get(0),
        )?;
        if count >= 128 {
            return Err(AppError::unavailable(
                "Too many active query handles; reuse a handle or wait for expiry",
            ));
        }
        db.execute(
            "INSERT INTO relation_results VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                result_id,
                who,
                serde_json::to_string(&q)?,
                rev,
                crate::now(),
                crate::now() + RESULT_TTL
            ],
        )?;
    }
    if input.response == ResponseMode::Rows {
        out = graph_read(
            s,
            p,
            &ReadInput {
                result_id,
                cursor: None,
                limit: original_limit,
                fields: input.fields.clone(),
                max_bytes: Some(max),
            },
        )?;
    }
    Ok(out)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReadCursor {
    result_id: String,
    data_rev: i64,
    fields_hash: String,
    backend: Option<String>,
    offset: usize,
}

fn normalized_rows(graph: &Value, mode: Mode) -> Vec<Value> {
    if mode == Mode::Overview {
        let mut rows = Vec::new();
        for side in ["left", "right"] {
            for group in graph["groups"][side].as_array().into_iter().flatten() {
                let mut row = group.clone();
                row["kind"] = json!("group");
                row["side"] = json!(side);
                rows.push(row);
            }
        }
        for bundle in graph["bundles"].as_array().into_iter().flatten() {
            let mut row = bundle.clone();
            row["kind"] = json!("bundle");
            row["id"] = json!(format!(
                "{}:{}",
                row["left_group_id"].as_str().unwrap_or(""),
                row["right_group_id"].as_str().unwrap_or("")
            ));
            rows.push(row);
        }
        rows
    } else {
        graph["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|raw| {
                if mode == Mode::Neighbors {
                    let mut row = raw["node"].clone();
                    row["kind"] = json!("neighbor");
                    row["edge_id"] = raw["edge"]["id"].clone();
                    for key in ["seeker_id", "job_id"] {
                        row[key] = raw["edge"][key].clone();
                    }
                    row["match_status"] = raw["edge"]["status"].clone();
                    row
                } else if mode == Mode::Edges {
                    let mut row = raw.clone();
                    row["kind"] = json!("edge");
                    row
                } else {
                    raw.clone()
                }
            })
            .collect()
    }
}

pub fn graph_read(s: &AppState, p: &Principal, input: &ReadInput) -> AppResult<Value> {
    let max = budget(input.max_bytes)?;
    let projection = fields(&input.fields)?;
    let field_hash = fingerprint("fields", &projection)?;
    let mut limit = input.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err(AppError::bad_request("limit must be 1–100"));
    }
    text(&input.result_id, 80, "result_id")?;
    let (mut query, rev) = {
        let db = s.conn()?;
        let who = owner(&db, p)?;
        let record:Option<(String,i64)>=db.query_row("SELECT query_json,data_rev FROM relation_results WHERE id=?1 AND owner_key=?2 AND expires_at>?3",
            params![input.result_id,who,crate::now()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (raw, rev) = record.ok_or_else(|| {
            AppError::not_found("result_expired_or_unavailable: run hf_graph_query again")
        })?;
        if relations::revision(&db)? != rev {
            return Err(AppError::conflict(
                "resync_required: result revision changed; run hf_graph_query again",
            ));
        }
        (
            serde_json::from_str::<GraphQuery>(&raw).map_err(AppError::internal)?,
            rev,
        )
    };
    let mut offset = 0;
    if let Some(raw) = &input.cursor {
        if raw.len() > 4096 {
            return Err(AppError::bad_request("Invalid cursor"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| AppError::bad_request("Invalid cursor"))?;
        let cursor: ReadCursor =
            serde_json::from_slice(&bytes).map_err(|_| AppError::bad_request("Invalid cursor"))?;
        if cursor.result_id != input.result_id
            || cursor.data_rev != rev
            || cursor.fields_hash != field_hash
        {
            return Err(AppError::bad_request(
                "Cursor does not belong to this result and field projection",
            ));
        }
        query.cursor = cursor.backend;
        offset = cursor.offset;
        if query.mode != Mode::Overview && offset != 0 {
            return Err(AppError::bad_request("Invalid cursor offset"));
        }
    }
    loop {
        query.limit = Some(limit);
        let graph = relations::query(s, p, &query)?;
        let rows = normalized_rows(&graph, query.mode);
        if offset > rows.len() {
            return Err(AppError::bad_request("Invalid overview cursor"));
        }
        let total = if query.mode == Mode::Overview {
            rows.len() as i64
        } else {
            graph["total"].as_i64().unwrap_or(0)
        };
        let chosen: Vec<Value> = rows
            .iter()
            .skip(offset)
            .take(limit as usize)
            .map(|row| {
                let mut item = serde_json::Map::new();
                for key in &projection {
                    if let Some(value) = row.get(key) {
                        item.insert(key.clone(), value.clone());
                    }
                }
                Value::Object(item)
            })
            .collect();
        let returned = chosen.len();
        let remaining = if query.mode == Mode::Overview {
            total - offset as i64 - returned as i64
        } else {
            graph["remaining"].as_i64().unwrap_or(0)
        };
        let backend = graph["next_cursor"].as_str().map(str::to_owned);
        let next = if remaining > 0 {
            Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&ReadCursor {
                result_id: input.result_id.clone(),
                data_rev: rev,
                fields_hash: field_hash.clone(),
                backend,
                offset: if query.mode == Mode::Overview {
                    offset + returned
                } else {
                    0
                },
            })?))
        } else {
            None
        };
        let out = json!({"result_id":input.result_id,"data_rev":rev,"mode":query.mode,
            "total":total,"returned":returned,"remaining":remaining,"items":chosen,"next_cursor":next});
        if serde_json::to_vec(&out)?.len() <= max {
            return Ok(out);
        }
        if limit == 1 {
            return Err(AppError::bad_request(
                "response_budget_exceeded: select fewer fields or increase max_bytes",
            ));
        }
        limit = (limit / 2).max(1);
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/relations/views", post(http_open))
        .route("/api/admin/relations/views/active", get(http_active))
        .route(
            "/api/admin/relations/views/{id}",
            get(http_get).patch(http_patch),
        )
        .route("/api/admin/relations/views/{id}/ack", post(http_ack))
        .route("/api/admin/relations/tools", get(http_tools))
        .route("/api/admin/relations/tools/{name}", post(http_tool_call))
}

async fn http_open(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<OpenInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, true)?;
    Ok(Json(open(&s, &p, &input)?))
}
fn poll(s: &AppState, p: &Principal, id: Option<&str>, input: &PollInput) -> AppResult<Value> {
    let mut value = get_view(s, p, id)?;
    // A browser poll is a liveness signal, never a rendering acknowledgement.
    if !p.via_ai {
        let db = s.conn()?;
        let who = owner(&db, p)?;
        db.execute(
            "UPDATE relation_views SET seen_at=?1 WHERE id=?2 AND owner_key=?3",
            params![crate::now(), value["view_id"].as_str(), who],
        )?;
    }
    if input.view_id.as_deref() == value["view_id"].as_str()
        && input.after_rev == value["view_rev"].as_i64()
    {
        value.as_object_mut().unwrap().remove("state");
        value["unchanged"] = json!(true);
    }
    Ok(value)
}
async fn http_active(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(input): Query<PollInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, false)?;
    Ok(Json(poll(&s, &p, None, &input)?))
}
async fn http_get(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<PollInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, false)?;
    Ok(Json(poll(&s, &p, Some(&id), &input)?))
}
async fn http_patch(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<PatchInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, true)?;
    Ok(Json(patch(&s, &p, &id, &input)?))
}
async fn http_ack(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<AckInput>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, true)?;
    Ok(Json(acknowledge(&s, &p, &id, &input)?))
}
async fn http_tools(State(s): State<AppState>, headers: HeaderMap) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, false)?;
    owner(&*s.conn()?, &p)?;
    Ok(Json(json!({"tools":tools()})))
}
async fn http_tool_call(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(args): Json<Value>,
) -> AppResult<Json<Value>> {
    let p = access::principal(&s, &headers, true)?;
    let value = tokio::task::spawn_blocking(move || call(&s, &p, &name, args))
        .await
        .map_err(AppError::internal)??;
    Ok(Json(value))
}

pub fn call(s: &AppState, p: &Principal, name: &str, mut args: Value) -> AppResult<Value> {
    {
        let db = s.conn()?;
        owner(&db, p)?;
    }
    match name {
        "hf_view_open" => open(s, p, &serde_json::from_value(args)?),
        "hf_view_patch" => {
            let id = args
                .as_object_mut()
                .and_then(|v| v.remove("view_id"))
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or_else(|| AppError::bad_request("view_id is required"))?;
            patch(s, p, &id, &serde_json::from_value(args)?)
        }
        "hf_graph_query" => graph_query(s, p, &serde_json::from_value(args)?),
        "hf_graph_read" => graph_read(s, p, &serde_json::from_value(args)?),
        "hf_view_save" => save(s, p, &serde_json::from_value(args)?),
        "hf_view_list" => {
            let input: ListInput = serde_json::from_value(args)?;
            list(s, p, input.limit.unwrap_or(20))
        }
        _ => Err(AppError::not_found("Unknown relation workbench tool")),
    }
}

fn object(props: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}

/// Compact input schemas shared by MCP and authenticated browser WebMCP.
pub fn tools() -> Vec<Value> {
    let id = json!({"type":"string","minLength":1,"maxLength":80});
    let filters = object(
        json!({
            "user_q":{"type":["string","null"],"maxLength":120},"job_q":{"type":["string","null"],"maxLength":120},
            "role":{"enum":[null,"seeker","employer","unassigned"]},"account_status":{"enum":[null,"active","suspended"]},
            "job_status":{"enum":[null,"draft","published","closed","deleted"]},
            "match_status":{"enum":[null,"introduced","interview","hired","declined","unknown"]},
            "employer_id":{"type":["string","null"],"maxLength":80},
            "user_ids":{"type":["array","null"],"maxItems":200,"items":id},"job_ids":{"type":["array","null"],"maxItems":200,"items":id},
            "left_group_ids":{"type":["array","null"],"maxItems":20,"items":{"type":"string","maxLength":120}},
            "right_group_ids":{"type":["array","null"],"maxItems":20,"items":{"type":"string","maxLength":120}}
        }),
        &[],
    );
    let grouping = object(
        json!({"left":{"enum":["role","industry","roles"]},"right":{"enum":["status","owner"]}}),
        &[],
    );
    let focus = object(
        json!({"side":{"enum":["left","right"]},"id":id}),
        &["side", "id"],
    );
    let mut commands = Vec::new();
    for (op, props, required) in [
        (
            "set_filters",
            json!({"filters":{"$ref":"#/$defs/filters"}}),
            vec!["filters"],
        ),
        (
            "set_grouping",
            json!({"grouping":{"$ref":"#/$defs/grouping"}}),
            vec!["grouping"],
        ),
        (
            "set_mode",
            json!({"mode":{"enum":["overview","focus","matrix","list"]}}),
            vec!["mode"],
        ),
        (
            "set_ordering",
            json!({"ordering":{"enum":["stable","name","degree"],"description":"Group order in overview/matrix; loaded-page order in focus/list. Scrolling preserves order."}}),
            vec!["ordering"],
        ),
        (
            "set_edge_display",
            json!({"edge_display":{"enum":["auto","hidden"],"description":"Automatic lines for the selected view mode, or hide all lines. Use set_mode for focus or grouped overview."}}),
            vec!["edge_display"],
        ),
        (
            "set_focus",
            json!({"focus":{"oneOf":[{"$ref":"#/$defs/focus"},{"type":"null"}]}}),
            vec!["focus"],
        ),
        (
            "set_visibility",
            json!({"side":{"enum":["left","right","edges"]},"ids":{"type":"array","minItems":1,"maxItems":100,"items":id},"visible":{"type":"boolean"}}),
            vec!["side", "ids", "visible"],
        ),
        (
            "set_presentation",
            json!({"presentation":{"enum":["standard","immersive"]}}),
            vec!["presentation"],
        ),
        ("request_fullscreen", json!({}), vec![]),
        ("undo", json!({}), vec![]),
    ] {
        let mut props = props;
        props["op"] = json!({"const":op});
        let mut required = required;
        required.push("op");
        commands.push(object(props, &required));
    }
    let query = object(
        json!({"mode":{"enum":["overview","nodes","neighbors","edges"]},"filters":{"$ref":"#/$defs/filters"},
        "grouping":{"$ref":"#/$defs/grouping"},"side":{"enum":[null,"left","right"]},
        "focus":{"oneOf":[{"$ref":"#/$defs/focus"},{"type":"null"}]},
        "limit":{"type":"integer","minimum":1,"maximum":100},"data_rev":{"type":"integer","minimum":0}}),
        &[],
    );
    let field_schema = json!({"type":"array","maxItems":16,"items":{"enum":FIELDS}});
    let byte_schema = json!({"type":"integer","minimum":512,"maximum":32768,"default":8192});
    let op_schema = json!({"type":"string","minLength":1,"maxLength":128});
    let mut patch_schema = object(
        json!({"view_id":id,"if_view_rev":{"type":"integer","minimum":1},"operation_id":op_schema,
        "commands":{"type":"array","minItems":1,"maxItems":20,"items":{"oneOf":commands}}}),
        &["view_id", "if_view_rev", "operation_id", "commands"],
    );
    patch_schema["$defs"] = json!({"filters":filters,"grouping":grouping,"focus":focus});
    let mut graph_schema = object(
        json!({"view_id":id,"query":query,"response":{"enum":["summary","rows"],"default":"summary"},"fields":field_schema,"max_bytes":byte_schema}),
        &[],
    );
    graph_schema["$defs"] = json!({"filters":filters,"grouping":grouping,"focus":focus});
    let schemas = [
        (
            "hf_view_open",
            "Open the active workbench or resume view_id without mutation. Create clones preset_id; activate switches to existing view_id without copying or revising. Create/activate require operation_id. Default summary; response=state reads configuration.",
            object(
                json!({"mode":{"enum":["active","create","resume","activate"],"default":"active"},"view_id":id,"preset_id":id,"name":{"type":"string","maxLength":80},"operation_id":op_schema,"response":{"enum":["summary","state"],"default":"summary"}}),
                &[],
            ),
            false,
        ),
        (
            "hf_view_patch",
            "Atomically change presentation only. Reuse operation_id for retries; stale if_view_rev conflicts. Undo is one-step and must stand alone. Applied requires browser acknowledgement; native fullscreen requires a user gesture.",
            patch_schema,
            false,
        ),
        (
            "hf_graph_query",
            "Query a bounded relation graph. Summary returns result_id without rows; an explicit query overrides the view configuration. Handles expire after 15 minutes and require resync if underlying data changes.",
            graph_schema,
            true,
        ),
        (
            "hf_graph_read",
            "Read projected, bounded result rows. Cursor binds result and fields; the current data revision must still match. Default 20 rows and 8 KiB, shrinking pages to fit without skipping rows.",
            object(
                json!({"result_id":id,"cursor":{"type":"string","maxLength":4096},"limit":{"type":"integer","minimum":1,"maximum":100,"default":20},"fields":field_schema,"max_bytes":byte_schema}),
                &["result_id"],
            ),
            true,
        ),
        (
            "hf_view_save",
            "Save an independent named preset without changing the source or active view. Retry with the same operation_id. Activate its preset_id as view_id to edit in place, or create a copy with preset_id.",
            object(
                json!({"view_id":id,"if_view_rev":{"type":"integer","minimum":1},"name":{"type":"string","minLength":1,"maxLength":80},"operation_id":op_schema}),
                &["view_id", "if_view_rev", "name", "operation_id"],
            ),
            false,
        ),
        (
            "hf_view_list",
            "List only this operator's saved views and active view ID, without graph records or configuration bodies.",
            object(
                json!({"limit":{"type":"integer","minimum":1,"maximum":64,"default":20}}),
                &[],
            ),
            true,
        ),
    ];
    schemas.into_iter().map(|(name,description,input,read)|json!({"name":name,"description":description,"inputSchema":input,
        "annotations":{"readOnlyHint":read,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}})).collect()
}
