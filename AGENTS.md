# Elysian Realm engineering contract

Elysian Realm is the clean-room Rust workspace for Elysian Fields. This repository owns implementation, verification, build tooling, and durable engineering specifications for the clean-room product suite.

## Clean-room wall

This repository sits strictly behind the clean-room wall:

- Work only from gatekeeper-approved specification packages committed to this repository, explicitly allowed assets, public third-party documentation, and source code written inside this repository.
- Never read, quote, copy, or mirror external reference source code, internal module layouts, crate graphs, DOM selectors, CSS classes, wire internals, or external build caches.
- Never place external repository paths, reference source symbols, production credentials, protected health data, sealed evaluation fixtures, or issue-tracker IDs in repository source, tests, or documentation.
- If a specification package is ambiguous or incomplete, stop and raise a question for the gatekeeper rather than guessing or inspecting anything across the wall.

## Writing code

- **Model the domain in the type system.** Start from meaningful domain distinctions, valid states, and allowed operations. Encode compiler-checkable invariants in types, ownership, and interfaces so components cannot be misused. Prefer static guarantees over caller conventions, and choose the simplest representation that prevents real mistakes. Avoid boolean flags or paired `Option`s that together encode one state (use an enum holding each variant's data), raw strings for codes, IDs, or states (parse once at the boundary into enums or newtypes), string-dispatched entry points (use typed functions), unreachable error variants, and wildcard arms or `#[serde(other)]` that discard exhaustiveness.
- **Use the typed owner, not raw text.** Structured meaning and embedded languages do not travel as spliced strings inside Rust. Persistence queries go through typed APIs, platform or browser bridge behavior lives in dedicated modules accepting structured arguments, and errors are typed values. When an owner genuinely cannot express an operation, document why at the call site and pass values as parameters rather than splicing text.
- **Define each rule once.** Every invariant the system depends on—such as scope membership, subpath normalization, or session validity—lives in one function at its owning module, and every caller delegates to it. Recovery, retry, and resume paths call the same owner as the live path so they cannot drift apart.
- **Preserve error causes.** An error reports both what failed and why. Keep the underlying error as its source instead of flattening it into a plain string, extend the error type that already owns the path rather than introducing a looser one, and choose user-facing wording at the presentation boundary.
- **Make failure look like failure.** Discard a `Result` only when failure genuinely matters to no one. A failed action never reports completion, and a failed read never reports empty data.
- **Keep control flow readable.** Keep related work together with explicit prerequisites, side effects, and exit conditions. Handle terminating cases early so the mainline reads sequentially. Perform an action first and branch on its result rather than folding effects into conditions. Separate unrelated responsibilities when changing one behavior requires reasoning about another.

New state, fields, and variants follow these standards from the start. Workspace-wide Clippy lints in [`Cargo.toml`](Cargo.toml) (`[workspace.lints]`) enforce the mechanical rules on both native host and `wasm32-unknown-unknown` builds with `-D warnings`; independent review covers the rest. Every workspace crate opts in with `[lints] workspace = true`.

## Architecture boundaries

- Keep typed domain state independent of rendering entities and view lifetimes; clients hold no second authoritative store.
- Keep server backends headless; client rendering engines are never dependencies of server-side crates.
- Keep product boundaries enforced: separate products keep separate accounts, domain types, and persistence, sharing code only through explicitly shared crates.
- Mount browser client bundles and APIs under the `/elysianfields` subpath rather than assuming a domain root.
- Provider and tool secrets never ship in any client bundle.

## Local verification and fast CI

Use the single local check entry point that matches CI before pushing:

```sh
scripts/dev-check.sh --tier touched      # changed crates + scripts (default)
scripts/dev-check.sh --tier pre-commit   # staged formatting, Clippy, and script checks
scripts/dev-check.sh --tier full         # full workspace fmt, host + wasm32 Clippy, tests, and script checks
```

Inside Nix, `nix develop` (or `nix develop .#agent --command <cmd>` for unattended runs) provides the pinned Rust toolchain (`rust-toolchain.toml`), `wasm32-unknown-unknown` target, `python3`, `cargo-deny`, `wasm-bindgen-cli`, `wasm-opt`, `mdbook`, and `lychee`.

CI and deploy time budgets are defined in [`docs/ci/budgets.md`](docs/ci/budgets.md) and enforced automatically from [`docs/ci/budgets.json`](docs/ci/budgets.json). Exceeding a time budget fails the CI gate just like a failing test.

## Documentation

Repository documentation states current truth and direction so a cold reader understands what holds today. Write descriptions rather than histories: omit dated changelogs, audit trails, issue-tracker IDs, and wording like "previously" or "we changed X to Y." Update text in place when truth changes; Git history keeps how it got there.
