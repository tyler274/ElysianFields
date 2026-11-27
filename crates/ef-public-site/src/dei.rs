//! Dei SAMHSA Recovery Facility Finder (`EF-PUBLIC-0002` / `SCN-M3-0019`).
//!
//! Enforces:
//! - Crisis resources (`988`, `911`, `SAMHSA National Helpline`) rendered strictly BEFORE any facility listing.
//! - Zero-JS semantic baseline: prerendered static state listing, complete city directory, and HTML form filter.
//! - Versioned Cache Storage shard state machine (`Loading`, `Fresh`, `Stale`, `Failed`) matching
//!   `specs/packages/m3-suite/contracts/m3-boundaries-and-states.schema.json`.
//! - Offline reload resilience with service-worker JS+Wasm asset pair retention and honest failure fallback
//!   (fetch failure NEVER reads as zero results).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::html::{RobotsPolicy, escape_html, render_semantic_page};
use crate::sha256::sha256_hex;

/// One SAMHSA recovery facility entry inside a state shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryFacility {
    pub id: String,
    pub name: String,
    pub city: String,
    pub state_code: String,
    pub phone: String,
    pub services: Vec<String>,
}

/// Complete state facility shard serialized as JSON and addressed by content SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeiStateShard {
    pub state_code: String,
    pub state_name: String,
    pub facilities: Vec<RecoveryFacility>,
}

impl DeiStateShard {
    /// Serializes this shard deterministically and computes its content SHA-256 version digest.
    #[must_use]
    pub fn serialize_with_version(&self) -> VersionedShardPayload {
        let json = serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned());
        let version_sha256 = sha256_hex(json.as_bytes());
        let cache_key = format!("data/{}.json?v={version_sha256}", self.state_code);
        VersionedShardPayload {
            state_code: self.state_code.clone(),
            version_sha256,
            cache_key,
            json,
        }
    }

    /// Sorted, deduplicated list of every city represented in this state shard.
    #[must_use]
    pub fn cities(&self) -> Vec<String> {
        let set: BTreeSet<String> = self.facilities.iter().map(|f| f.city.clone()).collect();
        set.into_iter().collect()
    }

    /// Filters facilities by optional city, service keyword, and free-text query.
    #[must_use]
    pub fn filter(&self, query: &DeiSearchQuery) -> Vec<&RecoveryFacility> {
        let city_filter = query
            .city
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let service_filter = query
            .service
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let text_filter = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);

        self.facilities
            .iter()
            .filter(|facility| {
                if let Some(ref city) = city_filter
                    && facility.city.to_ascii_lowercase() != *city
                {
                    return false;
                }
                if let Some(ref service) = service_filter
                    && !facility
                        .services
                        .iter()
                        .any(|s| s.to_ascii_lowercase().contains(service))
                {
                    return false;
                }
                if let Some(ref text) = text_filter {
                    let matches_name = facility.name.to_ascii_lowercase().contains(text);
                    let matches_city = facility.city.to_ascii_lowercase().contains(text);
                    let matches_svc = facility
                        .services
                        .iter()
                        .any(|s| s.to_ascii_lowercase().contains(text));
                    if !(matches_name || matches_city || matches_svc) {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

/// Content-addressed JSON payload for one state shard (`data/<ST>.json?v=<sha256>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedShardPayload {
    pub state_code: String,
    pub version_sha256: String,
    pub cache_key: String,
    pub json: String,
}

/// Filter query shared by the zero-JS HTML `<form>` fallback and progressive interactive finder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeiSearchQuery {
    pub city: Option<String>,
    pub service: Option<String>,
    pub text: Option<String>,
}

/// Explicit Dei Cache Storage shard load state matching `m3-boundaries-and-states.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum DeiShardLoadState {
    Loading {
        received: u64,
        total: Option<u64>,
    },
    Fresh {
        version_sha256: String,
    },
    Stale {
        version_sha256: String,
    },
    Failed {
        retryable: bool,
        stale_version_sha256: Option<String>,
    },
}

/// UI presentation contract derived from [`DeiShardLoadState`].
///
/// Guarantees that a fetch failure NEVER renders as "0 results": it either retains
/// the stale cached shard with a warning or displays an explicit error alert + **Try again**
/// over the prerendered static listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeiDisplayMode {
    PrerenderedWithProgress {
        received: u64,
        total: Option<u64>,
    },
    InteractiveFresh {
        version_sha256: String,
        facility_count: usize,
    },
    InteractiveStaleWithWarning {
        version_sha256: String,
        warning: &'static str,
        facility_count: usize,
    },
    ErrorOverPrerenderedListing {
        alert: &'static str,
        retry_label: &'static str,
        show_zero_results: bool,
    },
}

/// Errors produced by Dei shard cache operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeiCacheError {
    #[error("shard JSON digest mismatch: expected '{expected}', got '{actual}'")]
    DigestMismatch { expected: String, actual: String },
    #[error("invalid shard JSON: {0}")]
    InvalidShardJson(String),
}

/// Models browser Cache Storage for versioned Dei shards (`data/<ST>.json?v=<sha256>`).
#[derive(Debug, Default)]
pub struct DeiShardCache {
    /// Cached shards keyed by exact Cache Storage URL (`data/<ST>.json?v=<sha256>`).
    entries: BTreeMap<String, DeiStateShard>,
    /// Current load state per state code (e.g., `"CA"`, `"WA"`).
    states: BTreeMap<String, DeiShardLoadState>,
}

impl DeiShardCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts loading `expected_version_sha256` for `state_code`.
    ///
    /// - If the exact version is already cached -> immediately `Fresh` (zero download).
    /// - If an older version is cached -> `Stale` (older shard stays usable while latest downloads).
    /// - Otherwise -> `Loading { received: 0, total }`.
    pub fn begin_load(
        &mut self,
        state_code: &str,
        expected_version_sha256: &str,
        total_bytes: Option<u64>,
    ) -> &DeiShardLoadState {
        let exact_key = format!("data/{state_code}.json?v={expected_version_sha256}");
        if self.entries.contains_key(&exact_key) {
            self.states.insert(
                state_code.to_owned(),
                DeiShardLoadState::Fresh {
                    version_sha256: expected_version_sha256.to_owned(),
                },
            );
        } else if let Some(older_sha) = self.cached_version_for_state(state_code) {
            self.states.insert(
                state_code.to_owned(),
                DeiShardLoadState::Stale {
                    version_sha256: older_sha,
                },
            );
        } else {
            self.states.insert(
                state_code.to_owned(),
                DeiShardLoadState::Loading {
                    received: 0,
                    total: total_bytes,
                },
            );
        }
        &self.states[state_code]
    }

    /// Updates streaming byte progress while downloading a shard without a stale copy.
    pub fn record_download_progress(
        &mut self,
        state_code: &str,
        received: u64,
        total: Option<u64>,
    ) {
        if matches!(
            self.states.get(state_code),
            Some(DeiShardLoadState::Loading { .. }) | None
        ) {
            self.states.insert(
                state_code.to_owned(),
                DeiShardLoadState::Loading { received, total },
            );
        }
    }

    /// Verifies SHA-256, stores the newly downloaded shard in Cache Storage, prunes all
    /// superseded versions for `state_code`, and transitions to `Fresh`.
    pub fn complete_download(
        &mut self,
        state_code: &str,
        expected_version_sha256: &str,
        raw_json: &str,
    ) -> Result<&DeiShardLoadState, DeiCacheError> {
        let actual = sha256_hex(raw_json.as_bytes());
        if actual != expected_version_sha256 {
            return Err(DeiCacheError::DigestMismatch {
                expected: expected_version_sha256.to_owned(),
                actual,
            });
        }
        let shard: DeiStateShard = serde_json::from_str(raw_json)
            .map_err(|e| DeiCacheError::InvalidShardJson(e.to_string()))?;

        // Prune superseded versions for this state before inserting the new version.
        let prefix = format!("data/{state_code}.json?v=");
        let obsolete_keys: Vec<String> = self
            .entries
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for old_key in obsolete_keys {
            self.entries.remove(&old_key);
        }

        let cache_key = format!("data/{state_code}.json?v={expected_version_sha256}");
        self.entries.insert(cache_key, shard);
        self.states.insert(
            state_code.to_owned(),
            DeiShardLoadState::Fresh {
                version_sha256: expected_version_sha256.to_owned(),
            },
        );
        Ok(&self.states[state_code])
    }

    /// Records a network/fetch failure. Keeps any stale cached version usable under a warning;
    /// otherwise records `Failed { retryable: true, stale_version_sha256: None }`.
    pub fn record_failure(&mut self, state_code: &str) -> &DeiShardLoadState {
        let stale_version = self.cached_version_for_state(state_code);
        self.states.insert(
            state_code.to_owned(),
            DeiShardLoadState::Failed {
                retryable: true,
                stale_version_sha256: stale_version,
            },
        );
        &self.states[state_code]
    }

    /// Reloads `state_code` while offline: returns the cached shard seamlessly if present,
    /// otherwise records an honest retryable failure.
    pub fn reload_offline(&mut self, state_code: &str) -> Option<&DeiStateShard> {
        if let Some(version_sha256) = self.cached_version_for_state(state_code) {
            self.states.insert(
                state_code.to_owned(),
                DeiShardLoadState::Fresh {
                    version_sha256: version_sha256.clone(),
                },
            );
            let key = format!("data/{state_code}.json?v={version_sha256}");
            return self.entries.get(&key);
        }
        self.record_failure(state_code);
        None
    }

    /// Returns the currently cached shard for `state_code`, if any.
    #[must_use]
    pub fn active_shard(&self, state_code: &str) -> Option<&DeiStateShard> {
        let prefix = format!("data/{state_code}.json?v=");
        self.entries
            .iter()
            .find(|(k, _)| k.starts_with(&prefix))
            .map(|(_, shard)| shard)
    }

    /// Returns the current [`DeiShardLoadState`] for `state_code`.
    #[must_use]
    pub fn load_state(&self, state_code: &str) -> Option<&DeiShardLoadState> {
        self.states.get(state_code)
    }

    /// Returns all Cache Storage keys currently retained (for verifying old-version pruning).
    #[must_use]
    pub fn cache_keys(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// Computes the honest UI display mode for `state_code`.
    #[must_use]
    pub fn display_mode(&self, state_code: &str) -> DeiDisplayMode {
        match self.states.get(state_code) {
            Some(DeiShardLoadState::Loading { received, total }) => {
                DeiDisplayMode::PrerenderedWithProgress {
                    received: *received,
                    total: *total,
                }
            }
            Some(DeiShardLoadState::Fresh { version_sha256 }) => {
                let count = self
                    .active_shard(state_code)
                    .map_or(0, |s| s.facilities.len());
                DeiDisplayMode::InteractiveFresh {
                    version_sha256: version_sha256.clone(),
                    facility_count: count,
                }
            }
            Some(DeiShardLoadState::Stale { version_sha256 }) => {
                let count = self
                    .active_shard(state_code)
                    .map_or(0, |s| s.facilities.len());
                DeiDisplayMode::InteractiveStaleWithWarning {
                    version_sha256: version_sha256.clone(),
                    warning: "Showing saved facility list while downloading the latest update.",
                    facility_count: count,
                }
            }
            Some(DeiShardLoadState::Failed {
                stale_version_sha256: Some(version_sha256),
                ..
            }) => {
                let count = self
                    .active_shard(state_code)
                    .map_or(0, |s| s.facilities.len());
                DeiDisplayMode::InteractiveStaleWithWarning {
                    version_sha256: version_sha256.clone(),
                    warning: "Could not refresh the latest directory; showing your last saved list. Try again when online.",
                    facility_count: count,
                }
            }
            Some(DeiShardLoadState::Failed {
                stale_version_sha256: None,
                ..
            })
            | None => DeiDisplayMode::ErrorOverPrerenderedListing {
                alert: "Interactive directory could not load. Showing the static listing below.",
                retry_label: "Try again",
                show_zero_results: false,
            },
        }
    }

    fn cached_version_for_state(&self, state_code: &str) -> Option<String> {
        let prefix = format!("data/{state_code}.json?v=");
        self.entries
            .keys()
            .find_map(|k| k.strip_prefix(&prefix).map(str::to_owned))
    }
}

/// Content-hashed static bundle pair cached atomically by the Dei service worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceWorkerAssetPair {
    pub bundle_hash: String,
    pub js_url: String,
    pub wasm_url: String,
    pub css_url: String,
}

/// Models the Dei service worker cache, retaining matching JS+Wasm asset pairs for every
/// cached visited HTML page so offline reloads never encounter mismatched JS/Wasm versions.
#[derive(Debug, Default)]
pub struct DeiServiceWorkerCache {
    active_bundle: Option<ServiceWorkerAssetPair>,
    retained_bundles: BTreeMap<String, ServiceWorkerAssetPair>,
    cached_pages: BTreeMap<String, String>,
}

impl DeiServiceWorkerCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Precaches a new content-hashed bundle and prunes only bundles not referenced by any cached page.
    pub fn activate_bundle(&mut self, bundle: ServiceWorkerAssetPair) {
        let new_hash = bundle.bundle_hash.clone();
        self.retained_bundles
            .insert(new_hash.clone(), bundle.clone());
        self.active_bundle = Some(bundle);

        let referenced_hashes: BTreeSet<&str> = self
            .cached_pages
            .values()
            .map(String::as_str)
            .chain(std::iter::once(new_hash.as_str()))
            .collect();
        let unreferenced: Vec<String> = self
            .retained_bundles
            .keys()
            .filter(|hash| !referenced_hashes.contains(hash.as_str()))
            .cloned()
            .collect();
        for stale_hash in unreferenced {
            self.retained_bundles.remove(&stale_hash);
        }
    }

    /// Caches a visited page HTML route bound to the currently active bundle pair.
    pub fn cache_visited_page(&mut self, route: &str) {
        if let Some(ref bundle) = self.active_bundle {
            self.cached_pages
                .insert(route.to_owned(), bundle.bundle_hash.clone());
        }
    }

    /// Resolves the complete matching JS+Wasm+CSS bundle for an offline page reload.
    #[must_use]
    pub fn resolve_offline_page_bundle(&self, route: &str) -> Option<&ServiceWorkerAssetPair> {
        let hash = self.cached_pages.get(route)?;
        self.retained_bundles.get(hash)
    }

    /// Returns all currently retained bundle hashes.
    #[must_use]
    pub fn retained_bundle_hashes(&self) -> Vec<&str> {
        self.retained_bundles.keys().map(String::as_str).collect()
    }
}

/// Renders the mandatory crisis-lines banner (988, 911, SAMHSA National Helpline).
#[must_use]
pub fn render_crisis_lines_banner() -> String {
    "    <section id=\"crisis-resources\" class=\"crisis-banner\" aria-label=\"Immediate crisis support\">\n\
      <h2>Immediate Crisis &amp; Helpline Support</h2>\n\
      <ul>\n\
        <li><strong>988 Suicide &amp; Crisis Lifeline:</strong> Call or text <a href=\"tel:988\">988</a> (24/7, free, confidential).</li>\n\
        <li><strong>Medical or Safety Emergency:</strong> Call <a href=\"tel:911\">911</a> immediately.</li>\n\
        <li><strong>SAMHSA National Helpline:</strong> Call <a href=\"tel:18006624357\">1-800-662-4357</a> (24/7 treatment referral and information).</li>\n\
      </ul>\n\
    </section>"
        .to_owned()
}

/// Renders the Dei recovery finder index page with crisis lines strictly before state links.
#[must_use]
pub fn render_dei_index_html(shards: &[DeiStateShard]) -> String {
    let mut main = String::new();
    main.push_str("    <h1>Dei Recovery Facility Finder</h1>\n");
    main.push_str(&render_crisis_lines_banner());
    main.push_str("\n    <section id=\"state-directory\" aria-label=\"Browse by state\">\n");
    main.push_str("      <h2>Browse Recovery Facilities by State</h2>\n      <ul>\n");
    for shard in shards {
        let code_lower = shard.state_code.to_ascii_lowercase();
        main.push_str(&format!(
            "        <li><a href=\"/recovery/{}\">{} ({})</a> — {} listed facilities</li>\n",
            escape_html(&code_lower),
            escape_html(&shard.state_name),
            escape_html(&shard.state_code),
            shard.facilities.len()
        ));
    }
    main.push_str("      </ul>\n    </section>");

    render_semantic_page(
        "Dei Recovery Facility Finder",
        "Find SAMHSA substance-use and mental-health recovery facilities with immediate 988, 911, and SAMHSA helpline access.",
        "/recovery",
        RobotsPolicy::IndexFollow,
        &main,
    )
}

/// Renders a prerendered static state page with crisis lines FIRST, zero-JS HTML search form,
/// complete city directory, and initial facility listing.
#[must_use]
pub fn render_dei_state_html(shard: &DeiStateShard, query: &DeiSearchQuery) -> String {
    let versioned = shard.serialize_with_version();
    let code_lower = shard.state_code.to_ascii_lowercase();
    let route = format!("/recovery/{code_lower}");
    let filtered = shard.filter(query);
    let cities = shard.cities();

    let mut main = String::new();
    main.push_str(&format!(
        "    <h1>{} Recovery Facilities ({})</h1>\n",
        escape_html(&shard.state_name),
        escape_html(&shard.state_code)
    ));
    // Mandatory invariant: crisis lines ALWAYS render BEFORE any facility listing or city directory.
    main.push_str(&render_crisis_lines_banner());
    main.push('\n');

    // Zero-JS HTML <form method="get"> search fallback + data attributes for progressive enhancement.
    let city_val = escape_html(query.city.as_deref().unwrap_or(""));
    let svc_val = escape_html(query.service.as_deref().unwrap_or(""));
    let q_val = escape_html(query.text.as_deref().unwrap_or(""));
    main.push_str(&format!(
        "    <section id=\"finder-controls\" data-shard-url=\"/recovery/{}\" data-shard-sha256=\"{}\">\n\
      <h2>Filter Facilities (Works Without JavaScript)</h2>\n\
      <form method=\"get\" action=\"{}\">\n\
        <label for=\"filter-city\">City</label>\n\
        <input id=\"filter-city\" name=\"city\" type=\"text\" value=\"{city_val}\">\n\
        <label for=\"filter-service\">Service</label>\n\
        <input id=\"filter-service\" name=\"service\" type=\"text\" value=\"{svc_val}\">\n\
        <label for=\"filter-q\">Keyword</label>\n\
        <input id=\"filter-q\" name=\"q\" type=\"search\" value=\"{q_val}\">\n\
        <button type=\"submit\">Filter facilities</button>\n\
      </form>\n\
    </section>\n",
        escape_html(&versioned.cache_key),
        escape_html(&versioned.version_sha256),
        escape_html(&route)
    ));

    // Complete city directory accessible with JavaScript disabled.
    main.push_str("    <nav id=\"city-directory\" aria-label=\"City directory\">\n      <h2>Cities</h2>\n      <ul>\n");
    for city in &cities {
        let city_esc = escape_html(city);
        main.push_str(&format!(
            "        <li><a href=\"{}?city={city_esc}\">{city_esc}</a></li>\n",
            escape_html(&route)
        ));
    }
    main.push_str("      </ul>\n    </nav>\n");

    // Prerendered static facility listing.
    main.push_str("    <section id=\"facilities\" aria-label=\"Recovery facility listing\">\n      <h2>Listed Facilities</h2>\n      <ul>\n");
    for facility in &filtered {
        let services_joined = facility.services.join(", ");
        main.push_str(&format!(
            "        <li data-facility-id=\"{}\"><h3>{}</h3><p>{}, {} — <a href=\"tel:{}\">{}</a></p><p>Services: {}</p></li>\n",
            escape_html(&facility.id),
            escape_html(&facility.name),
            escape_html(&facility.city),
            escape_html(&facility.state_code),
            escape_html(&facility.phone),
            escape_html(&facility.phone),
            escape_html(&services_joined)
        ));
    }
    main.push_str("      </ul>\n    </section>");

    render_semantic_page(
        &format!("{} Recovery Facilities — Dei", shard.state_name),
        &format!(
            "Substance-use and mental-health recovery facilities in {} with 988, 911, and SAMHSA Helpline resources.",
            shard.state_name
        ),
        &route,
        RobotsPolicy::IndexFollow,
        &main,
    )
}

/// Verifies that rendered Dei HTML places 988, 911, and SAMHSA National Helpline strictly
/// before `<section id="facilities"` (or `<section id="state-directory"`).
#[must_use]
pub fn crisis_lines_appear_before_listings(html: &str) -> bool {
    let Some(idx_988) = html.find("988") else {
        return false;
    };
    let Some(idx_911) = html.find("911") else {
        return false;
    };
    let Some(idx_samhsa) = html.find("SAMHSA National Helpline") else {
        return false;
    };

    let listing_idx = html
        .find("id=\"facilities\"")
        .or_else(|| html.find("id=\"state-directory\""));
    let Some(listing_pos) = listing_idx else {
        return false;
    };

    idx_988 < listing_pos && idx_911 < listing_pos && idx_samhsa < listing_pos
}

/// Canonical synthetic SAMHSA state shards used for local verification and zero-JS/offline demos.
#[must_use]
pub fn sample_dei_shards() -> Vec<DeiStateShard> {
    vec![
        DeiStateShard {
            state_code: "WA".to_owned(),
            state_name: "Washington".to_owned(),
            facilities: vec![
                RecoveryFacility {
                    id: "wa-001".to_owned(),
                    name: "Cascade Outpatient Recovery Center".to_owned(),
                    city: "Seattle".to_owned(),
                    state_code: "WA".to_owned(),
                    phone: "206-555-0141".to_owned(),
                    services: vec![
                        "Outpatient".to_owned(),
                        "MAT".to_owned(),
                        "Counseling".to_owned(),
                    ],
                },
                RecoveryFacility {
                    id: "wa-002".to_owned(),
                    name: "Puget Sound Residential Wellness".to_owned(),
                    city: "Tacoma".to_owned(),
                    state_code: "WA".to_owned(),
                    phone: "253-555-0192".to_owned(),
                    services: vec!["Residential".to_owned(), "Detox".to_owned()],
                },
                RecoveryFacility {
                    id: "wa-003".to_owned(),
                    name: "Inland Northwest Recovery Clinic".to_owned(),
                    city: "Spokane".to_owned(),
                    state_code: "WA".to_owned(),
                    phone: "509-555-0177".to_owned(),
                    services: vec!["Outpatient".to_owned(), "Peer Support".to_owned()],
                },
            ],
        },
        DeiStateShard {
            state_code: "OR".to_owned(),
            state_name: "Oregon".to_owned(),
            facilities: vec![
                RecoveryFacility {
                    id: "or-001".to_owned(),
                    name: "Willamette Recovery Collective".to_owned(),
                    city: "Portland".to_owned(),
                    state_code: "OR".to_owned(),
                    phone: "503-555-0133".to_owned(),
                    services: vec!["Outpatient".to_owned(), "MAT".to_owned()],
                },
                RecoveryFacility {
                    id: "or-002".to_owned(),
                    name: "Deschutes Family Recovery House".to_owned(),
                    city: "Bend".to_owned(),
                    state_code: "OR".to_owned(),
                    phone: "541-555-0188".to_owned(),
                    services: vec!["Residential".to_owned(), "Family Therapy".to_owned()],
                },
            ],
        },
    ]
}
