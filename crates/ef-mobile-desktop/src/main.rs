//! Native desktop binary entry point for Curie Mobile (`ef-mobile-desktop`).

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use std::process::ExitCode;

    use ef_mobile_desktop::{MobileDesktopConfig, RunTarget, run_headless_smoke, run_windowed_app};

    let config = match MobileDesktopConfig::parse_args(std::env::args().skip(1)) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!(
                "ef-mobile-desktop error: {err}\n\n{}",
                MobileDesktopConfig::usage()
            );
            return ExitCode::from(2);
        }
    };

    match config.target.clone() {
        RunTarget::Help => {
            println!("{}", MobileDesktopConfig::usage());
            ExitCode::SUCCESS
        }
        RunTarget::HeadlessSmoke => match run_headless_smoke(&config) {
            Ok(report) => match serde_json::to_string_pretty(&report) {
                Ok(json) => {
                    println!("{json}");
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    eprintln!("ef-mobile-desktop failed to serialize smoke report: {err}");
                    ExitCode::FAILURE
                }
            },
            Err(err) => {
                eprintln!("ef-mobile-desktop headless smoke failed: {err}");
                ExitCode::FAILURE
            }
        },
        RunTarget::Windowed { screenshot_path } => {
            run_windowed_app(&config, screenshot_path);
            ExitCode::SUCCESS
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
