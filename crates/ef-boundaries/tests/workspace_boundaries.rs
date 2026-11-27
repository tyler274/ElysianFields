//! Automated workspace boundary tests enforcing AGENTS.md architecture rules.

use std::path::PathBuf;

use ef_boundaries::{
    BoundaryViolation, audit_workspace_boundaries, is_headless_crate, load_workspace_manifests,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .expect("ef-boundaries must reside under <workspace>/crates/ef-boundaries")
        .to_path_buf()
}

#[test]
fn workspace_manifests_satisfy_all_architecture_boundaries() {
    let root = workspace_root();
    let manifests = load_workspace_manifests(&root).expect("load workspace manifests");
    assert!(
        manifests.iter().any(|m| m.name == "ef-core"),
        "ef-core must be registered in workspace Cargo.toml"
    );
    assert!(
        manifests.iter().any(|m| m.name == "ef-clinic-domain"),
        "ef-clinic-domain must be registered in workspace Cargo.toml"
    );
    assert!(
        manifests.iter().any(|m| m.name == "ef-harness"),
        "ef-harness must be registered in workspace Cargo.toml"
    );

    let violations = audit_workspace_boundaries(&root);
    assert!(
        violations.is_empty(),
        "workspace architecture boundary violations detected: {violations:#?}"
    );
}

#[test]
fn headless_classification_covers_all_m2_headless_crates() {
    for name in [
        "ef-core",
        "ef-clinic-domain",
        "ef-harness",
        "ef-boundaries",
        "ef-clinic-server",
        "ef-mobile-domain",
        "ef-clinic-worker",
    ] {
        assert!(
            is_headless_crate(name),
            "{name} must be classified headless"
        );
    }
    assert!(!is_headless_crate("ef-ui-catalog"));
    assert!(!is_headless_crate("m1-streaming-perf"));
}

#[test]
fn boundary_violation_error_formatting_is_explicit() {
    let v = BoundaryViolation::HeadlessCrateDependsOnBevy {
        crate_name: "ef-clinic-domain".to_owned(),
    };
    assert!(v.to_string().contains("ef-clinic-domain"));
    assert!(v.to_string().contains("bevy"));
}
