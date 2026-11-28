//! Semantic public web router, ingress rate limiter, body cap, cache headers, and
//! production observability exclusion (`EF-PUBLIC-0001` & `EF-PUBLIC-0002`).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::dei::{
    DeiSearchQuery, DeiStateShard, render_dei_finder_html, render_dei_state_html, sample_dei_shards,
};
use crate::demo::{DemoTurnOutcome, SyntheticDemoEngine};
use crate::html::{RobotsPolicy, escape_html, render_semantic_page};
use crate::legal::LegalArchiveRegistry;

/// Maximum request body accepted by standalone public site endpoints (`64 KiB`).
pub const PUBLIC_MAX_BODY_BYTES: usize = 65_536;

/// Deployment mode controlling whether staging-only observability routes exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentMode {
    Production,
    Staging,
}

/// Incoming HTTP-like request dispatched to [`PublicSiteRouter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRequest {
    pub method: String,
    pub path_and_query: String,
    pub client_ip: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl PublicRequest {
    #[must_use]
    pub fn get(path_and_query: impl Into<String>, client_ip: impl Into<String>) -> Self {
        Self {
            method: "GET".to_owned(),
            path_and_query: path_and_query.into(),
            client_ip: client_ip.into(),
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
    }

    #[must_use]
    pub fn post_json(
        path_and_query: impl Into<String>,
        client_ip: impl Into<String>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            method: "POST".to_owned(),
            path_and_query: path_and_query.into(),
            client_ip: client_ip.into(),
            headers: BTreeMap::new(),
            body,
        }
    }

    /// Attaches an HTTP request header (stored normalized to lowercase ASCII).
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .insert(name.into().to_ascii_lowercase(), value.into());
        self
    }

    /// Looks up a request header value case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// Structured HTTP response produced by [`PublicSiteRouter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

impl PublicResponse {
    fn html(status: u16, cache_control: &str, body: String) -> Self {
        let mut headers = BTreeMap::new();
        headers.insert("Cache-Control".to_owned(), cache_control.to_owned());
        if cache_control.contains("no-store") {
            headers.insert("CDN-Cache-Control".to_owned(), "no-store".to_owned());
        }
        headers.insert("X-Content-Type-Options".to_owned(), "nosniff".to_owned());
        Self {
            status,
            content_type: "text/html; charset=utf-8",
            headers,
            body,
        }
    }

    fn text(status: u16, content_type: &'static str, cache_control: &str, body: String) -> Self {
        let mut headers = BTreeMap::new();
        headers.insert("Cache-Control".to_owned(), cache_control.to_owned());
        if cache_control.contains("no-store") {
            headers.insert("CDN-Cache-Control".to_owned(), "no-store".to_owned());
        }
        headers.insert("X-Content-Type-Options".to_owned(), "nosniff".to_owned());
        Self {
            status,
            content_type,
            headers,
            body,
        }
    }
}

/// Per-IP fixed-window rate limiter and body-size guard for public ingress.
#[derive(Debug, Clone)]
pub struct PublicIngressGuard {
    max_requests_per_window: u32,
    window_ms: u64,
    max_body_bytes: usize,
    buckets: BTreeMap<String, (u64, u32)>,
}

impl PublicIngressGuard {
    #[must_use]
    pub fn new(max_requests_per_window: u32, window_ms: u64) -> Self {
        Self {
            max_requests_per_window,
            window_ms,
            max_body_bytes: PUBLIC_MAX_BODY_BYTES,
            buckets: BTreeMap::new(),
        }
    }

    /// Checks body limit (`<= 64 KiB`) and per-IP rate limit at `now_ms`.
    pub fn check(&mut self, client_ip: &str, body_len: usize, now_ms: u64) -> Result<(), u16> {
        if body_len > self.max_body_bytes {
            return Err(413);
        }
        let entry = self
            .buckets
            .entry(client_ip.to_owned())
            .or_insert((now_ms, 0));
        if now_ms.saturating_sub(entry.0) >= self.window_ms {
            *entry = (now_ms, 0);
        }
        if entry.1 >= self.max_requests_per_window {
            return Err(429);
        }
        entry.1 += 1;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct DemoTurnRequest {
    visitor_id: String,
    prompt: String,
}

#[derive(Debug, Serialize)]
struct HealthPayload {
    status: &'static str,
}

/// Zero-PHI atomic Prometheus counters/gauges for [`PublicSiteRouter`].
#[derive(Debug)]
struct PublicMetrics {
    requests_2xx: AtomicU64,
    requests_4xx: AtomicU64,
    requests_5xx: AtomicU64,
    #[cfg(not(target_arch = "wasm32"))]
    started_at: Instant,
    #[cfg(target_arch = "wasm32")]
    started_ms: AtomicU64,
}

impl Clone for PublicMetrics {
    fn clone(&self) -> Self {
        Self {
            requests_2xx: AtomicU64::new(self.requests_2xx.load(Ordering::Relaxed)),
            requests_4xx: AtomicU64::new(self.requests_4xx.load(Ordering::Relaxed)),
            requests_5xx: AtomicU64::new(self.requests_5xx.load(Ordering::Relaxed)),
            #[cfg(not(target_arch = "wasm32"))]
            started_at: self.started_at,
            #[cfg(target_arch = "wasm32")]
            started_ms: AtomicU64::new(self.started_ms.load(Ordering::Relaxed)),
        }
    }
}

impl PublicMetrics {
    fn new() -> Self {
        Self {
            requests_2xx: AtomicU64::new(0),
            requests_4xx: AtomicU64::new(0),
            requests_5xx: AtomicU64::new(0),
            #[cfg(not(target_arch = "wasm32"))]
            started_at: Instant::now(),
            #[cfg(target_arch = "wasm32")]
            started_ms: AtomicU64::new(0),
        }
    }

    fn record_status(&self, status: u16) {
        match status {
            200..=299 => {
                self.requests_2xx.fetch_add(1, Ordering::Relaxed);
            }
            400..=499 => {
                self.requests_4xx.fetch_add(1, Ordering::Relaxed);
            }
            500..=599 => {
                self.requests_5xx.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn uptime_seconds(&self, now_ms: u64) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = now_ms;
            self.started_at.elapsed().as_secs()
        }
        #[cfg(target_arch = "wasm32")]
        {
            let start = self.started_ms.load(Ordering::Relaxed);
            if start == 0 {
                let _ = self.started_ms.compare_exchange(
                    0,
                    now_ms,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
                0
            } else {
                now_ms.saturating_sub(start) / 1_000
            }
        }
    }

    fn render_prometheus(&self, now_ms: u64) -> String {
        let r2 = self.requests_2xx.load(Ordering::Relaxed);
        let r4 = self.requests_4xx.load(Ordering::Relaxed);
        let r5 = self.requests_5xx.load(Ordering::Relaxed);
        let uptime_secs = self.uptime_seconds(now_ms);
        let mut out = String::with_capacity(384);
        let _ = writeln!(
            out,
            "# HELP ef_public_http_requests_total Total HTTP requests handled by ef-public-site by status class."
        );
        let _ = writeln!(out, "# TYPE ef_public_http_requests_total counter");
        let _ = writeln!(
            out,
            "ef_public_http_requests_total{{status_class=\"2xx\"}} {r2}"
        );
        let _ = writeln!(
            out,
            "ef_public_http_requests_total{{status_class=\"4xx\"}} {r4}"
        );
        let _ = writeln!(
            out,
            "ef_public_http_requests_total{{status_class=\"5xx\"}} {r5}"
        );
        let _ = writeln!(
            out,
            "# HELP ef_public_uptime_seconds Seconds elapsed since ef-public-site started."
        );
        let _ = writeln!(out, "# TYPE ef_public_uptime_seconds gauge");
        let _ = writeln!(out, "ef_public_uptime_seconds {uptime_secs}");
        out
    }
}

/// Semantic HTML public site router with zero clinical DB credentials or secrets.
#[derive(Debug, Clone)]
pub struct PublicSiteRouter {
    mode: DeploymentMode,
    legal: LegalArchiveRegistry,
    demo: SyntheticDemoEngine,
    dei_shards: BTreeMap<String, DeiStateShard>,
    ingress: PublicIngressGuard,
    metrics: PublicMetrics,
}

impl PublicSiteRouter {
    /// Builds a canonical public site router in the requested deployment mode.
    #[must_use]
    pub fn new(mode: DeploymentMode) -> Self {
        let mut dei_shards = BTreeMap::new();
        for shard in sample_dei_shards() {
            dei_shards.insert(shard.state_code.to_ascii_lowercase(), shard);
        }
        Self {
            mode,
            legal: LegalArchiveRegistry::canonical(),
            demo: SyntheticDemoEngine::new(5),
            dei_shards,
            ingress: PublicIngressGuard::new(60, 60_000),
            metrics: PublicMetrics::new(),
        }
    }

    /// Overrides the per-IP ingress rate limiter (useful for deterministic tests).
    pub fn set_ingress_guard(&mut self, guard: PublicIngressGuard) {
        self.ingress = guard;
    }

    /// Overrides the synthetic demo turn budget.
    pub fn set_demo_engine(&mut self, demo: SyntheticDemoEngine) {
        self.demo = demo;
    }

    /// Replaces or inserts a Dei state shard (for testing shard version upgrades).
    pub fn upsert_dei_shard(&mut self, shard: DeiStateShard) {
        self.dei_shards
            .insert(shard.state_code.to_ascii_lowercase(), shard);
    }

    /// Read-only access to the legal archive registry.
    #[must_use]
    pub const fn legal(&self) -> &LegalArchiveRegistry {
        &self.legal
    }

    /// Read-only access to the isolated synthetic demo engine.
    #[must_use]
    pub const fn demo(&self) -> &SyntheticDemoEngine {
        &self.demo
    }

    /// Dispatches a public HTTP request with ingress checks, cache headers, `X-Request-Id`
    /// propagation, and semantic HTML rendering.
    pub fn handle(&mut self, req: &PublicRequest, now_ms: u64) -> PublicResponse {
        let request_id = ef_core::HttpRequestId::parse_or_generate(
            req.header(ef_core::REQUEST_ID_HEADER),
            "public",
        );
        let mut resp = self.dispatch_handle(req, now_ms);
        resp.headers.insert(
            ef_core::REQUEST_ID_HEADER.to_owned(),
            request_id.as_str().to_owned(),
        );
        self.metrics.record_status(resp.status);
        resp
    }

    /// Alias for [`Self::handle`] matching the server request-handler naming convention.
    pub fn handle_request(&mut self, req: &PublicRequest, now_ms: u64) -> PublicResponse {
        self.handle(req, now_ms)
    }

    fn dispatch_handle(&mut self, req: &PublicRequest, now_ms: u64) -> PublicResponse {
        if let Err(code) = self.ingress.check(&req.client_ip, req.body.len(), now_ms) {
            let msg = if code == 413 {
                "Payload Too Large (64 KiB limit exceeded)"
            } else {
                "Too Many Requests"
            };
            let mut resp = PublicResponse::text(
                code,
                "text/plain; charset=utf-8",
                "no-store",
                msg.to_owned(),
            );
            if code == 429 {
                resp.headers
                    .insert("Retry-After".to_owned(), "60".to_owned());
            }
            return resp;
        }

        let (path, raw_query) = match req.path_and_query.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (req.path_and_query.as_str(), None),
        };

        // Staging-only observability routes must NEVER be exposed in production.
        if path == "/grafana" || path.starts_with("/grafana/") {
            if self.mode == DeploymentMode::Production {
                return self.render_not_found(path);
            }
            return PublicResponse::text(
                200,
                "text/plain; charset=utf-8",
                "no-store",
                "staging-observability-route".to_owned(),
            );
        }

        match (req.method.as_str(), path) {
            ("GET", "/") => PublicResponse::html(
                200,
                "public, max-age=300",
                render_semantic_page(
                    "Elysian Fields — Clinical Workspace & Patient Health",
                    "Curie Clinic for physicians, Curie Mobile for patients, and Dei recovery finder.",
                    "/",
                    RobotsPolicy::IndexFollow,
                    "    <h1>Elysian Fields</h1>\n\
    <section aria-label=\"Products\">\n\
      <h2>Curie Clinic</h2>\n\
      <p>Calm clinical assistance with verifiable chart citations and clinician-owned workflows.</p>\n\
      <h2>Curie Mobile</h2>\n\
      <p>Standalone patient health records with curated, consent-first sharing.</p>\n\
      <h2>Dei Recovery Finder</h2>\n\
      <p><a href=\"/recovery\">Browse SAMHSA recovery facilities</a> with 988 and 911 crisis access first.</p>\n\
    </section>",
                ),
            ),
            ("GET", "/mobile") => PublicResponse::html(
                200,
                "public, max-age=300",
                render_semantic_page(
                    "Curie Mobile — Patient-Owned Health Records",
                    "Keep your records in a standalone account and choose what to share with your doctor.",
                    "/mobile",
                    RobotsPolicy::IndexFollow,
                    "    <h1>Curie Mobile</h1>\n\
    <p>Import records from portals, track on-device health trends, and share curated updates only when you confirm.</p>\n\
    <p><a href=\"/mobile/privacy\">Mobile Privacy Policy</a> | <a href=\"/mobile/consumer-health-data\">Consumer Health Data Policy</a></p>",
                ),
            ),
            ("GET", "/invite") => {
                let mut resp = PublicResponse::html(
                    200,
                    "no-store",
                    render_semantic_page(
                        "Practice Invitation — Curie Mobile",
                        "Open your practice invitation in Curie Mobile.",
                        "/invite",
                        RobotsPolicy::NoIndexNoFollow,
                        "    <h1>Open Your Practice Invitation</h1>\n\
    <p>Install Curie Mobile to verify your date of birth and decide whether to share with your care team.</p>\n\
    <ul>\n\
      <li><a href=\"https://apps.apple.com/\">Download on the App Store</a></li>\n\
      <li><a href=\"https://play.google.com/\">Get it on Google Play</a></li>\n\
    </ul>",
                    ),
                );
                resp.headers
                    .insert("X-Robots-Tag".to_owned(), "noindex, nofollow".to_owned());
                resp
            }
            ("GET", "/demo") => PublicResponse::html(
                200,
                "no-store",
                render_semantic_page(
                    "Interactive Synthetic Physician Demo",
                    "Explore Curie Clinic on an isolated synthetic patient chart with verbatim citations.",
                    "/demo",
                    RobotsPolicy::IndexFollow,
                    "    <h1>Interactive Synthetic Physician Demo</h1>\n\
    <p>This demo uses 100% synthetic fixture data isolated per visitor with zero access to clinical databases.</p>",
                ),
            ),
            ("POST", "/demo/turn") => self.handle_demo_turn(&req.body),
            ("GET", "/recovery") | ("GET", "/recovery/") | ("GET", "/dei") | ("GET", "/dei/") => {
                let shards: Vec<DeiStateShard> = self.dei_shards.values().cloned().collect();
                let query = parse_dei_query(raw_query);
                let cache_control = if raw_query.is_some_and(|q| !q.is_empty()) {
                    "public, max-age=120"
                } else {
                    "public, max-age=300"
                };
                PublicResponse::html(200, cache_control, render_dei_finder_html(&shards, &query))
            }
            ("GET", "/dei/search") | ("GET", "/recovery/search") => {
                self.handle_dei_search(raw_query)
            }
            ("GET", "/robots.txt") => PublicResponse::text(
                200,
                "text/plain; charset=utf-8",
                "public, max-age=3600",
                "User-agent: *\nAllow: /\nDisallow: /invite\nSitemap: /sitemap.xml\n".to_owned(),
            ),
            ("GET", "/sitemap.xml") => PublicResponse::text(
                200,
                "application/xml; charset=utf-8",
                "public, max-age=3600",
                self.render_sitemap_xml(),
            ),
            ("GET", "/health") | ("GET", "/healthz") | ("GET", "/readyz") => {
                let body = serde_json::to_string(&HealthPayload { status: "ok" })
                    .unwrap_or_else(|_| "{\"status\":\"ok\"}".to_owned());
                PublicResponse::text(200, "application/json", "no-store", body)
            }
            ("GET", "/metrics") => PublicResponse::text(
                200,
                "text/plain; version=0.0.4; charset=utf-8",
                "no-store",
                self.metrics.render_prometheus(now_ms),
            ),
            ("GET", p) if self.legal.get(p).is_some() => match self.legal.render_html(p) {
                Ok(html) => PublicResponse::html(200, "public, max-age=300", html),
                Err(_) => self.render_not_found(p),
            },
            ("GET", p) if p.starts_with("/recovery/data/") => self.handle_dei_shard_json(p),
            ("GET", p) if p.starts_with("/dei/data/") => {
                self.handle_dei_shard_json(&p.replacen("/dei/data/", "/recovery/data/", 1))
            }
            ("GET", p) if p.starts_with("/recovery/") => self.handle_dei_state(p, raw_query),
            ("GET", p) if p.starts_with("/dei/") => {
                self.handle_dei_state(&p.replacen("/dei/", "/recovery/", 1), raw_query)
            }
            _ => self.render_not_found(path),
        }
    }

    fn handle_demo_turn(&mut self, body: &[u8]) -> PublicResponse {
        let Ok(parsed) = serde_json::from_slice::<DemoTurnRequest>(body) else {
            return PublicResponse::text(
                400,
                "application/json",
                "no-store",
                "{\"error\":\"invalid JSON request\"}".to_owned(),
            );
        };

        match self.demo.submit_turn(&parsed.visitor_id, &parsed.prompt) {
            Ok(outcome) => {
                let status = match outcome {
                    DemoTurnOutcome::Answered { .. } => 200,
                    DemoTurnOutcome::BudgetExhausted { .. } => 429,
                };
                let json = serde_json::to_string(&outcome)
                    .unwrap_or_else(|_| "{\"error\":\"serialization\"}".to_owned());
                PublicResponse::text(status, "application/json", "no-store", json)
            }
            Err(err) => PublicResponse::text(
                400,
                "application/json",
                "no-store",
                format!("{{\"error\":\"{}\"}}", escape_html(&err.to_string())),
            ),
        }
    }

    fn handle_dei_search(&self, raw_query: Option<&str>) -> PublicResponse {
        let query = parse_dei_query(raw_query);
        let state_filter = query
            .state
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let mut all_facilities = Vec::new();
        for shard in self.dei_shards.values() {
            if let Some(ref st) = state_filter
                && shard.state_code.to_ascii_lowercase() != *st
                && shard.state_name.to_ascii_lowercase() != *st
            {
                continue;
            }
            for fac in shard.filter(&query) {
                all_facilities.push(fac.clone());
            }
        }
        let aggregate = DeiStateShard {
            state_code: "ALL".to_owned(),
            state_name: "All States — Search Results".to_owned(),
            facilities: all_facilities,
        };
        PublicResponse::html(
            200,
            "public, max-age=120",
            render_dei_state_html(&aggregate, &DeiSearchQuery::default()),
        )
    }

    fn handle_dei_state(&self, path: &str, raw_query: Option<&str>) -> PublicResponse {
        let state_slug = path
            .trim_start_matches("/recovery/")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        let Some(shard) = self.dei_shards.get(&state_slug) else {
            return self.render_not_found(path);
        };
        let query = parse_dei_query(raw_query);
        PublicResponse::html(
            200,
            "public, max-age=300",
            render_dei_state_html(shard, &query),
        )
    }

    fn handle_dei_shard_json(&self, path: &str) -> PublicResponse {
        let filename = path.trim_start_matches("/recovery/data/");
        let Some(stem) = filename.strip_suffix(".json") else {
            return self.render_not_found(path);
        };
        let Some(shard) = self.dei_shards.get(&stem.to_ascii_lowercase()) else {
            return self.render_not_found(path);
        };
        let versioned = shard.serialize_with_version();
        let mut resp = PublicResponse::text(
            200,
            "application/json",
            "public, max-age=31536000, immutable",
            versioned.json,
        );
        resp.headers.insert(
            "ETag".to_owned(),
            format!("\"{}\"", versioned.version_sha256),
        );
        resp
    }

    fn render_not_found(&self, path: &str) -> PublicResponse {
        PublicResponse::html(
            404,
            "no-store",
            render_semantic_page(
                "Page Not Found — Elysian Fields",
                "The requested page could not be found.",
                "/404",
                RobotsPolicy::NoIndexNoFollow,
                &format!(
                    "    <h1>404 — Page Not Found</h1>\n    <p>No page exists at <code>{}</code>.</p>",
                    escape_html(path)
                ),
            ),
        )
    }

    fn render_sitemap_xml(&self) -> String {
        let mut xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
        );
        for static_route in ["/", "/mobile", "/recovery", "/demo"] {
            xml.push_str(&format!("  <url><loc>{static_route}</loc></url>\n"));
        }
        for legal_route in self.legal.routes() {
            xml.push_str(&format!("  <url><loc>{legal_route}</loc></url>\n"));
        }
        for state_slug in self.dei_shards.keys() {
            xml.push_str(&format!("  <url><loc>/recovery/{state_slug}</loc></url>\n"));
        }
        xml.push_str("</urlset>\n");
        xml
    }
}

fn parse_dei_query(raw_query: Option<&str>) -> DeiSearchQuery {
    let mut query = DeiSearchQuery::default();
    let Some(qs) = raw_query else {
        return query;
    };
    for pair in qs.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let decoded = decode_query_value(v);
        match k {
            "state" => query.state = Some(decoded),
            "city" => query.city = Some(decoded),
            "service" => query.service = Some(decoded),
            "q" => query.text = Some(decoded),
            _ => {}
        }
    }
    query
}

fn decode_query_value(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        match bytes[idx] {
            b'+' => {
                out.push(b' ');
                idx += 1;
            }
            b'%' if idx + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) =
                    (hex_nibble(bytes[idx + 1]), hex_nibble(bytes[idx + 2]))
                {
                    out.push((hi << 4) | lo);
                    idx += 3;
                } else {
                    out.push(b'%');
                    idx += 1;
                }
            }
            other => {
                out.push(other);
                idx += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
