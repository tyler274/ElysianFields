//! Binary entry point for the headless Curie Mobile HTTP/1.1 service.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ef_mobile_domain::FileReceiptJournal;
    use ef_mobile_server::{MobileService, TcpMobileServer};

    let bind_addr = std::env::var("EF_MOBILE_BIND").unwrap_or_else(|_| "127.0.0.1:8089".to_owned());
    let data_dir = std::env::var("EF_MOBILE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./target/ef-mobile-data"));
    let journal_dir = std::env::var("EF_MOBILE_JOURNAL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./target/ef-mobile-privacy-journal"));

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1_760_000_000, |d| d.as_secs());
    let journal = FileReceiptJournal::open(&journal_dir)?;
    let pg_url = std::env::var("EF_MOBILE_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .filter(|s| !s.trim().is_empty());
    let mut service = if let Some(ref url) = pg_url {
        let pg_cfg = ef_pgwire::PgConnectionConfig::parse_url(url)?;
        MobileService::open_postgres(&pg_cfg, Box::new(journal), now_secs, true)?
    } else {
        MobileService::open_durable(&data_dir, Box::new(journal), now_secs, true)?
    };
    let report = service.execute_restore_replay_gate(10_000)?;
    eprintln!(
        "ef-mobile-server restore replay verified: applied={}",
        report.applied
    );

    let server = TcpMobileServer::bind(&bind_addr, service)?;
    eprintln!(
        "ef-mobile-server listening on http://{}/elysianfields/mobile-api/readyz",
        server.local_addr()
    );
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
