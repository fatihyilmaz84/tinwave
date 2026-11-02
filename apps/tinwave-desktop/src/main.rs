//! Tinwave desktop entry point: everything lives in the library; see `tinwave_desktop::app::wiring`.

use std::process::ExitCode;

fn main() -> ExitCode {
    tinwave_desktop::app::wiring::run()
}
