mod application;
mod cli;
mod dependency_sync;
mod domain;
mod fs_support;
mod platform;
mod runtime_executor;
mod safety_guards;
mod source_integration;
mod state_ledger;
mod toml_support;

use clap::{Parser, error::ErrorKind};

use crate::cli::Cli;

fn main() {
    std::process::exit(run(std::env::args_os()));
}

fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(cli) => match application::dispatch(cli) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("error: {error}");
                error.exit_code()
            }
        },
        Err(error) => match error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                print!("{error}");
                0
            }
            _ => {
                eprint!("{error}");
                2
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::run;

    #[test]
    fn help_command_returns_zero() {
        assert_eq!(run(["gov", "help"]), 0);
    }
}
