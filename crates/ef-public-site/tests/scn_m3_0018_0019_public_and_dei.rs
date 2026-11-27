//! Acceptance tests proving SCN-M3-0018 (`EF-PUBLIC-0001`) and SCN-M3-0019 (`EF-PUBLIC-0002`):
//! - Semantic indexable HTML public site, verbatim legal archives, `/invite` noindex,
//!   isolated synthetic physician demo with verbatim citations & turn budgets, ingress rate/body
//!   limits, `no-store` headers, and zero `/grafana/` exposure in production.
//! - Dei SAMHSA recovery finder: crisis lines (988/911/SAMHSA) strictly before listings,
//!   zero-JS fallback + city directory + HTML form filter, Cache Storage versioned shard state
//!   machine (`Loading`/`Fresh`/`Stale`/`Failed`), offline reload, old-version pruning,
//!   service-worker JS+Wasm pair retention, and failure-never-shows-zero-results guarantee.

use ef_public_site::{
    DeiDisplayMode, DeiSearchQuery, DeiServiceWorkerCache, DeiShardCache, DeiShardLoadState,
    DeiStateShard, DemoTurnOutcome, DeploymentMode, LegalArchiveRegistry, PUBLIC_MAX_BODY_BYTES,
    PublicIngressGuard, PublicRequest, PublicSiteRouter, RecoveryFacility, ServiceWorkerAssetPair,
    SyntheticDemoEngine, crisis_lines_appear_before_listings, sample_dei_shards,
};
use serde_json::json;

#[test]
fn scn_m3_0018_semantic_public_and_legal_routes_render_verbatim_without_js_or_canvas() {
    let mut router = PublicSiteRouter::new(DeploymentMode::Production);

    let indexable_routes = [
        "/",
        "/mobile",
        "/privacy",
        "/terms",
        "/clinical-transparency",
        "/mobile/privacy",
        "/mobile/privacy/2025-11-01",
        "/mobile/privacy/2026-03-01",
        "/mobile/terms",
        "/mobile/terms/2025-11-01",
        "/mobile/consumer-health-data",
    ];

    for route in indexable_routes {
        let resp = router.handle(&PublicRequest::get(route, "203.0.113.10"), 1_000);
        assert_eq!(resp.status, 200, "route {route} must return 200");
        assert!(
            resp.body.contains("<!DOCTYPE html>"),
            "{route} missing doctype"
        );
        assert!(
            resp.body.contains("<main"),
            "{route} missing semantic <main>"
        );
        assert!(
            !resp.body.to_ascii_lowercase().contains("<canvas"),
            "{route} must never use canvas rendering"
        );
        assert!(
            resp.body.contains("content=\"index, follow\""),
            "{route} must be indexable"
        );
    }

    // Legal archive verbatim JSON round-trip check.
    let archive_json = router
        .legal()
        .to_json_pretty()
        .expect("legal archive exports JSON");
    let reloaded =
        LegalArchiveRegistry::from_json_str(&archive_json).expect("legal archive reloads verbatim");
    let archived_doc = reloaded
        .get("/mobile/privacy/2025-11-01")
        .expect("dated archive must exist");
    let rendered_archive = router
        .handle(
            &PublicRequest::get("/mobile/privacy/2025-11-01", "203.0.113.10"),
            1_010,
        )
        .body;
    for section in &archived_doc.sections {
        assert!(rendered_archive.contains(&ef_public_site::escape_html(&section.heading)));
        for paragraph in &section.paragraphs {
            assert!(rendered_archive.contains(&ef_public_site::escape_html(paragraph)));
        }
    }

    // /invite emits noindex, nofollow (meta + header) and App Store / Play Store fallback links.
    let invite = router.handle(&PublicRequest::get("/invite", "203.0.113.10"), 1_020);
    assert_eq!(invite.status, 200);
    assert!(invite.body.contains("content=\"noindex, nofollow\""));
    assert_eq!(
        invite.headers.get("X-Robots-Tag").map(String::as_str),
        Some("noindex, nofollow")
    );
    assert!(invite.body.contains("App Store"));
    assert!(invite.body.contains("Google Play"));

    // /robots.txt, /sitemap.xml, /health, /readyz
    let robots = router.handle(&PublicRequest::get("/robots.txt", "203.0.113.10"), 1_030);
    assert_eq!(robots.status, 200);
    assert!(robots.body.contains("Disallow: /invite"));
    let sitemap = router.handle(&PublicRequest::get("/sitemap.xml", "203.0.113.10"), 1_040);
    assert_eq!(sitemap.status, 200);
    assert!(sitemap.body.contains("/mobile/privacy/2025-11-01"));
    assert!(sitemap.body.contains("/recovery/wa"));

    for probe in ["/health", "/readyz"] {
        let resp = router.handle(&PublicRequest::get(probe, "203.0.113.10"), 1_050);
        assert_eq!(resp.status, 200);
        assert_eq!(
            resp.headers.get("Cache-Control").map(String::as_str),
            Some("no-store")
        );
        assert_eq!(
            resp.headers.get("CDN-Cache-Control").map(String::as_str),
            Some("no-store")
        );
    }

    // /grafana/ is strictly absent (404 + no-store) in production configuration.
    let grafana = router.handle(&PublicRequest::get("/grafana/", "203.0.113.10"), 1_060);
    assert_eq!(grafana.status, 404);
    assert_eq!(
        grafana.headers.get("Cache-Control").map(String::as_str),
        Some("no-store")
    );
}

#[test]
fn scn_m3_0018_synthetic_demo_isolation_citations_turn_budget_and_ingress_limits() {
    let mut router = PublicSiteRouter::new(DeploymentMode::Production);
    router.set_demo_engine(SyntheticDemoEngine::new(2));

    let post_turn = |router: &mut PublicSiteRouter, visitor: &str, prompt: &str, ts: u64| {
        let body = serde_json::to_vec(&json!({
            "visitor_id": visitor,
            "prompt": prompt
        }))
        .expect("serialize turn");
        router.handle(
            &PublicRequest::post_json("/demo/turn", "198.51.100.7", body),
            ts,
        )
    };

    let r1 = post_turn(&mut router, "visitor-a", "What is the latest A1c?", 2_000);
    assert_eq!(r1.status, 200);
    assert_eq!(
        r1.headers.get("Cache-Control").map(String::as_str),
        Some("no-store")
    );
    let o1: DemoTurnOutcome = serde_json::from_str(&r1.body).expect("parse turn 1");
    let DemoTurnOutcome::Answered {
        record: rec1,
        turns_remaining: rem1,
    } = o1
    else {
        panic!("expected answered turn 1");
    };
    assert_eq!(rem1, 1);
    assert!(router.demo().verify_citations_verbatim("visitor-a", &rec1));

    let r2 = post_turn(&mut router, "visitor-a", "List active medications", 2_100);
    assert_eq!(r2.status, 200);
    let o2: DemoTurnOutcome = serde_json::from_str(&r2.body).expect("parse turn 2");
    assert!(matches!(
        o2,
        DemoTurnOutcome::Answered {
            turns_remaining: 0,
            ..
        }
    ));

    // Turn 3 exhausts visitor-a's budget cleanly while visitor-b stays isolated with fresh budget.
    let r3 = post_turn(
        &mut router,
        "visitor-a",
        "What about home blood pressure?",
        2_200,
    );
    assert_eq!(r3.status, 429);
    let o3: DemoTurnOutcome = serde_json::from_str(&r3.body).expect("parse budget exhaustion");
    assert!(matches!(
        o3,
        DemoTurnOutcome::BudgetExhausted { max_turns: 2, .. }
    ));

    let rb = post_turn(&mut router, "visitor-b", "Home blood pressure?", 2_300);
    assert_eq!(rb.status, 200);
    assert_eq!(router.demo().active_session_count(), 2);

    // Ingress body cap (> 64 KiB -> 413) and per-IP rate limiting (-> 429) with no-store.
    let oversized = PublicRequest::post_json(
        "/demo/turn",
        "198.51.100.8",
        vec![b'a'; PUBLIC_MAX_BODY_BYTES + 1],
    );
    let resp_413 = router.handle(&oversized, 3_000);
    assert_eq!(resp_413.status, 413);
    assert_eq!(
        resp_413.headers.get("Cache-Control").map(String::as_str),
        Some("no-store")
    );

    router.set_ingress_guard(PublicIngressGuard::new(2, 10_000));
    assert_eq!(
        router
            .handle(&PublicRequest::get("/", "198.51.100.9"), 4_000)
            .status,
        200
    );
    assert_eq!(
        router
            .handle(&PublicRequest::get("/", "198.51.100.9"), 4_100)
            .status,
        200
    );
    let resp_429 = router.handle(&PublicRequest::get("/", "198.51.100.9"), 4_200);
    assert_eq!(resp_429.status, 429);
    assert_eq!(
        resp_429.headers.get("Cache-Control").map(String::as_str),
        Some("no-store")
    );
}

#[test]
fn scn_m3_0019_dei_crisis_lines_first_zero_js_fallback_and_versioned_shard_offline_lifecycle() {
    let mut router = PublicSiteRouter::new(DeploymentMode::Production);

    // 1. Index & state page with JS disabled: crisis lines strictly precede facility listings.
    let index_resp = router.handle(&PublicRequest::get("/recovery", "203.0.113.20"), 5_000);
    assert_eq!(index_resp.status, 200);
    assert!(crisis_lines_appear_before_listings(&index_resp.body));

    let wa_resp = router.handle(&PublicRequest::get("/recovery/wa", "203.0.113.20"), 5_010);
    assert_eq!(wa_resp.status, 200);
    assert!(crisis_lines_appear_before_listings(&wa_resp.body));
    assert!(wa_resp.body.contains("id=\"city-directory\""));
    assert!(wa_resp.body.contains("Seattle"));
    assert!(wa_resp.body.contains("Tacoma"));
    assert!(wa_resp.body.contains("Spokane"));
    assert!(wa_resp.body.contains("Cascade Outpatient Recovery Center"));

    // Zero-JS HTML form query filters without JavaScript.
    let wa_filtered = router.handle(
        &PublicRequest::get("/recovery/wa?city=Seattle&service=MAT", "203.0.113.20"),
        5_020,
    );
    assert_eq!(wa_filtered.status, 200);
    assert!(crisis_lines_appear_before_listings(&wa_filtered.body));
    assert!(
        wa_filtered
            .body
            .contains("Cascade Outpatient Recovery Center")
    );
    assert!(
        !wa_filtered
            .body
            .contains("Puget Sound Residential Wellness")
    );

    // 2. Fresh Cache Storage shard download + JSON schema shape verification.
    let shards = sample_dei_shards();
    let wa_v1 = shards
        .into_iter()
        .find(|s| s.state_code == "WA")
        .expect("WA sample shard");
    let v1_payload = wa_v1.serialize_with_version();

    let mut cache = DeiShardCache::new();
    let loading = cache.begin_load(
        "WA",
        &v1_payload.version_sha256,
        Some(v1_payload.json.len() as u64),
    );
    assert!(matches!(
        loading,
        DeiShardLoadState::Loading { received: 0, .. }
    ));
    let loading_json = serde_json::to_value(loading).expect("serialize loading");
    assert_eq!(loading_json["state"], "loading");

    cache.record_download_progress("WA", 128, Some(v1_payload.json.len() as u64));
    let fresh = cache
        .complete_download("WA", &v1_payload.version_sha256, &v1_payload.json)
        .expect("v1 download must verify sha256");
    assert_eq!(
        fresh,
        &DeiShardLoadState::Fresh {
            version_sha256: v1_payload.version_sha256.clone()
        }
    );
    let fresh_json = serde_json::to_value(fresh).expect("serialize fresh");
    assert_eq!(fresh_json["state"], "fresh");
    assert_eq!(fresh_json["version_sha256"], v1_payload.version_sha256);

    // 3. Offline reload filters cached shard seamlessly + service worker retains matching JS+Wasm pair.
    let mut sw = DeiServiceWorkerCache::new();
    let bundle_v1 = ServiceWorkerAssetPair {
        bundle_hash: "h1111".to_owned(),
        js_url: "/recovery/assets/dei.h1111.js".to_owned(),
        wasm_url: "/recovery/assets/dei.h1111.wasm".to_owned(),
        css_url: "/recovery/assets/dei.h1111.css".to_owned(),
    };
    sw.activate_bundle(bundle_v1.clone());
    sw.cache_visited_page("/recovery/wa");

    let offline_shard = cache
        .reload_offline("WA")
        .expect("offline reload must serve cached WA shard");
    let tacoma_hits = offline_shard.filter(&DeiSearchQuery {
        city: Some("Tacoma".to_owned()),
        ..DeiSearchQuery::default()
    });
    assert_eq!(tacoma_hits.len(), 1);
    assert_eq!(tacoma_hits[0].name, "Puget Sound Residential Wellness");
    assert_eq!(
        sw.resolve_offline_page_bundle("/recovery/wa"),
        Some(&bundle_v1)
    );

    // 4. Server restarts with a newer shard moving facility Tacoma -> Olympia;
    //    cache enters Stale (usable), downloads v2, prunes v1, and SW keeps v1 pair for cached page.
    let mut wa_v2: DeiStateShard = wa_v1.clone();
    wa_v2.facilities[1] = RecoveryFacility {
        city: "Olympia".to_owned(),
        ..wa_v2.facilities[1].clone()
    };
    let v2_payload = wa_v2.serialize_with_version();
    assert_ne!(v1_payload.version_sha256, v2_payload.version_sha256);

    let stale = cache.begin_load(
        "WA",
        &v2_payload.version_sha256,
        Some(v2_payload.json.len() as u64),
    );
    assert_eq!(
        stale,
        &DeiShardLoadState::Stale {
            version_sha256: v1_payload.version_sha256.clone()
        }
    );
    assert!(matches!(
        cache.display_mode("WA"),
        DeiDisplayMode::InteractiveStaleWithWarning {
            facility_count: 3,
            ..
        }
    ));

    cache
        .complete_download("WA", &v2_payload.version_sha256, &v2_payload.json)
        .expect("v2 upgrade must succeed");
    assert_eq!(
        cache.cache_keys(),
        vec![v2_payload.cache_key.as_str()],
        "superseded v1 shard must be pruned from Cache Storage"
    );
    let olympia_hits = cache
        .active_shard("WA")
        .expect("active v2 shard")
        .filter(&DeiSearchQuery {
            city: Some("Olympia".to_owned()),
            ..DeiSearchQuery::default()
        });
    assert_eq!(olympia_hits.len(), 1);

    let bundle_v2 = ServiceWorkerAssetPair {
        bundle_hash: "h2222".to_owned(),
        js_url: "/recovery/assets/dei.h2222.js".to_owned(),
        wasm_url: "/recovery/assets/dei.h2222.wasm".to_owned(),
        css_url: "/recovery/assets/dei.h2222.css".to_owned(),
    };
    sw.activate_bundle(bundle_v2);
    // Cached /recovery/wa page still resolves its complete matching v1 JS+Wasm pair!
    assert_eq!(
        sw.resolve_offline_page_bundle("/recovery/wa"),
        Some(&bundle_v1)
    );

    // 5. Fetch failure with NO cached shard displays error alert + Try again and NEVER 0 results.
    let failed = cache.record_failure("NV");
    assert_eq!(
        failed,
        &DeiShardLoadState::Failed {
            retryable: true,
            stale_version_sha256: None
        }
    );
    let failed_json = serde_json::to_value(failed).expect("serialize failed");
    assert_eq!(failed_json["state"], "failed");
    assert_eq!(failed_json["retryable"], true);
    assert_eq!(
        cache.display_mode("NV"),
        DeiDisplayMode::ErrorOverPrerenderedListing {
            alert: "Interactive directory could not load. Showing the static listing below.",
            retry_label: "Try again",
            show_zero_results: false,
        }
    );
}
