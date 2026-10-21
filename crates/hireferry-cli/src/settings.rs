use crate::{
    Cli, defaults,
    error::{Error, Result},
    storage,
};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::PathBuf};

#[derive(Clone, Copy, Default, Deserialize, Serialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageChoice {
    #[default]
    Auto,
    Keyring,
    File,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub server: String,
    pub client_id: String,
    pub scope: String,
    pub auth_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
    pub poll_interval_seconds: u64,
    pub storage: StorageChoice,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: defaults::SERVER.to_owned(),
            client_id: defaults::CLIENT_ID.to_owned(),
            scope: defaults::SCOPE.to_owned(),
            auth_timeout_seconds: defaults::AUTH_TIMEOUT_SECONDS,
            request_timeout_seconds: defaults::REQUEST_TIMEOUT_SECONDS,
            poll_interval_seconds: defaults::POLL_INTERVAL_SECONDS,
            storage: StorageChoice::Auto,
        }
    }
}

impl Settings {
    pub fn load(cli: &Cli) -> Result<(Self, PathBuf)> {
        let directory = match cli
            .config_dir
            .clone()
            .or_else(|| std::env::var_os("HFY_CONFIG_DIR").map(PathBuf::from))
        {
            Some(path) => path,
            None => storage::default_directory()?,
        };
        let path = directory.join("config.toml");
        let read_config = || -> std::io::Result<String> {
            let file = fs::File::open(path)?;
            let mut text = String::new();
            file.take(defaults::MAX_RESPONSE_BYTES as u64 + 1)
                .read_to_string(&mut text)?;
            Ok(text)
        };
        let mut settings = match read_config() {
            Ok(text) if text.len() <= defaults::MAX_RESPONSE_BYTES => {
                toml::from_str(&text).map_err(|_| invalid_settings())?
            }
            Ok(_) => return Err(invalid_settings()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(_) => {
                return Err(Error::new(
                    "config_read",
                    "Could not read the CLI config.toml file.",
                ));
            }
        };
        settings.apply(cli, |name| std::env::var(name).ok())?;
        settings.validate()?;
        Ok((settings, directory))
    }

    fn apply(&mut self, cli: &Cli, environment: impl Fn(&str) -> Option<String>) -> Result<()> {
        self.server = cli
            .server
            .clone()
            .or_else(|| environment("HFY_SERVER"))
            .unwrap_or_else(|| self.server.clone());
        self.client_id = cli
            .client_id
            .clone()
            .or_else(|| environment("HFY_CLIENT_ID"))
            .unwrap_or_else(|| self.client_id.clone());
        self.scope = cli
            .scope
            .clone()
            .or_else(|| environment("HFY_SCOPE"))
            .unwrap_or_else(|| self.scope.clone());
        let auth_timeout = match &cli.command {
            crate::Command::Auth {
                command: crate::AuthCommand::Login(args),
            } => args.timeout,
            _ => None,
        };
        self.auth_timeout_seconds = number(
            auth_timeout,
            environment("HFY_AUTH_TIMEOUT_SECONDS"),
            self.auth_timeout_seconds,
        )?;
        self.request_timeout_seconds = number(
            cli.request_timeout_seconds,
            environment("HFY_REQUEST_TIMEOUT_SECONDS"),
            self.request_timeout_seconds,
        )?;
        self.poll_interval_seconds = number(
            cli.poll_interval_seconds,
            environment("HFY_POLL_INTERVAL_SECONDS"),
            self.poll_interval_seconds,
        )?;
        self.storage = match cli.storage {
            Some(storage) => storage,
            None => match environment("HFY_STORAGE").as_deref() {
                Some("auto") => StorageChoice::Auto,
                Some("keyring") => StorageChoice::Keyring,
                Some("file") => StorageChoice::File,
                None => self.storage,
                _ => return Err(invalid_settings()),
            },
        };
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.client_id.is_empty()
            || self.client_id.len() > 80
            || !self
                .client_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || !matches!(self.scope.as_str(), "identity" | "mcp" | "admin")
            || !(10..=1800).contains(&self.auth_timeout_seconds)
            || !(1..=120).contains(&self.request_timeout_seconds)
            || !(1..=300).contains(&self.poll_interval_seconds)
        {
            return Err(invalid_settings());
        }
        Ok(())
    }
}

fn number(cli: Option<u64>, environment: Option<String>, fallback: u64) -> Result<u64> {
    if let Some(value) = cli {
        return Ok(value);
    }
    environment
        .map(|value| value.parse().map_err(|_| invalid_settings()))
        .unwrap_or(Ok(fallback))
}

fn invalid_settings() -> Error {
    Error::new(
        "invalid_config",
        "Invalid CLI settings. Check config.toml, HFY_* variables and command options.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_environment_and_file_settings_have_explicit_priority() {
        let cli = Cli::try_parse_from([
            "hfy",
            "whoami",
            "--scope",
            "admin",
            "--request-timeout-seconds",
            "40",
        ])
        .unwrap();
        let mut settings: Settings =
            toml::from_str("scope='mcp'\nrequest_timeout_seconds=10\npoll_interval_seconds=8")
                .unwrap();
        settings
            .apply(&cli, |name| match name {
                "HFY_SCOPE" => Some("identity".into()),
                "HFY_CLIENT_ID" => Some("test-cli".into()),
                "HFY_REQUEST_TIMEOUT_SECONDS" => Some("30".into()),
                _ => None,
            })
            .unwrap();
        assert_eq!(settings.scope, "admin");
        assert_eq!(settings.client_id, "test-cli");
        assert_eq!(settings.request_timeout_seconds, 40);
        assert_eq!(settings.poll_interval_seconds, 8);
        assert_eq!(
            settings.auth_timeout_seconds,
            defaults::AUTH_TIMEOUT_SECONDS
        );
    }
}
