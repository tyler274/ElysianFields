# `tyler274/DabneyMoeWebsite` CI Timing Baseline

Empirical baseline of `tyler274/DabneyMoeWebsite` workflow `ci.yml` collected via the GitHub Actions API (`gh run list` and `gh api repos/tyler274/DabneyMoeWebsite/actions/runs/<id>/jobs`). All budgets in [`budgets.md`](budgets.md) and [`budgets.json`](budgets.json) derive directly from these measurements.

## Cohort Summary

Sample window: 12 consecutive completed, successful `pull_request` runs and 12 consecutive completed, successful `push` runs on `main` (2026-11-26 to 2026-11-27 UTC).

| Cohort | Sample (`n`) | Wall Clock Median (exec start -> end) | Wall Clock p90 | Wall Clock Max | Queue + Wall Median (created -> end) |
| --- | ---: | ---: | ---: | ---: | ---: |
| `pull_request` | 12 | 1,368.0 s (22m 48s) | 1,678.6 s (27m 59s) | 1,725.0 s (28m 45s) | 1,370.0 s (22m 50s) |
| `push` (`main`, includes deploy) | 12 | 1,340.0 s (22m 20s) | 1,639.8 s (27m 20s) | 1,981.0 s (33m 01s) | 1,342.5 s (22m 23s) |

## Per-Job Durations

### Pull Request Runs (`n = 12`)

| Job | `n` | Median | p90 | Max |
| --- | ---: | ---: | ---: | ---: |
| `Test suite` | 12 | 1,020.5 s (17m 01s) | 1,483.4 s (24m 43s) | 1,531.0 s (25m 31s) |
| `Build Android app (APK)` | 12 | 990.0 s (16m 30s) | 1,126.6 s (18m 47s) | 1,181.0 s (19m 41s) |
| `Build server OCI image` | 12 | 989.5 s (16m 30s) | 1,292.4 s (21m 32s) | 1,389.0 s (23m 09s) |
| `Build web (Leptos SSR)` | 12 | 922.5 s (15m 23s) | 1,067.7 s (17m 48s) | 1,200.0 s (20m 00s) |
| `Build desktop app (Tauri)` | 12 | 513.5 s (8m 34s) | 598.8 s (9m 59s) | 645.0 s (10m 45s) |
| `Clippy` | 12 | 345.0 s (5m 45s) | 774.4 s (12m 54s) | 898.0 s (14m 58s) |
| `Format (rustfmt + nixpkgs-fmt)` | 12 | 89.5 s (1m 30s) | 139.6 s (2m 20s) | 152.0 s (2m 32s) |
| `Dependency guardrails (cargo-deny)` | 12 | 51.5 s | 59.8 s | 62.0 s |
| `Bootstrap CI base image` | 12 | 10.0 s | 178.2 s (2m 58s) | 205.0 s (3m 25s) |
| `Select CI scope` | 12 | 7.5 s | 9.0 s | 9.0 s |

### `main` Push Runs (`n = 12`)

| Job | `n` | Median | p90 | Max |
| --- | ---: | ---: | ---: | ---: |
| `Build server OCI image` | 12 | 1,015.5 s (16m 56s) | 1,261.3 s (21m 01s) | 1,304.0 s (21m 44s) |
| `Build Android app (APK)` | 12 | 951.5 s (15m 52s) | 1,045.2 s (17m 25s) | 1,135.0 s (18m 55s) |
| `Build web (Leptos SSR)` | 12 | 870.0 s (14m 30s) | 1,223.8 s (20m 24s) | 1,332.0 s (22m 12s) |
| `Test suite` | 12 | 743.0 s (12m 23s) | 1,386.6 s (23m 07s) | 1,561.0 s (26m 01s) |
| `Build desktop app (Tauri)` | 12 | 394.0 s (6m 34s) | 523.2 s (8m 43s) | 577.0 s (9m 37s) |
| `Deploy web (GCP Cloud Run)` | 12 | 274.0 s (4m 34s) | 382.3 s (6m 22s) | 393.0 s (6m 33s) |
| `Clippy` | 12 | 221.5 s (3m 42s) | 857.7 s (14m 18s) | 961.0 s (16m 01s) |
| `Format (rustfmt + nixpkgs-fmt)` | 12 | 103.5 s (1m 44s) | 147.0 s (2m 27s) | 152.0 s (2m 32s) |
| `Dependency guardrails (cargo-deny)` | 12 | 48.0 s | 53.8 s | 63.0 s |
| `Bootstrap CI base image` | 12 | 10.0 s | 12.9 s | 112.0 s |
| `Select CI scope` | 12 | 8.0 s | 9.0 s | 13.0 s |

## Bottleneck Analysis

Top step durations measured across the 24 sampled runs expose five structural causes of the 22–33 minute wall clock:

1. **Monolithic Nix derivation recompiles all release dependencies on every edit.**
   `Build server OCI image :: nix build .#serverImage` alone consumes **907.0 s median (max 1,356 s)** on PRs and **983.0 s median (max 1,258 s)** on `main`. Because third-party dependencies are not compiled in a separate Nix build root (`crane.buildDepsOnly`), any single-line source change recompiles the entire release dependency graph.
2. **Duplicate release compilation in every run.**
   Both `Build web (Leptos SSR)` (`cargo leptos build --release`, **730.5–778.0 s median**, max 1,228 s) and `Build server OCI image` (**907.0–983.0 s median**) compile release artifacts independently in every PR and `main` run instead of building release artifacts once and consuming them downstream.
3. **Coarse path scope runs every platform target on every code edit.**
   Every sampled PR—including UI-only fixes such as run `45980617327` (`fix(ui): fix recovery pagination RSX attribute...`, 1,698 s)—executed `Build Android app (APK)` (990 s median; `Build & verify Android release AAB` 643 s + `cargo tauri android init` 283.5 s), `Build desktop app (Tauri)` (513.5 s median), `Build server OCI image` (989.5 s median), and `Build web` (922.5 s median).
4. **Full server image rebuild and Cloud Run rollout for client-only changes.**
   `Deploy web (GCP Cloud Run)` takes **274.0 s median (max 393 s)**—of which **189.0 s median (max 299 s)** is `Post Magic Nix Cache`—and waits sequentially behind the 1,015.5 s `Build server OCI image` job. Client-only changes therefore pay ~22 minutes to go live.
5. **Monolithic serial `Test suite` job.**
   `Test suite` takes **1,020.5 s median (max 1,531 s)** on PRs because `test: web` (447 s median), `test: dabney-app` (231.5 s median), `test: ui` (192 s median), and `test: forge-linear-sync` (119 s median) build and execute serially in one job without target-level scoping.

## Sampled GitHub Actions Runs

### Pull Request Runs (`n = 12`)

| Run ID | Created (UTC) | Wall Clock | Title | URL |
| --- | --- | ---: | --- | --- |
| `45986675215` | 2026-11-27T10:59:21Z | 1,725 s | feat: Dabney.moe rebrand UX fixes, pure-Rust Gitoxide forge, Linear PR bridge, and EU stack | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45986675215 |
| `45980617327` | 2026-11-27T10:15:54Z | 1,698 s | fix(ui): fix recovery pagination RSX attribute, patient tab aria-selected, and Commissions i18n | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45980617327 |
| `45980393261` | 2026-11-27T10:14:20Z | 1,117 s | feat(ui,clinical): integrate Dabney crest icon and bounded agentic chat harness | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45980393261 |
| `45979755864` | 2026-11-27T10:09:51Z | 1,080 s | feat(clinical): patient EHR portal link-through, portable health passport, and automated ROI dispatch | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45979755864 |
| `45979383476` | 2026-11-27T10:07:12Z | 1,399 s | feat(recovery): shared-staff network, cached LLM extraction, and AZ/CA/CO/NY enrichment | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45979383476 |
| `45977271034` | 2026-11-27T09:52:04Z | 1,022 s | feat(ehr): expand major SMART FHIR vendors, KipuEMR (42 CFR Part 2), C-CDA R2.1, HL7 v2, and SMART Health Links | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45977271034 |
| `45976722167` | 2026-11-27T09:48:04Z | 1,497 s | feat(i18n): add 9-language zero-dependency typed Rust i18n and RTL support | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45976722167 |
| `45965693711` | 2026-11-27T08:26:36Z | 1,365 s | feat(ui): add recovery search, saved dashboard layouts, navigation, and filters | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45965693711 |
| `45905041936` | 2026-11-26T22:28:49Z | 1,371 s | Remove the Discord community policy and activity pages | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45905041936 |
| `45896810954` | 2026-11-26T21:15:00Z | 1,200 s | feat(clinical): patient EHR portal link-through, portable health passport, and automated ROI dispatch | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45896810954 |
| `45884468015` | 2026-11-26T19:33:02Z | 1,504 s | feat(clinical): patient EHR portal link-through, portable health passport, and automated ROI dispatch | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45884468015 |
| `45875567460` | 2026-11-26T18:23:45Z | 1,250 s | infra: add Cloudflare Free edge protection, Anubis PoW fallback, and GCP Cloud Run integration | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45875567460 |

### `main` Push Runs (`n = 12`)

| Run ID | Created (UTC) | Wall Clock | Title | URL |
| --- | --- | ---: | --- | --- |
| `45995903315` | 2026-11-27T12:06:41Z | 1,299 s | feat(dux): mount and link Dux Ancestry & Genetics workbench at /dux/ | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45995903315 |
| `45989835333` | 2026-11-27T11:21:43Z | 1,981 s | Merge pull request #32 from tyler274/feat/rebrand-forge-eu-stack | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45989835333 |
| `45980621967` | 2026-11-27T10:15:56Z | 1,533 s | Merge pull request #45 from tyler274/fix/recovery-pagination-and-a11y | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45980621967 |
| `45979392841` | 2026-11-27T10:07:16Z | 1,641 s | Merge pull request #44 from tyler274/feat/recovery-staff-network | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45979392841 |
| `45976724578` | 2026-11-27T09:48:05Z | 1,180 s | Merge pull request #43 from tyler274/feat/multilingual-i18n | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45976724578 |
| `45965726212` | 2026-11-27T08:26:51Z | 1,629 s | Merge pull request #41 from tyler274/feat/recovery-search-and-nav | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45965726212 |
| `45950239907` | 2026-11-27T06:11:09Z | 1,238 s | feat(ui): polish Curie Clinic & Mobile clinical UX, safety, accessibility | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45950239907 |
| `45942216861` | 2026-11-27T04:50:02Z | 1,071 s | chore(ownership): refresh metrics for integrated dabney-moe@ea5e400a5 | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45942216861 |
| `45926899182` | 2026-11-27T02:11:52Z | 1,381 s | Refresh ownership metrics for dabney-moe Wave 5A+5B & Curie/Moleworks | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45926899182 |
| `45878417775` | 2026-11-26T18:45:16Z | 1,034 s | Merge pull request #39 from tyler274/infra/cloudflare-edge-anubis-fallback | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45878417775 |
| `45828174789` | 2026-11-26T13:20:23Z | 1,132 s | Merge pull request #38 from tyler274/feat/glucose-range-alerts | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45828174789 |
| `45813261750` | 2026-11-26T11:44:20Z | 1,578 s | Merge remote-tracking branch 'origin/main' into release/v1.0.0-codeberg | https://github.com/tyler274/DabneyMoeWebsite/actions/runs/45813261750 |
