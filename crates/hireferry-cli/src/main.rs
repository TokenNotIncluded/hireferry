use clap::{Parser, error::ErrorKind};
use hireferry_cli::{Cli, error::Error};
use serde_json::json;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().collect();
    let json_mode = arguments.iter().any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            if json_mode {
                println!("{}", json!({"help":error.to_string()}));
            } else {
                print!("{error}");
            }
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            output_error(
                Error::new(
                    "invalid_arguments",
                    "Invalid command arguments. Run hfy --help.",
                ),
                json_mode,
            );
            return ExitCode::from(2);
        }
    };
    let result = tokio::select! {
        result = hireferry_cli::run(cli) => result,
        _ = tokio::signal::ctrl_c() => Err(Error::new("cancelled", "Command cancelled.")),
    };
    match result {
        Ok(result) => {
            if json_mode {
                println!("{result}");
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&result).expect("JSON value is serializable")
                );
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            output_error(error, json_mode);
            ExitCode::FAILURE
        }
    }
}

fn output_error(error: Error, json_mode: bool) {
    if json_mode {
        println!("{}", json!({"error":error}));
    } else {
        eprintln!("Error [{}]: {}", error.code, error.message);
    }
}
