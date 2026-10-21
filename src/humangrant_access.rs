//! HumanGrant permissions at the existing HireFerry business boundary.
//!
//! `self` means the grantor's currently accessible collection, never all users.
//! A single-resource permission never authorizes a collection query. Checks run
//! while AppState holds the same database lock used by the business operation.
use crate::{
    AppError, AppResult, AppState, access,
    humangrant::{GrantContext, Permission},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

pub const ACTIONS: &[&str] = &[
    "profile.read",
    "profile.write",
    "jobs.read",
    "jobs.write",
    "people.read",
    "orders.read",
    "chat.read",
    "chat.write",
    "matches.read",
    "mandate.read",
];

/// Request-local context is immutable and is never accepted from browser input.
#[derive(Clone)]
pub struct Execution {
    pub grant: GrantContext,
    pub tool: Option<String>,
    pub arguments: Value,
}

pub fn grantable(db: &Connection, uid: &str, permission: &Permission) -> AppResult<bool> {
    let role: Option<String> = db
        .query_row(
            "SELECT role FROM users WHERE id=?1 AND banned=0",
            [uid],
            |row| row.get(0),
        )
        .optional()?;
    let Some(role) = role else {
        return Ok(false);
    };
    if access::eligibility(db, uid)?["eligible"] != true {
        return Ok(false);
    }
    let resource = permission.resource.as_str();
    let collection = resource == "self";
    match permission.action.as_str() {
        "profile.read" | "profile.write" | "matches.read" => Ok(collection),
        "mandate.read" => Ok(collection && role == "seeker"),
        "jobs.read" => Ok(collection || access::can_read_job(db, uid, resource)?),
        "jobs.write" => {
            if role != "employer" {
                return Ok(false);
            }
            Ok(collection
                || db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM jobs WHERE id=?1 AND owner_id=?2 AND status<>'deleted')",
                    params![resource, uid],
                    |row| row.get(0),
                )?)
        }
        "people.read" => {
            if collection {
                return Ok(role == "employer");
            }
            let seeker: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM users WHERE id=?1 AND role='seeker' AND banned=0)",
                [resource],
                |row| row.get(0),
            )?;
            Ok(seeker && (resource == uid || access::matched_pair(db, uid, resource)?))
        }
        "orders.read" => Ok(collection
            || db.query_row(
                "SELECT EXISTS(SELECT 1 FROM orders WHERE id=?1 AND user_id=?2)",
                params![resource, uid],
                |row| row.get(0),
            )?),
        "chat.read" | "chat.write" => Ok(collection || conversation_allowed(db, uid, resource)?),
        _ => Ok(false),
    }
}

fn conversation_allowed(db: &Connection, uid: &str, resource: &str) -> AppResult<bool> {
    if resource.strip_prefix("platform_") == Some(uid) {
        return Ok(true);
    }
    let Some(id) = resource.strip_prefix("match_") else {
        return Ok(false);
    };
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM matches m JOIN jobs j ON j.id=m.job_id JOIN users sk ON sk.id=m.seeker_id JOIN users er ON er.id=j.owner_id WHERE m.id=?1 AND m.status IN ('introduced','interview','hired') AND j.status<>'deleted' AND sk.banned=0 AND er.banned=0 AND (m.seeker_id=?2 OR j.owner_id=?2))",
        params![id, uid],
        |row| row.get(0),
    )?)
}

/// The second item is the resource argument, or None for a collection operation.
fn requirement(tool: &str) -> Option<(&'static str, Option<&'static str>)> {
    Some(match tool {
        "get_profile" => ("profile.read", None),
        "update_profile" | "set_avatar_svg" | "set_image_avatar" => ("profile.write", None),
        "list_jobs" | "list_my_jobs" => ("jobs.read", None),
        "get_job" => ("jobs.read", Some("id")),
        "create_job" => ("jobs.write", None),
        "edit_job" | "close_job" => ("jobs.write", Some("id")),
        "list_people" => ("people.read", None),
        "get_person" => ("people.read", Some("id")),
        "list_orders" => ("orders.read", None),
        "get_order" => ("orders.read", Some("id")),
        "list_matches" => ("matches.read", None),
        "get_mandate" | "list_outreach" => ("mandate.read", None),
        "list_conversations" | "get_updates" | "open_platform_conversation" => ("chat.read", None),
        "get_messages" => ("chat.read", Some("conversation_id")),
        "send_message" | "mark_read" => ("chat.write", Some("conversation_id")),
        // Account role, new outreach consent, checkout and administrator powers
        // require their existing human flows; HumanGrant cannot delegate them.
        _ => return None,
    })
}

pub fn authorize_tool(
    db: &Connection,
    grant: &GrantContext,
    tool: &str,
    arguments: &Value,
) -> AppResult<()> {
    let current = crate::humangrant::validate_grant(db, grant)?;
    let (action, argument) = requirement(tool)
        .ok_or_else(|| AppError::forbidden("This operation cannot be delegated with HumanGrant"))?;
    let resource = match argument {
        Some(key) => arguments[key]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| AppError::bad_request(format!("{key} is required")))?,
        None => "self",
    };
    let permitted = current.permissions.iter().any(|permission| {
        permission.action == action
            && (permission.resource == resource || permission.resource == "self")
    });
    if !permitted
        || !grantable(
            db,
            &current.user_id,
            &Permission {
                action: action.into(),
                resource: resource.into(),
            },
        )?
    {
        return Err(AppError::forbidden(
            "The human did not grant this action and resource",
        ));
    }
    Ok(())
}

/// Discovery is a helpful subset; authorize_tool remains the enforcement point.
pub fn tools(state: &AppState, grant: &GrantContext) -> Vec<Value> {
    crate::mcp::configured_tools(state, false)
        .into_iter()
        .filter_map(|mut tool| {
            let (action, argument) = requirement(tool["name"].as_str()?)?;
            let resources: Vec<_> = grant
                .permissions
                .iter()
                .filter(|permission| permission.action == action)
                .map(|permission| permission.resource.clone())
                .collect();
            let collection = resources.iter().any(|resource| resource == "self");
            if resources.is_empty() || (argument.is_none() && !collection) {
                return None;
            }
            if let Some(argument) = argument {
                if !collection {
                    tool["inputSchema"]["properties"][argument]["enum"] =
                        serde_json::json!(resources);
                }
            }
            Some(tool)
        })
        .collect()
}
