# Native Desktop Applications

Elysian Fields ships two standalone native desktop GUI applications built on **Bevy `0.20.1`** and **AccessKit `0.24.1`**:

- **Curie Clinic Desktop** (`crates/ef-clinic-desktop`): full 3-column clinical workspace (`1280×800` default window).
- **Curie Mobile Desktop** (`crates/ef-mobile-desktop`): mobile-proportioned personal health companion (`420×860` default window).

Both applications target **Linux x86_64**, **macOS ARM64**, and **Windows x86_64 MSVC** from a single Rust codebase with zero cross-product imports (`ef-boundaries` enforces strict separation between Clinic and Mobile).

## Reactive Rendering and Linux Runtime Portability

- **Reactive Event-Driven Loop**: Both apps configure `WinitSettings::desktop_app()` so idle windows consume near-zero CPU/GPU frames and wake immediately on pointer, keyboard, scroll, resize, or network events.
- **Wayland, X11, and Fontconfig via `dlopen`**: On Linux, `winit` uses `x11`, `wayland`, `wayland-dlopen`, and `wayland-csd-adwaita-notitle` (avoiding unmaintained glyph crates under `cargo-deny`), while `fontique` enables `fontconfig-dlopen` so builds require no compile-time `fontconfig.pc` while discovering system Noto fonts at runtime.
- **Nix Development Shell**: `nix develop` (`flake.nix`) exports `LD_LIBRARY_PATH` with `fontconfig.lib`, `libxkbcommon`, `wayland`, `xorg.libX11`, `xorg.libXcursor`, `xorg.libXi`, `xorg.libXrandr`, and `vulkan-loader` so Vulkan/Wayland and X11 windows launch out-of-the-box on NixOS and Linux.

## Multilingual Typography, RTL Mirroring, and WCAG 2.1 AA UX

- **12 Compile-Time-Exhaustive Locales**: Every UI chrome label across both apps is served from `ef-ui-catalog` across `en`, `es`, `fr`, `pt`, `de`, `zh-Hans`, `ja`, `ko`, `ar`, `he`, `vi`, and `hi` (`107` keys × `12` locales = `1,284` statically checked strings).
- **7-Script Shaping and BiDi**: `FontSource::sans_serif()` resolves system Noto fonts across Latin, Hebrew (with Nikud), Arabic (with Harakat), Simplified Chinese, Japanese, Vietnamese stacked diacritics, and Hindi Devanagari conjuncts, using clean ASCII separators (`|`, `-`) to avoid tofu fallback glyphs.
- **Geometric RTL Mirroring**: Selecting Hebrew (`he`) or Arabic (`ar`) flips layout containers via `FlexDirection::RowReverse` so navigation rails move to the right edge and citation sidebars move to the left edge.
- **WCAG 2.1 AA Themes and Accessibility**: Both apps provide live Dark and Light theme palettes meeting `>= 4.5:1` contrast, zoom scaling (`100%`, `150%`, `200%`), interactive Privacy Cover (`Ctrl/Cmd+Shift+L`, dismissed with `Escape`) and Shortcuts overlay (`?`), scrollable transcript/evidence panels (`MouseWheel`, `PageUp`/`PageDown`), and duplicate-free native AccessKit trees (`AccessibilityNode`).
- **OS-Native Preferences**: Theme, locale, and zoom persist as typed JSON under `$XDG_CONFIG_HOME/elysian-fields/` (Linux), `~/Library/Application Support/ElysianFields/` (macOS), and `%APPDATA%\ElysianFields\` (Windows), separated per product and automatically bypassed under `--headless-smoke`, `--screenshot`, or `--no-prefs`.

## Non-Blocking Live Server Workers and Interactive Workflows

- **Curie Clinic Desktop (`crates/ef-clinic-desktop`)**:
  - Runs network I/O on a dedicated background `LiveServerWorker` thread so UI rendering never stalls on HTTP/SSE calls.
  - Streams assistant responses progressively token-by-token over Server-Sent Events with mid-flight `WorkStop` cancellation (`Escape` or the Cancel button), Retry recovery, `Ctrl/Cmd+K` instant patient roster search, and OS clipboard copy.
  - Verified end-to-end against a live loopback `ef-clinic-server` in `crates/ef-clinic-desktop/tests/live_clinic_worker_e2e.rs`.
- **Curie Mobile Desktop (`crates/ef-mobile-desktop`)**:
  - Provides interactive 4-field forms (sign-in email, invitation code, DOB verification, and curated doctor update notes) driven over a non-blocking `LiveServerWorker` thread.
  - Tracks offline/online sign-out synchronization via typed `OwedRecoveryBannerState` (`SyncedOnline`, `OfflineHolding`, `OnlineFlushPending`) with a live status banner and clickable **Flush Owed Sign-Outs** recovery button.
  - Verified end-to-end against a live loopback `ef-mobile-server` in `crates/ef-mobile-desktop/tests/live_mobile_worker_e2e.rs`.

## Launching and Smoke-Testing Locally

Use `scripts/run_desktop.sh` for offline demo mode, live local server mode, or automated headless verification:

```sh
# Offline demo mode
./scripts/run_desktop.sh clinic --demo --locale en --theme dark
./scripts/run_desktop.sh mobile --demo --locale he --theme light

# Launch paired local headless server on an ephemeral port and connect over HTTP
./scripts/run_desktop.sh clinic --with-server
./scripts/run_desktop.sh mobile --with-server

# Automated headless smoke test for both desktop binaries
./scripts/run_desktop.sh --smoke
```

## CLI Flags

Both `ef-clinic-desktop` and `ef-mobile-desktop` support typed CLI configuration:

- `--demo`: run against in-process deterministic demo state (default).
- `--server <url>`: connect to a live `ef-clinic-server` or `ef-mobile-server` over HTTP/1.1 (`LiveHttpClinicTransport` / `LiveHttpMobileTransport`).
- `--locale <bcp47>`: set initial UI locale (`en`, `he`, `ar`, `zh-Hans`, `ja`, `vi`, `hi`, etc.).
- `--theme <dark|light>`: set initial WCAG 2.1 AA theme.
- `--zoom <100|150|200>`: set initial Clinic UI scale.
- `--headless-smoke`: run full ECS/UI/theme/RTL/a11y smoke verification headlessly, emit JSON, and exit `0`.
- `--screenshot <path.png>`: render a live GPU frame and save a PNG screenshot.
- `--no-prefs`: bypass reading/writing OS-native preference files.
