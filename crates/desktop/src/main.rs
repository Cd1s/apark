// No console window for the GUI on Windows; CLI mode re-attaches to the parent console.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod fonts;
mod widgets;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    // Any argument (except macOS' legacy -psn_ launch flag) means CLI mode.
    let cli = args.get(1).is_some_and(|a| !a.to_string_lossy().starts_with("-psn_"));
    if cli {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
        return apark_cli::run(args);
    }
    match app::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Apark: {e:#}");
            ExitCode::FAILURE
        }
    }
}
