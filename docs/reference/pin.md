# Elysian Fields Reference Pin

This document pins the exact `dabney-moe` reference revision, build inputs, toolchain, configuration profiles, workspace member list, and synthetic fixture digests observed on the observer side of the clean-room wall. The machine-readable companion is [`pin.toml`](pin.toml).

## Pinned Revisions (Most Recent Deployed `main`)

### Primary Product Suite (`dabney-moe` `origin/main`)

| Field | Value |
| --- | --- |
| Reference repository | `dabney-moe` (`origin/main`, synced across `gitlab/main`, `codeberg/main`, `atomgit/main`) |
| Git SHA | `01b762838d639a51f82e9d7d15e788e02bac772f` |
| Commit subject | `fix(ancestry): widen painting karyogram hit area/padding and support ANCESTRY_SUBDIR export` |
| Author date | `2026-11-27T04:21:41-07:00` |
| Committer date | `2026-11-27T05:08:01-07:00` |
| Observer read-only worktree | `/home/luluco/code/coda/worktrees/elysian-ref-01b762838` |
| Git submodules present | None |
| `eval/` directories present | None (strictly excluded from all roles) |

### Host Website (`DabneyMoeWebsite` `origin/main`)

| Field | Value |
| --- | --- |
| Reference repository | `DabneyMoeWebsite` (`origin/main`, live deployed build run `45995903315`) |
| Git SHA | `194624690007f82c21be73ba5cde285938917582` (`1946246`) |
| Commit subject | `feat(dux): mount and link Dux Ancestry & Genetics workbench at /dux/` |
| Author date | `2026-11-27T04:45:09-07:00` |
| Committer date | `2026-11-27T04:57:50-07:00` |
| `Cargo.toml` / `Cargo.lock` SHA-256 | `2732f3193f0bbd0adf0fda701c4c7112cbd37d52a3abaff442a0552d8bd78ba9` / `722bc58fa6ec8660806ab7d49e3db150876b08e6436ff67070f25bc42312e20b` |
| `flake.nix` / `flake.lock` SHA-256 | `4ecd2ed6c24fec1b0abd3474da62992e59b0f4e7062ca73881f704436959664e` / `6a0ef327e41be1f51400e925a19498f2f2a1f081dc93a652593f1d1883da841a` |
| Deployed sibling subpaths | `/dux/` (Dux Ancestry & Genetics static Wasm mount; `/dux` redirects to `/dux/`), `/ownership`, `/ownership.json` |
| Target Elysian Fields subpath | `/elysianfields/` (mounts alongside `/dux/` without colliding with domain root `/`) |

## Reference Intake Rule (Decision D3)

`dabney-moe` `origin/main` remains active while Elysian Fields progresses. Per owner decision **D3**:

1. **Fixed baseline:** All M0–M1 observations, contracts, and baselines are anchored to `01b762838d639a51f82e9d7d15e788e02bac772f`. Later commits on `origin/main` do not retroactively alter this pin or silently invalidate existing requirements.
2. **Delta admission:** Any subsequent change on `origin/main` that must be carried into Elysian Fields enters strictly as a **new observation** recorded by `Su · observe` against the named delta SHA, assigned **new requirement IDs** in the requirement ledger, reviewed by `Aponia · spec`, and packaged into a fresh gatekeeper-approved specification package before crossing the wall.
3. **Unadmitted changes stay outside scope:** Changes on `origin/main` not explicitly admitted through the delta intake workflow are not requirements for builders.

## Toolchain & Lockfile Digests

### Core Manifests & Lockfiles

| File | SHA-256 |
| --- | --- |
| `Cargo.toml` | `80fd9c89e1e65b69136fbfeb6097d010cd29bcea60db0feb6b3265289812f57d` |
| `Cargo.lock` | `c13fb9c0aa921c9f64db67d404c5f3b8e0e44764f3075e22e0c3ddd9fcbf2c36` |
| `flake.nix` | `437271b76c819e146c5447b35cc7a87101b9b07d997e78d1331a6f127d7f05a6` |
| `flake.lock` | `5ed3702d598d8d6167a0ef2bf835ad3c47892d4bbef15085fbcc24e977d166ac` |
| `rust-toolchain.toml` | `d3d88a3d3a3624be3676659a0f00185908bcc1a0a3677e93ac23dd331256ead4` |
| `deny.toml` | `e28030697ca05e787559d637b3b07c03aa2c4c793c3f0e4a23fc160356eb8a02` |
| `Dioxus.toml` | `1e6c61991d1cd263b5574d0c681f8d1438d2638e5fe012c9bbc5cb2c190b445a` |
| `doppler.yaml` | `8700167e86e19f6e373b297cc2cd0c4917aa005445ceb42dc454655e7691bd81` |
| `EXPORT-MANIFEST.json` | `637e07b183167f0219e4caa85109fd78e95c14f300a9142571dea881b308a131` |
| `extension/chrome/manifest.json` | `892609621a7f7cc70c1747c0a1749ab1c1c455ad49bba4e3ac375a6f3537adb8` |

### Rust Toolchain Pin

- **Channel:** `1.99.1` (`rustc 1.99.1`, commit `58cf8f9c35b355199727627d07ae34a4e2846d67`, `2026-10-13`, LLVM `23.1.1`)
- **Profile:** `default`
- **Components:** `rustfmt`, `clippy` (plus `rust-src` injected by `nix/dev-shell.nix`)
- **Targets:** `wasm32-unknown-unknown` (host `x86_64-unknown-linux-gnu`; `x86_64-linux-android` added in `.#android`)
- **Wasm bindgen CLI:** `0.2.129` (`wasm-bindgen-0.2.129-x86_64-unknown-linux-musl.tar.gz`, SHA-256 `82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e`)

### Flake Inputs (`flake.lock`)

| Input | Source | Locked Rev | NAR Hash | Last Modified |
| --- | --- | --- | --- | --- |
| `nixpkgs` | `github:NixOS/nixpkgs/nixos-unstable` | `7fd4004d0511991fff3a0524856a9be09e28c5f0` | `sha256-i5qk1kLiit98rRbh1AXmWBNT9QXC/y4l5Wk/u5EdpDQ=` | `1791734076` |
| `rust-overlay` | `github:oxalica/rust-overlay` | `5424e1e6dda30b0626935d2a03a6526db4858d60` | `sha256-arni7i+yH5ZnkvAN9UotvhMml5+fZyXcMHpiOvI3WFY=` | `1792480937` |
| `crane` | `github:ipetkov/crane` | `788369ed71ed9d84ef72cb056e4d920050d5b32c` | `sha256-9EtV4I6Ohw0tjidL31TMOKHhW/QGVG8NuGiemzIaNCs=` | `1793669695` |

## Configuration Profiles

- **Nix devShells (`flake.nix`):**
  - `default`: full developer toolchain + interactive packages, `DOPPLER_PROJECT=dabney-moe`, `DOPPLER_CONFIG=dev`.
  - `agent`: unattended/reproducible profile (same toolchain without interactive IDE binaries; unsets `DOPPLER_*`).
  - `release`: container/OpenTofu release tools (`nix`, `docker-client`, `skopeo`, `opentofu`, `cargo-deny`, `postgresql_16`).
  - `observability`: `opentelemetry-collector-contrib`, `opentofu`.
  - `android` (`x86_64-linux` only): Android SDK platform `35`, build-tools `37.0.0`, NDK, headless emulator, `google_apis` `x86_64` image, `x86_64-linux-android` Rust target, `dioxus-cli`, `jdk17_headless`.
- **Nix package derivations (`nix/packages.nix`, `x86_64-linux`):**
  - Dependency roots: `dabney-servers-deps`, `dabney-web-deps`, `patient-bank-deps`.
  - Container images (8): `physician-sample-image`, `physician-worker-image`, `mobile-api-image`, `physician-migration-image`, `mobile-migration-image`, `mobile-app-privacy-delivery-image`, `availability-probe-image`, `observability-relay-image`.
- **Doppler secret configs:** local `dabney-moe/dev` (`doppler.yaml`); hosted per-product, per-role child configs (`stg_physician_runtime`, `stg_physician_migration`, `stg_mobile_runtime`, `stg_mobile_migration`, and production equivalents). Unit tests, synthetic servers, and Postgres qualification require no Doppler login.
- **Model execution modes (`CURIE_LOCAL_MODEL_EXECUTION`):** `stub` (loopback scripted Responses stub for local scale-stack/browser qualification), `subscription` (developer synthetic-only), `api` (hosted Responses API).
- **Hosted runtime flags:** `PHYSICIAN_AUTH=required`, `PHYSICIAN_DURABLE_CHAT=1`, `CURIE_HOSTED_PROFILE=staging-preview` (for synthetic chart loading in staging).

## Workspace Members (43)

Root `Cargo.toml` (`resolver = "3"`, `edition = "2024"`, `license = "Unlicense"`) declares exactly **43** workspace members:

| # | Workspace Path | Package Name |
| --- | --- | --- |
| 1 | `crates/agent-skills` | `agent-skills` |
| 2 | `crates/ancestry` | `ancestry` |
| 3 | `crates/ancestry-app` | `ancestry-app` |
| 4 | `crates/ancestry-fit` | `ancestry-fit` |
| 5 | `crates/android` | `android` |
| 6 | `crates/availability-probe` | `availability-probe` |
| 7 | `crates/boundaries` | `boundaries` |
| 8 | `crates/clinical-library` | `clinical-library` |
| 9 | `crates/dei-site` | `dei-site` |
| 10 | `crates/encoder` | `encoder` |
| 11 | `crates/extension-popup` | `extension-popup` |
| 12 | `crates/front-door` | `front-door` |
| 13 | `crates/gmail` | `gmail` |
| 14 | `crates/harness` | `harness` |
| 15 | `crates/http` | `curie-http` |
| 16 | `crates/local-product` | `local-product` |
| 17 | `crates/mobile` | `mobile` |
| 18 | `crates/mobile-api` | `mobile-api` |
| 19 | `crates/mobile-app` | `mobile-app` |
| 20 | `crates/mobile-client` | `mobile-client` |
| 21 | `crates/mobile-health` | `mobile-health` |
| 22 | `crates/mobile-notify` | `mobile-notify` |
| 23 | `crates/mobile-stub` | `mobile-stub` |
| 24 | `crates/mobile-wire` | `mobile_wire` |
| 25 | `crates/observability-relay` | `observability-relay` |
| 26 | `crates/patient-bank` | `patient-bank` |
| 27 | `crates/patient-work` | `patient-work` |
| 28 | `crates/physician-app` | `physician-app` |
| 29 | `crates/physician-auth` | `physician-auth` |
| 30 | `crates/physician-bus` | `physician-bus` |
| 31 | `crates/physician-client` | `physician-client` |
| 32 | `crates/physician-demo-api` | `physician-demo-api` |
| 33 | `crates/physician-demo-db` | `physician-demo-db` |
| 34 | `crates/physician-sample` | `physician-sample` |
| 35 | `crates/physician-wire` | `physician-wire` |
| 36 | `crates/public-site` | `public-site` |
| 37 | `crates/recovery-directory` | `recovery-directory` |
| 38 | `crates/spruce` | `spruce` |
| 39 | `crates/telemetry` | `telemetry` |
| 40 | `crates/ui` | `ui` |
| 41 | `crates/ui-gallery` | `ui-gallery` |
| 42 | `crates/workspace-tools` | `workspace-tools` |
| 43 | `crates/workspace-tools/sandbox` | `mole-sandbox` |

## Synthetic Fixtures & Content Digests

Tree digests are computed deterministically as `SHA-256` over sorted `<relative-path>\0<file-sha256>\n` lines. Zero `eval/` paths exist or are included.

| Fixture / Dataset | Path | Count | Digest (SHA-256) |
| --- | --- | --- | --- |
| Synthetic primary-care unit ledgers | `patients/units` | 468 | `eaf67d38eb60bcbdefe49042852b41c78d99600a5dcc9f7eedc6ff23afde47e1` |
| Synthetic patients README | `patients/README.md` | 1 | `888743c47752682d366c59eb1039ca73fd320d44e9d0c5bcca4bece93db7831b` |
| Flagship unit `unit-2013` | `patients/units/unit-2013/ledger.json` | 1 | `223046eeb906717285fb827f8edfadf6102159f81f563ef12102c3439eb0c581` |
| Flagship unit `unit-1672` | `patients/units/unit-1672/ledger.json` | 1 | `b05443d8d8e2d1e31af26c73be2913ceb5312a5e32e4d839693f95cc52bac40e` |
| Flagship unit `unit-2188` | `patients/units/unit-2188/ledger.json` | 1 | `eb525a495a4b8d1abbb936b673b9ebe98f548d1bd9cedbea4f123dbad43c9110` |
| Flagship unit `unit-2797` | `patients/units/unit-2797/ledger.json` | 1 | `409e3c11a496b78ad1725a98adc95f0389b9bbb28981397ff406efc236bc5622` |
| Flagship unit `unit-1670` | `patients/units/unit-1670/ledger.json` | 1 | `05393fe3ed816644d1d2b3a788a0b6a67da3a8126b1a6f642935cd5da2174fcd` |
| Clinical skills package | `skills` | 6 | `32bcacbe6aff70300f82d443a6c6325b24d84e76574397864286f1ac1728ae61` |
| SAMHSA recovery facility shards | `crates/dei-site/data/recovery` | 56 | `8693f59d5b87ec5e1211eabb614523544c77c6707031956af62194ecb1e3ebc7` |
| Legal JSON documents | `crates/public-site/data/legal` | 10 | `159f774864963144fd8857f368cc02d9db15b2ce9b8b1db20f46d4d6f5101eed` |
| UI 9-language i18n catalog | `crates/ui/src/i18n.rs` | 1 | `0f77f2797945d1cfe284b4641c8cec19ee0a29b0e32c5d8c737983332826a597` |
| Translated README docs | `docs/i18n` | 20 | `cedf773e060781f29c7168202a17fae469ebb12b4fc543c7299f8b3912c4784b` |

## Exact Rebuild Commands

Run from a clean checkout of `01b762838d639a51f82e9d7d15e788e02bac772f`:

```bash
# 1. Host workspace binaries & libraries
nix develop .#agent --command cargo build --locked --workspace

# 2. All five WebAssembly client targets (release)
nix develop .#agent --command cargo build --locked --release \
  --target wasm32-unknown-unknown \
  -p physician-app -p mobile-app -p mobile -p ui-gallery -p extension-popup \
  --features physician-app/web,mobile-app/web,mobile/web,ui-gallery/web,extension-popup/web

# 3. Realize synthetic FHIR R4 chart bank
nix develop .#agent --command cargo run --locked -p patient-bank -- build --out target/patient-bank/bank

# 4. Build locked container image (example: physician-sample)
nix build .#physician-sample-image

# 5. Workspace unit test suite
nix develop .#agent --command cargo test --locked --workspace
```
