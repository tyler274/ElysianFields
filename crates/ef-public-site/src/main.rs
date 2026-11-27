//! Standalone HTTP/1.1 binary entry point for the Elysian Fields public site and Dei recovery finder.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use ef_public_site::{DeploymentMode, PublicSiteRouter, TcpPublicSiteServer};

    let bind_addr = std::env::var("EF_PUBLIC_BIND").unwrap_or_else(|_| "127.0.0.1:8082".to_owned());
    let mode = match std::env::var("EF_PUBLIC_MODE").as_deref() {
        Ok("staging") => DeploymentMode::Staging,
        _ => DeploymentMode::Production,
    };

    let router = PublicSiteRouter::new(mode);
    let server = TcpPublicSiteServer::bind(&bind_addr, router)?;
    eprintln!(
        "ef-public-site listening on http://{}/ (mode: {mode:?})",
        server.local_addr()
    );
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
