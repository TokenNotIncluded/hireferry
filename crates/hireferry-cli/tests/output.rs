use serde_json::Value;
use std::process::{Command, Output};

fn hfy(arguments: &[&str]) -> Output {
    let temp = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_hfy"))
        .args(arguments)
        .args(["--storage", "file"])
        .arg("--config-dir")
        .arg(temp.path().join("private"))
        .env_remove("HFY_SERVER")
        .output()
        .unwrap()
}

#[test]
fn json_status_and_logout_work_without_a_saved_credential() {
    let status = hfy(&["auth", "status", "--json"]);
    assert!(status.status.success());
    let value: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(value["authenticated"], false);
    assert!(status.stderr.is_empty());
    let logout = hfy(&["--json", "auth", "logout"]);
    assert!(logout.status.success());
    let value: Value = serde_json::from_slice(&logout.stdout).unwrap();
    assert_eq!(value["logged_out"], true);
    assert_eq!(value["already_logged_out"], true);
}

#[test]
fn json_failure_is_single_safe_document_with_nonzero_exit_status() {
    let output = hfy(&["whoami", "--json"]);
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "not_authenticated");
    assert!(output.stderr.is_empty());

    let output = hfy(&[
        "auth",
        "status",
        "--json",
        "--server",
        "https://secret-user:secret-password@example.com",
    ]);
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "invalid_server");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-password"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-password"));

    let output = hfy(&["auth", "login", "--json", "--timeout", "secret-token"]);
    assert_eq!(output.status.code(), Some(2));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "invalid_arguments");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-token"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-token"));
}

#[test]
fn an_explicit_config_directory_does_not_require_home() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hfy"))
        .args(["auth", "status", "--json", "--config-dir"])
        .arg(temp.path().join("private"))
        .args(["--storage", "file"])
        .env_remove("HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("LOCALAPPDATA")
        .env_remove("HFY_SERVER")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["authenticated"], false);
}
