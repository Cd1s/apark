mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    // Any argument means CLI mode: the desktop binary accepts every `apark` command.
    if args.len() > 1 {
        return apark_cli::run(args);
    }
    match ui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Apark: {e:#}");
            ExitCode::FAILURE
        }
    }
}
