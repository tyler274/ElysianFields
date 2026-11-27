//! Automated workspace boundary tests proving SCN-M3-0020 (`EF-PLATFORM-0004`)
//! and AGENTS.md architecture rules.

use std::fs;
use std::path::PathBuf;

use ef_boundaries::{
    BoundaryViolation, CrateManifestSummary, audit_manifest_dependencies,
    audit_workspace_boundaries, is_headless_crate, load_workspace_manifests,
    scan_dir_for_mobile_ui_health_bridges,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .expect("ef-boundaries must reside under <workspace>/crates/ef-boundaries")
        .to_path_buf()
}

#[test]
fn scn_m3_0020_workspace_manifests_satisfy_all_architecture_boundaries() {
    let root = workspace_root();
    let manifests = load_workspace_manifests(&root).expect("load workspace manifests");
    for required in [
        "ef-core",
        "ef-clinic-domain",
        "ef-harness",
        "ef-extension",
        "ef-public-site",
    ] {
        assert!(
            manifests.iter().any(|m| m.name == required),
            "{required} must be registered in workspace Cargo.toml"
        );
    }

    let violations = audit_workspace_boundaries(&root);
    assert!(
        violations.is_empty(),
        "workspace architecture boundary violations detected: {violations:#?}"
    );
}

#[test]
fn scn_m3_0020_headless_classification_covers_all_m2_and_m3_headless_crates() {
    for name in [
        "ef-core",
        "ef-clinic-domain",
        "ef-clinic-workflows",
        "ef-harness",
        "ef-boundaries",
        "ef-clinic-server",
        "ef-mobile-domain",
        "ef-mobile-server",
        "ef-connected-care",
        "ef-clinical-lib",
        "ef-encoder",
        "ef-patient-bank",
        "ef-extension",
        "ef-public-site",
        "ef-clinic-worker",
    ] {
        assert!(
            is_headless_crate(name),
            "{name} must be classified headless"
        );
    }
    assert!(!is_headless_crate("ef-ui-catalog"));
    assert!(!is_headless_crate("ef-clinic-client"));
    assert!(!is_headless_crate("ef-mobile-client"));
    assert!(!is_headless_crate("m1-streaming-perf"));
}

#[test]
fn scn_m3_0020_negative_injections_fail_closed_with_explicit_diagnostics() {
    let synthetic_manifests = vec![
        CrateManifestSummary {
            name: "ef-clinic-server".to_owned(),
            path: PathBuf::from("crates/ef-clinic-server/Cargo.toml"),
            dependencies: vec!["bevy".to_owned()],
        },
        CrateManifestSummary {
            name: "ef-mobile-domain".to_owned(),
            path: PathBuf::from("crates/ef-mobile-domain/Cargo.toml"),
            dependencies: vec!["ef-clinic-domain".to_owned()],
        },
        CrateManifestSummary {
            name: "ef-harness".to_owned(),
            path: PathBuf::from("crates/ef-harness/Cargo.toml"),
            dependencies: vec!["ef-mobile-domain".to_owned(), "sqlx".to_owned()],
        },
        CrateManifestSummary {
            name: "ef-clinical-lib".to_owned(),
            path: PathBuf::from("crates/ef-clinical-lib/Cargo.toml"),
            dependencies: vec!["ef-mobile-domain".to_owned()],
        },
        CrateManifestSummary {
            name: "ef-public-site".to_owned(),
            path: PathBuf::from("crates/ef-public-site/Cargo.toml"),
            dependencies: vec!["ef-clinic-domain".to_owned()],
        },
    ];

    let violations = audit_manifest_dependencies(&synthetic_manifests);
    assert!(
        violations.contains(&BoundaryViolation::HeadlessCrateDependsOnBevy {
            crate_name: "ef-clinic-server".to_owned(),
        })
    );
    assert!(violations.contains(&BoundaryViolation::CrossProductImport {
        crate_name: "ef-mobile-domain".to_owned(),
        forbidden_dep: "ef-clinic-domain".to_owned(),
    }));
    assert!(violations.contains(&BoundaryViolation::CrossProductImport {
        crate_name: "ef-harness".to_owned(),
        forbidden_dep: "ef-mobile-domain".to_owned(),
    }));
    assert!(
        violations.contains(&BoundaryViolation::HarnessDependsOnPersistence {
            crate_name: "ef-harness".to_owned(),
            forbidden_dep: "sqlx".to_owned(),
        })
    );
    assert!(
        violations.contains(&BoundaryViolation::SharedCrateImportsProduct {
            crate_name: "ef-clinical-lib".to_owned(),
            forbidden_dep: "ef-mobile-domain".to_owned(),
        })
    );
    assert!(
        violations.contains(&BoundaryViolation::SharedCrateImportsProduct {
            crate_name: "ef-public-site".to_owned(),
            forbidden_dep: "ef-clinic-domain".to_owned(),
        })
    );

    // Negative injection: mobile UI calling native health bridge directly fails closed.
    let temp_dir = std::env::temp_dir().join("ef-boundaries-mobile-ui-neg-test");
    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir).expect("create temp src dir");
    let bad_file = src_dir.join("health_view.rs");
    fs::write(&bad_file, "fn read() { let _ = HKHealthStore::new(); }").expect("write bad source");

    let mut ui_violations = Vec::new();
    scan_dir_for_mobile_ui_health_bridges(&temp_dir, &temp_dir, &mut ui_violations);
    let _ = fs::remove_dir_all(&temp_dir);

    assert!(ui_violations.iter().any(|v| matches!(
        v,
        BoundaryViolation::MobileUiCallsNativeHealthBridge {
            symbol: "HKHealthStore",
            ..
        }
    )));

    // Missing workspace root fails closed with BoundaryViolation::Io.
    let missing = audit_workspace_boundaries(&PathBuf::from("/nonexistent/elysian-workspace-path"));
    assert!(matches!(missing.as_slice(), [BoundaryViolation::Io { .. }]));
}
