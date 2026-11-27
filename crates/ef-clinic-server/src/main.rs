//! Binary entry point for the headless Curie Clinic M2 vertical-slice server.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ef_clinic_domain::auth::SignInMode;
    use ef_clinic_domain::chat::RetryBackoffMode;
    use ef_clinic_server::{ClinicService, TcpClinicServer};
    use ef_core::SourceKey;

    let bind_addr = std::env::var("EF_CLINIC_BIND").unwrap_or_else(|_| "127.0.0.1:8088".to_owned());
    let data_dir = std::env::var("EF_CLINIC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./target/ef-clinic-data"));

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1_760_000_000, |d| d.as_secs());
    let default_source = SourceKey::new("src-note-1")?;
    let service = ClinicService::open_durable(
        &data_dir,
        SignInMode::Hosted,
        RetryBackoffMode::Hosted,
        default_source,
        now_secs,
    )?;

    let server = TcpClinicServer::bind(&bind_addr, service)?;
    eprintln!(
        "ef-clinic-server listening on http://{}/elysianfields/ (wal: {})",
        server.local_addr(),
        data_dir.display()
    );
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
