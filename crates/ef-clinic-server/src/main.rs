//! Binary entry point for the headless Curie Clinic HTTP/1.1 + SSE server.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ef_clinic_domain::auth::SignInMode;
    use ef_clinic_domain::chat::RetryBackoffMode;
    use ef_clinic_server::{ClinicService, RateLimitConfig, TcpClinicServer};
    use ef_core::SourceKey;

    let mut bind_addr =
        std::env::var("EF_CLINIC_BIND").unwrap_or_else(|_| "127.0.0.1:8088".to_owned());
    let mut data_dir = std::env::var("EF_CLINIC_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./target/ef-clinic-data"));
    let mut web_dir = std::env::var("EF_CLINIC_WEB_DIR").ok().map(PathBuf::from);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if let Some(val) = arg.strip_prefix("--web-dir=") {
            web_dir = Some(PathBuf::from(val));
        } else if arg == "--web-dir" {
            if let Some(val) = args.next() {
                web_dir = Some(PathBuf::from(val));
            }
        } else if let Some(val) = arg.strip_prefix("--bind=") {
            bind_addr = val.to_owned();
        } else if arg == "--bind" {
            if let Some(val) = args.next() {
                bind_addr = val;
            }
        } else if let Some(val) = arg.strip_prefix("--data-dir=") {
            data_dir = PathBuf::from(val);
        } else if arg == "--data-dir"
            && let Some(val) = args.next()
        {
            data_dir = PathBuf::from(val);
        }
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1_760_000_000, |d| d.as_secs());
    let default_source = SourceKey::new("src-note-1")?;
    let pg_url = std::env::var("EF_CLINIC_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .filter(|s| !s.trim().is_empty());
    let mut service = if let Some(ref url) = pg_url {
        let pg_cfg = ef_pgwire::PgConnectionConfig::parse_url(url)?;
        ClinicService::open_postgres(
            &pg_cfg,
            SignInMode::Hosted,
            RetryBackoffMode::Hosted,
            default_source,
            now_secs,
        )?
    } else {
        ClinicService::open_durable(
            &data_dir,
            SignInMode::Hosted,
            RetryBackoffMode::Hosted,
            default_source,
            now_secs,
        )?
    };

    if let Some(dir) = web_dir.clone() {
        service.set_web_dir(dir);
    }

    if let (Ok(max_raw), Ok(win_raw)) = (
        std::env::var("EF_CLINIC_RATE_LIMIT_MAX"),
        std::env::var("EF_CLINIC_RATE_LIMIT_WINDOW_SECS"),
    ) && let (Ok(max_requests), Ok(window_secs)) =
        (max_raw.parse::<usize>(), win_raw.parse::<u64>())
    {
        service.set_rate_limit(RateLimitConfig {
            max_requests,
            window_secs,
        });
    }

    let server = TcpClinicServer::bind(&bind_addr, service)?;
    eprintln!(
        "ef-clinic-server listening on http://{}/elysianfields/ (wal: {}, web_dir: {})",
        server.local_addr(),
        data_dir.display(),
        web_dir
            .as_ref()
            .map_or_else(|| "embedded".to_owned(), |d| d.display().to_string()),
    );
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
