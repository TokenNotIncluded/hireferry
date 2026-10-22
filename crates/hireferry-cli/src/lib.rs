//! Network-only client: no dependency on the HireFerry service or database.
pub mod callback;
pub mod defaults;
pub mod error;
pub mod keyring;
pub mod login;
pub mod server;
pub mod settings;
pub mod storage;

use clap::{Parser, Subcommand};
use error::{Error, Result};
use serde_json::{Value, json};
use server::{Server, UserInfo};
use settings::{Settings, StorageChoice};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use storage::{AccountSubject, Credential, Store};

#[derive(Parser)]
#[command(
    name = "hfy",
    version,
    about = "Sign in to HireFerry from your terminal."
)]
pub struct Cli {
    /// Print one JSON result on stdout. Prompts stay on stderr.
    #[arg(long, global = true)]
    pub json: bool,
    /// Server origin; overrides HFY_SERVER and the client config.toml.
    #[arg(long, global = true)]
    pub server: Option<String>,
    /// Public OAuth client ID; never a client secret.
    #[arg(long, global = true)]
    pub client_id: Option<String>,
    /// Request identity only, member MCP access, or administrator access.
    #[arg(long, global = true, value_parser = ["identity", "mcp", "admin"])]
    pub scope: Option<String>,
    /// Preferred credential store.
    #[arg(long, global = true, value_enum)]
    pub storage: Option<StorageChoice>,
    /// Private directory containing client config.toml and connection metadata.
    #[arg(long, global = true)]
    pub config_dir: Option<PathBuf>,
    /// Timeout for each API request, in seconds.
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..=120))]
    pub request_timeout_seconds: Option<u64>,
    /// Minimum device-poll interval; server limits still apply.
    #[arg(long, global = true, value_parser = clap::value_parser!(u64).range(1..=300))]
    pub poll_interval_seconds: Option<u64>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage this CLI connection.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Show the identity verified by the server.
    Whoami,
    /// Show current permissions and AI eligibility reported by the server.
    Capabilities,
    /// Send a quick report or review the bounded administrator inbox.
    Feedback {
        #[command(subcommand)]
        command: FeedbackCommand,
    },
}

#[derive(Subcommand)]
pub enum FeedbackCommand {
    /// Send short plain-text feedback with your current CLI identity.
    Send {
        message: String,
        #[arg(long, default_value = "bug", value_parser = ["bug", "idea", "other"])]
        category: String,
    },
    /// List recent reports (requires --scope admin).
    List {
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..=100))]
        limit: Option<i64>,
        #[arg(long, value_parser = clap::value_parser!(i64).range(1..))]
        before_id: Option<i64>,
        #[arg(long, value_parser = ["open", "resolved"])]
        status: Option<String>,
        #[arg(long, value_parser = ["bug", "idea", "other"])]
        category: Option<String>,
        #[arg(long, value_parser = ["web", "cli"])]
        source: Option<String>,
    },
    /// Read one report (requires --scope admin).
    Get {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
    },
    /// Mark a report resolved (or reopen it with --reopen); administrator only.
    Resolve {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long)]
        reopen: bool,
    },
}

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Reuse a valid connection, or request browser/device-code sign-in.
    Login(login::LoginArgs),
    /// Check the saved connection with the server.
    Status,
    /// Rotate saved tokens now; normally done automatically before expiry.
    Refresh,
    /// Revoke this connection and remove credentials from this device.
    Logout,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub async fn run(cli: Cli) -> Result<Value> {
    let (settings, directory) = Settings::load(&cli)?;
    let server = Server::with_settings(&settings)?;
    let store = Store::new(
        directory,
        &server.origin,
        &server.client_id,
        settings.storage,
    );
    match cli.command {
        Command::Auth {
            command: AuthCommand::Login(args),
        } => {
            {
                let _guard = store.lock().await?;
                if let Some(mut credential) = store.load()? {
                    if scope_covers(&credential.scope, &server.scope)
                        && !credential.refresh_pending
                        && credential.refresh_expires_at > now()
                    {
                        match verified_identity(&server, &store, &mut credential, false).await {
                            Ok((identity, _)) => return signed_in(&server, &store, identity, true),
                            Err(error)
                                if matches!(
                                    error.code,
                                    "not_authenticated"
                                        | "session_revoked"
                                        | "session_expired"
                                        | "access_denied"
                                ) => {}
                            // Never turn an uncertain network result into a new
                            // authorization or an unsafe refresh retry.
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
            let mut credential =
                login::login(&server, &args, settings.auth_timeout_seconds).await?;
            let identity = match server.userinfo(&credential.access_token).await {
                Ok(identity) if identity.scope == credential.scope => identity,
                Ok(_) => {
                    let _ = server.revoke(&credential.refresh_token).await;
                    return Err(Error::invalid_response());
                }
                Err(error) => {
                    let _ = server.revoke(&credential.refresh_token).await;
                    return Err(error);
                }
            };
            credential.account = Some(account_subject(&identity));
            credential.expires_at = identity.expires_at;
            let _guard = store.lock().await?;
            let previous = store.load()?;
            if let Err(error) = store.save(&credential) {
                let _ = server.revoke(&credential.refresh_token).await;
                return Err(error);
            }
            if let Some(previous) = previous {
                if previous.refresh_token != credential.refresh_token
                    && server.revoke(&previous.refresh_token).await.is_err()
                {
                    eprintln!(
                        "Signed in. The previous connection could not be revoked; remove it from the website's connections page."
                    );
                }
            }
            signed_in(&server, &store, identity, false)
        }
        Command::Auth {
            command: AuthCommand::Status,
        } => {
            let _guard = store.lock().await?;
            let Some(mut credential) = store.load()? else {
                return Ok(
                    json!({"authenticated":false,"server":server.origin,"client_id":server.client_id,"reason":"no_saved_credentials"}),
                );
            };
            match verified_identity(&server, &store, &mut credential, false).await {
                Ok((identity, _)) => signed_in(&server, &store, identity, true),
                Err(error)
                    if matches!(
                        error.code,
                        "not_authenticated"
                            | "access_denied"
                            | "session_revoked"
                            | "session_expired"
                    ) =>
                {
                    Ok(
                        json!({"authenticated":false,"server":server.origin,"client_id":server.client_id,"reason":error.code}),
                    )
                }
                Err(error) => Err(error),
            }
        }
        Command::Whoami
        | Command::Capabilities
        | Command::Auth {
            command: AuthCommand::Refresh,
        } => {
            let capabilities_only = matches!(cli.command, Command::Capabilities);
            let force = matches!(
                cli.command,
                Command::Auth {
                    command: AuthCommand::Refresh
                }
            );
            let _guard = store.lock().await?;
            let mut credential = store.load()?.ok_or_else(Error::signed_out)?;
            let (identity, refreshed) =
                verified_identity(&server, &store, &mut credential, force).await?;
            if capabilities_only {
                Ok(
                    json!({"server":server.origin,"client_id":server.client_id,"scope":identity.scope,
                    "ai_eligibility":identity.ai_eligibility,"capabilities":identity.capabilities}),
                )
            } else {
                Ok(
                    json!({"server":server.origin,"client_id":server.client_id,"identity":identity,"storage":store.info()?,"refreshed":refreshed}),
                )
            }
        }
        Command::Feedback { command } => {
            let _guard = store.lock().await?;
            let mut credential = store.load()?.ok_or_else(Error::signed_out)?;
            let (identity, _) = verified_identity(&server, &store, &mut credential, false).await?;
            if !matches!(&command, FeedbackCommand::Send { .. })
                && (!identity.admin || !identity.capabilities.admin)
            {
                return Err(Error::new(
                    "admin_required",
                    "Run hfy --scope admin auth login before reviewing feedback.",
                ));
            }
            let data = match command {
                FeedbackCommand::Send { message, category } => {
                    if !(2..=2000).contains(&message.trim().chars().count())
                        || message.len() > 8000
                        || message
                            .chars()
                            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                    {
                        return Err(Error::new(
                            "invalid_feedback",
                            "Use 2–2000 characters of plain text.",
                        ));
                    }
                    server
                        .submit_feedback(&credential.access_token, &category, &message)
                        .await?
                }
                FeedbackCommand::List {
                    limit,
                    before_id,
                    status,
                    category,
                    source,
                } => {
                    server
                        .list_feedback(
                            &credential.access_token,
                            limit,
                            before_id,
                            status.as_deref(),
                            category.as_deref(),
                            source.as_deref(),
                        )
                        .await?
                }
                FeedbackCommand::Get { id } => {
                    server.get_feedback(&credential.access_token, id).await?
                }
                FeedbackCommand::Resolve { id, reopen } => {
                    server
                        .update_feedback(
                            &credential.access_token,
                            id,
                            if reopen { "open" } else { "resolved" },
                        )
                        .await?
                }
            };
            Ok(json!({"server":server.origin,"feedback":data}))
        }
        Command::Auth {
            command: AuthCommand::Logout,
        } => {
            let _guard = store.lock().await?;
            let Some(credential) = store.load()? else {
                return Ok(
                    json!({"logged_out":true,"server":server.origin,"local_removed":true,"remote_revoked":false,"already_logged_out":true}),
                );
            };
            let revoked = server.revoke(&credential.refresh_token).await.is_ok();
            if store.remove().is_err() {
                return Err(Error::new("logout_local_failed", "Could not remove local credentials. Check access to the selected credential store.")
                    .with_details(json!({"local_removed":false,"remote_revoked":revoked})));
            }
            if !revoked {
                return Err(Error::new("logout_incomplete", "Local credentials were removed, but remote revocation could not be confirmed. Revoke this connection on the website.")
                    .with_details(json!({"local_removed":true,"remote_revoked":false})));
            }
            Ok(
                json!({"logged_out":true,"server":server.origin,"local_removed":true,"remote_revoked":true,"already_logged_out":false}),
            )
        }
    }
}

async fn verified_identity(
    server: &Server,
    store: &Store,
    credential: &mut Credential,
    force: bool,
) -> Result<(UserInfo, bool)> {
    if credential.refresh_pending {
        return Err(login::refresh_uncertain());
    }
    let mut refreshed = false;
    if force || credential.expires_at <= now().saturating_add(defaults::REFRESH_EARLY_SECONDS) {
        login::refresh(server, store, credential).await?;
        refreshed = true;
    }
    let identity = match server.userinfo(&credential.access_token).await {
        Ok(identity) => identity,
        Err(error) if error.code == "not_authenticated" && !refreshed => {
            login::refresh(server, store, credential).await?;
            refreshed = true;
            server.userinfo(&credential.access_token).await?
        }
        Err(error) => return Err(error),
    };
    if identity.scope != credential.scope {
        return Err(Error::invalid_response());
    }
    if credential.account.as_ref() != Some(&account_subject(&identity)) {
        let _ = server.revoke(&credential.refresh_token).await;
        store.remove()?;
        return Err(Error::new(
            "account_mismatch",
            "The server returned a different account for the saved connection. Local credentials were removed. Run hfy auth login to authorize a new session.",
        ));
    }
    if credential.expires_at != identity.expires_at {
        credential.expires_at = identity.expires_at;
        store.save(credential)?;
    }
    Ok((identity, refreshed))
}

fn account_subject(identity: &UserInfo) -> AccountSubject {
    match &identity.user {
        Some(user) => AccountSubject::User(user.id.clone()),
        None => AccountSubject::Administrator,
    }
}

fn scope_covers(granted: &str, requested: &str) -> bool {
    requested
        .split_ascii_whitespace()
        .all(|scope| granted.split_ascii_whitespace().any(|value| value == scope))
}

fn signed_in(server: &Server, store: &Store, identity: UserInfo, reused: bool) -> Result<Value> {
    Ok(
        json!({"authenticated":true,"server":server.origin,"client_id":server.client_id,
        "identity":identity,"storage":store.info()?,"reused":reused}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuse_requires_the_saved_grant_to_cover_the_requested_scope() {
        assert!(scope_covers("identity mcp", "identity"));
        assert!(scope_covers("identity admin", "identity"));
        assert!(scope_covers("identity mcp", "identity mcp"));
        assert!(!scope_covers("identity", "identity mcp"));
        assert!(!scope_covers("identity mcp", "identity admin"));
    }

    #[test]
    fn global_options_work_after_subcommands_and_modes_conflict() {
        let cli = Cli::try_parse_from([
            "hfy",
            "auth",
            "login",
            "--device",
            "--json",
            "--server",
            "https://example.com",
        ])
        .unwrap();
        assert!(cli.json);
        assert_eq!(cli.server.as_deref(), Some("https://example.com"));
        assert!(Cli::try_parse_from(["hfy", "auth", "login", "--device", "--oauth"]).is_err());
        assert!(Cli::try_parse_from(["hfy", "auth", "login", "--timeout", "0"]).is_err());
        assert!(Cli::try_parse_from(["hfy", "whoami", "--scope", "identity mcp"]).is_err());
        assert!(Cli::try_parse_from(["hfy", "capabilities", "--scope", "mcp"]).is_ok());
    }
}
