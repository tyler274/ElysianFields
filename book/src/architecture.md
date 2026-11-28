# Architecture and Boundaries

Elysian Realm is a clean-room Rust workspace implementing the Elysian Fields suite: **Curie Clinic** (clinical workspace for physicians), **Curie Mobile** (personal health record and recovery companion for patients), the **Public Site and Dei Recovery Finder**, and the **Chrome MV3 Connector Extension**.

## Core Architectural Invariants

1. **Product Isolation**:
   - Curie Clinic and Curie Mobile are distinct products with separate accounts, domain types, HTTP prefixes, WAL stores, and PostgreSQL 18 databases (`ef_clinic` and `ef_mobile`).
   - Data crosses the boundary only through explicit, consent-gated **Connected Care** contracts owned by `ef-connected-care`.
   - Neither product's crates may import the other's crates. This invariant is enforced statically by `ef-boundaries` tests on every PR and `main` build.

2. **Single-Owner Typed Domain State**:
   - Authoritative domain state lives in typed Rust enums, newtypes, and state machines independent of rendering entities and view lifetimes.
   - Browser shells, native Bevy views, and OS shims hold no second authoritative store and project state from the owning domain or server API.
   - Rules such as access scope, subpath normalization, session validity, and privacy-receipt replay live in one owning function so live, retry, and recovery paths cannot drift apart.

3. **Headless Server Isolation**:
   - Server binaries (`ef-clinic-server`, `ef-mobile-server`, `ef-public-site`), the AI harness (`ef-harness`), persistence (`ef-pgwire`), and domain crates never depend on client rendering engines (`bevy` or `accesskit`).
   - `ef-harness` is strictly product-neutral: it imports neither `ef-clinic-*` nor `ef-mobile-*`, nor persistence, nor UI crates.
   - Provider and tool secrets live exclusively on server runtimes and are never compiled into browser Wasm bundles, static web files, extension assets, or desktop binaries.

4. **Strict `/elysianfields` Subpath Contract**:
   - Browser client bundles, Clinic APIs (`/elysianfields/api/*`), Mobile PWA assets (`/elysianfields/mobile/`), and Mobile APIs (`/elysianfields/mobile-api/*`) mount strictly under `/elysianfields` rather than assuming a domain root.
   - Canonical path normalization rejects `..`, encoded traversal, and unmounted root paths fail-closed.
   - Static web and extension surfaces enforce strict Content-Security-Policy (`default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; object-src 'none'`), zero inline scripts, and zero inline styles.

5. **Clean-Room Wall**:
   - All specifications under `specs/`, crates under `crates/`, and documentation under `book/` operate strictly behind the clean-room wall.
   - `scripts/check_wall.py` scans builder-side files automatically in local verification and CI to block reference paths, forbidden framework names, internal selectors, or issue-tracker identifiers.

## Workspace Dependency Flow

```text
  ef-core (IDs, locales, errors, shared primitives)
     ├── ef-ui-catalog (12-locale catalog, WCAG AA tokens, composer, prefs)
     │      ├── ef-clinic-client ──► ef-clinic-desktop (native Bevy GUI)
     │      └── ef-mobile-client ──► ef-mobile-desktop (native Bevy GUI)
     ├── ef-connected-care (consent-gated cross-product bridge)
     ├── ef-harness (product-neutral streaming LLM orchestration)
     ├── ef-pgwire (pure-Rust PostgreSQL Wire Protocol v3.0)
     ├── ef-clinic-domain ─► ef-clinic-workflows ─► ef-clinic-server (headless)
     ├── ef-mobile-domain ────────────────────────► ef-mobile-server (headless)
     ├── ef-clinical-lib, ef-encoder, ef-patient-bank (de-identified search & eval)
     ├── ef-extension (MV3 connector protocol & memory-only session model)
     └── ef-public-site (headless static-HTML public pages & Dei finder)
```
