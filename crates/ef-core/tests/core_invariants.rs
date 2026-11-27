//! Integration tests covering `ef-core` invariants (IDs, product boundaries,
//! `/elysianfields` subpath, loopback overrides, 9-locale i18n, WCAG AA themes, refusals).

use ef_core::{
    CachePolicy, IdError, LOCALE_STORAGE_KEY, Locale, LoopbackApiOverride, PhysicianId,
    ProductBoundary, RefusalCode, RequestId, StructuredRefusal, Subpath, SubpathError,
    TextDirection, ThemeMode, ThemePalette, WorkspaceId,
};

#[test]
fn typed_ids_reject_empty_and_control_chars_and_roundtrip_json() {
    assert_eq!(
        WorkspaceId::new("   "),
        Err(IdError::Empty {
            kind: "WorkspaceId"
        })
    );
    assert_eq!(
        PhysicianId::new("dr\n1"),
        Err(IdError::ControlCharacter {
            kind: "PhysicianId"
        })
    );

    let ws = WorkspaceId::new("pw-flagship-1").expect("valid workspace id");
    let json = serde_json::to_string(&ws).expect("serialize");
    assert_eq!(json, "\"pw-flagship-1\"");
    let parsed: WorkspaceId = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, ws);

    let r1 = RequestId::new(1);
    let r2 = r1.next();
    assert!(r2 > r1);
    assert_eq!(r2.as_u64(), 2);
}

#[test]
fn product_boundaries_keep_clinic_and_mobile_strictly_isolated() {
    assert_eq!(ProductBoundary::CurieClinic.slug(), "curie-clinic");
    assert_eq!(ProductBoundary::CurieMobile.slug(), "curie-mobile");
    assert!(ProductBoundary::CurieClinic.shares_storage_with(ProductBoundary::CurieClinic));
    assert!(ProductBoundary::CurieMobile.shares_storage_with(ProductBoundary::CurieMobile));
    assert!(!ProductBoundary::CurieClinic.shares_storage_with(ProductBoundary::CurieMobile));
}

#[test]
fn subpath_and_loopback_override_satisfy_m2_platform_rules() {
    assert!(Subpath::is_within_mount("/elysianfields"));
    assert!(Subpath::is_within_mount("/elysianfields/"));
    assert!(Subpath::is_within_mount("/elysianfields/api/auth/whoami"));
    assert!(!Subpath::is_within_mount("/"));
    assert!(!Subpath::is_within_mount("/elysianfields-other/app"));
    assert!(!Subpath::is_within_mount("/elysianfields2"));

    assert_eq!(
        Subpath::canonical_redirect_if_bare("/elysianfields"),
        Some("/elysianfields/".to_owned())
    );
    assert_eq!(
        Subpath::canonical_redirect_if_bare("/elysianfields?patient=pw-1&chat=c-1"),
        Some("/elysianfields/?patient=pw-1&chat=c-1".to_owned())
    );
    assert_eq!(Subpath::canonical_redirect_if_bare("/elysianfields/"), None);

    assert_eq!(
        Subpath::resolve_asset("fonts/inter.woff2").expect("asset"),
        "/elysianfields/assets/fonts/inter.woff2"
    );
    assert_eq!(
        Subpath::resolve_api("v1/commands").expect("api"),
        "/elysianfields/api/v1/commands"
    );
    assert_eq!(
        Subpath::resolve_asset("../secret.txt"),
        Err(SubpathError::PathTraversal)
    );
    assert_eq!(
        Subpath::resolve_api("https://attacker.example/steal"),
        Err(SubpathError::ExternalUrlNotAllowed)
    );

    assert!(LoopbackApiOverride::parse("http://127.0.0.1:8080").is_ok());
    assert!(LoopbackApiOverride::parse("https://localhost:8443/elysianfields").is_ok());
    assert!(LoopbackApiOverride::parse("http://[::1]:3000").is_ok());
    assert_eq!(
        LoopbackApiOverride::parse("https://attacker.example"),
        Err(SubpathError::NonLoopbackOverrideRejected)
    );
    assert_eq!(
        LoopbackApiOverride::parse("http://user@127.0.0.1:8080"),
        Err(SubpathError::NonLoopbackOverrideRejected)
    );

    assert_eq!(
        CachePolicy::EntryHtmlNoCache.cache_control_header(),
        "no-cache"
    );
    assert!(CachePolicy::EntryHtmlNoCache.strips_last_modified());
    assert_eq!(
        CachePolicy::ClinicalNoStore.cdn_cache_control_header(),
        Some("no-store")
    );
}

#[test]
fn nine_locales_and_startup_priority_match_spec() {
    assert_eq!(LOCALE_STORAGE_KEY, "curie.locale.v1");
    assert_eq!(Locale::ALL.len(), 9);
    assert_eq!(Locale::Ar.direction(), TextDirection::Rtl);
    for locale in Locale::ALL {
        if locale != Locale::Ar {
            assert_eq!(locale.direction(), TextDirection::Ltr);
        }
        assert_eq!(Locale::parse_bcp47(locale.bcp47()), Some(locale));
    }

    assert_eq!(Locale::parse_bcp47("es-MX"), Some(Locale::Es));
    assert_eq!(Locale::parse_bcp47("zh_CN"), Some(Locale::ZhHans));
    assert_eq!(Locale::parse_bcp47("ar-EG"), Some(Locale::Ar));
    assert_eq!(Locale::parse_bcp47("zz-unknown"), None);

    // Priority: ?lang= > curie.locale.v1 > OS preferred > en.
    assert_eq!(
        Locale::resolve_startup(Some("ar-EG"), Some("fr"), &["de"]),
        Locale::Ar
    );
    assert_eq!(
        Locale::resolve_startup(Some("invalid"), Some("zh_Hans"), &["ja"]),
        Locale::ZhHans
    );
    assert_eq!(
        Locale::resolve_startup(None, None, &["xx", "ko-KR"]),
        Locale::Ko
    );
    assert_eq!(Locale::resolve_startup(None, None, &[]), Locale::En);
}

#[test]
fn light_and_dark_palettes_pass_wcag_aa_and_refusals_serialize_cleanly() {
    assert!(ThemePalette::for_mode(ThemeMode::Light).meets_wcag_aa());
    assert!(ThemePalette::for_mode(ThemeMode::Dark).meets_wcag_aa());

    let ended = StructuredRefusal::session_ended();
    assert_eq!(ended.reason, RefusalCode::SessionEnded);
    assert_eq!(ended.reason.http_status_code(), 401);
    let removed = StructuredRefusal::access_removed();
    assert_eq!(removed.reason.http_status_code(), 403);
    let json = serde_json::to_string(&removed).expect("serialize refusal");
    assert!(json.contains("\"reason\":\"access_removed\""));
    assert!(json.contains("\"code\":\"access_removed\""));
}
