//! Native Curie Clinic desktop binary entry point.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use ef_clinic_desktop::{DesktopConfigError, DesktopLaunchConfig, run_desktop_app};

    let config = match DesktopLaunchConfig::parse_args(std::env::args().skip(1)) {
        Ok(cfg) => cfg,
        Err(DesktopConfigError::HelpRequested(text)) => {
            println!("{text}");
            return std::process::ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("ef-clinic-desktop: {err}");
            return std::process::ExitCode::from(2);
        }
    };

    if let Err(err) = run_desktop_app(config) {
        eprintln!("ef-clinic-desktop failed: {err}");
        return std::process::ExitCode::FAILURE;
    }

    std::process::ExitCode::SUCCESS
}

#[cfg(target_arch = "wasm32")]
fn main() {}
