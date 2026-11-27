# 4. Subpath Mounting (`/elysianfields`), Security Headers / CSP, Accessibility & i18n

## 4.1 Subpath Mounting Under `/elysianfields`

Per target architecture constraints, the web client and its APIs mount on the dedicated `/elysianfields/` subpath alongside sibling routes without owning domain root `/`:

1. **Canonical trailing slash redirect:** `GET` or `HEAD` to bare `/elysianfields` MUST return `308 Permanent Redirect` to `/elysianfields/` preserving any query string (e.g., `/elysianfields?patient=pw-1&chat=c-1` -> `/elysianfields/?patient=pw-1&chat=c-1`).
2. **Segment-bounded path resolution:** Client runtime config derives API and asset base URLs from the mounted subpath (`/elysianfields`) using segment-bounded prefix matching (`/elysianfields` or `/elysianfields/...`, never matching `/elysianfields-other`). All static assets (`.wasm`, `.js`, fonts, icons) load via relative URLs (`./...`) or subpath-prefixed URLs—never hard-coded domain-root `/assets/...` or `/api/...`.
3. **Loopback override safety:** Optional `?api=<url>` query overrides (for local development/testing) MUST be accepted strictly for loopback hosts (`127.0.0.1`, `localhost`, `[::1]`) with `http://` or `https://` scheme and zero userinfo (`@`) or control characters; non-loopback overrides are ignored.
4. **Static cache & Nix store `mtime` hygiene:**
   - Entry HTML (`index.html`) is served with `Cache-Control: no-cache`.
   - Content-hashed / version-queried Wasm, JS, and font assets are served with `Cache-Control: public, max-age=31536000, immutable`.
   - Strip `If-Modified-Since` / `If-Unmodified-Since` on incoming static requests and strip `Last-Modified` on static responses so Nix-store `1970-01-01` timestamps never trigger stale `304 Not Modified` responses or multi-year heuristic browser caching across deploys.
5. **Independent deployability:** Deploying Elysian Fields static client assets or APIs under `/elysianfields/` does not rebuild, overwrite, or collide with sibling routes hosted on the same domain.

## 4.2 Security Headers, Cache Isolation & Strict CSP

- **Clinical & auth responses:** Every `/auth/*`, `/patient-workspaces/*`, `/commands`, `/physician-state/*`, `401`/`403`/`404`/`413`/`429` response MUST carry `Cache-Control: no-store` (and `private, no-store, max-age=0` + `CDN-Cache-Control: no-store` / `Surrogate-Control: no-store` behind any CDN or proxy) so clinical data and bearer tokens are never cached by shared proxies or browsers.
- **Ingress protection:** Enforce per-IP rate limiting (`429 Too Many Requests` with `Retry-After` and `Cache-Control: no-store`), request body size caps (`64 KiB` on auth endpoints, `2 MiB` on `/commands`, returning `413 Payload Too Large`), and bounded concurrent in-flight requests.
- **Zero client secrets:** Provider API keys, database credentials, mail tokens, and signing keys never appear in client bundles, HTML, or Wasm binaries.
- **Strict Content Security Policy (Browser Wasm):**
  - `default-src 'self'`
  - `script-src 'self' 'wasm-unsafe-eval'` (zero `'unsafe-inline'`, zero `'unsafe-eval'`, zero third-party script origins)
  - `connect-src 'self'` (plus explicit loopback origin only in local dev)
  - `img-src 'self' data: blob:`
  - `font-src 'self'` (bundled open-source fonts only; zero third-party font CDNs)
  - `frame-ancestors 'none'` (clickjacking protection; paired with `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`)
- **Token storage discipline:** Active session credentials are never logged, never placed in URL query strings for general navigation (only one-time verification links carry `?code=` which is immediately consumed and stripped from browser history), and cleared immediately on sign-out or revocation.

## 4.3 Accessibility, Keyboard Navigation & Focus Management

### Global & Shell Keyboard Shortcuts

| Shortcut | Scope | Action |
| --- | --- | --- |
| **`Cmd+Shift+L` / `Ctrl+Shift+L`** | **Global (always active, even while typing or inside a modal)** | Toggles the **Privacy Screen Cover**, immediately hiding all clinical content behind an opaque privacy veil (and restoring exact prior focus when uncovered). |
| **`Cmd+K` / `Ctrl+K`** | Workspace (paused inside modal dialogs) | Opens Patient Finder / Search overlay. |
| **`/`** | Workspace (paused while editing text or inside a modal) | Opens Patient Finder / Search overlay. |
| **`?`** | Workspace (paused while editing text or inside a modal) | Opens Keyboard Shortcuts help dialog. |
| **`j` / `ArrowDown`** | Workspace list/navigation (paused while editing or in modal) | Moves focus to next patient/item. |
| **`k` / `ArrowUp`** | Workspace list/navigation (paused while editing or in modal) | Moves focus to previous patient/item. |
| **`Enter`** | When an item is focused (paused while editing or in modal) | Activates the focused item / opens patient or citation. |
| **`Escape`** | Overlays, search, citation pane, mobile drawer | First press clears non-empty search input; next press dismisses the topmost overlay/drawer/citation panel and restores focus to its triggering control. |

### Focus & Screen-Reader Invariants

1. **Single state owner (text & accessibility):** Any text-input, IME, or accessibility bridge (AccessKit / ARIA) MUST have one state owner—never maintaining a second editable copy or duplicate screen-reader tree.
2. **Focus restoration:** Opening a modal dialog, patient finder, or mobile navigation drawer moves focus inside it (with Tab focus trapping for modals); dismissing restores focus to the exact button/control that opened it (e.g., mobile menu button or clicked citation chip).
3. **Interactive controls & landmarks:** Every primary navigation tab, patient row, citation chip, Stop/Resume/Try-again button, and composer field exposes an accessible role, localized label, keyboard focusability (`Tab` / `Shift+Tab`), and visible focus ring (`≥ 2 px`, high contrast).
4. **Live status announcements:** Streaming state transitions (working, stopped, couldn't finish, citation loaded/unavailable, draft saved/conflict/not saved, session ended) emit polite/assertive accessibility live announcements without spamming per-token updates.
5. **Multiline composer ergonomics:** Supports multiline input, OS IME composition (CJK, Hangul, Arabic), selection, copy/paste/cut clipboard, undo/redo, and `Enter` to send vs `Shift+Enter` for newline (with mobile soft-keyboard viewport resize keeping the composer visible).

## 4.4 Internationalization (9 Shipped UI Languages + RTL & Complex Shaping)

### Locale Resolution Order

On startup, client locale resolves in strict priority order:
1. URL query parameter `?lang=<bcp47>` (case-insensitive, supports region/script subtags and `_`, e.g., `es-MX`, `zh-CN`, `zh_Hans`, `ar-EG`).
2. Persisted user preference (`curie.locale.v1`).
3. OS / browser preferred language list (`navigator.languages` / system locale).
4. Default fallback: `en`.

### Nine Shipped UI Locales (Compile-Time Exhaustive Catalog)

All UI controls across Curie Clinic and Curie Mobile (minimum 51 core message keys covering shared controls, Clinic workspace, and Mobile workspace) MUST have compile-time exhaustive translations for all **9 shipped locales**:

| # | BCP-47 Tag | Native Autonym | Short Badge | Script | Layout Direction |
| --- | --- | --- | --- | --- | --- |
| 1 | `en` | English | `EN` | Latin | `ltr` |
| 2 | `es` | Español | `ES` | Latin | `ltr` |
| 3 | `fr` | Français | `FR` | Latin | `ltr` |
| 4 | `pt` | Português | `PT` | Latin | `ltr` |
| 5 | `de` | Deutsch | `DE` | Latin | `ltr` |
| 6 | `zh-Hans` | 简体中文 | `中文` | Simplified Han (CJK) | `ltr` |
| 7 | `ja` | 日本語 | `日本語` | Kanji / Kana | `ltr` |
| 8 | `ko` | 한국어 | `한국어` | Hangul | `ltr` |
| 9 | `ar` | العربية | `العربية` | Arabic | **`rtl`** |

### RTL, Complex Script Shaping & Font Rules

- **RTL layout mirroring:** Switching to `ar` mirrors horizontal layout flow (sidebar rail, message alignment, citation panel edge, directional icons, text alignment, and caret/selection progression) and updates document and accessibility language (`ar`) and layout direction (`rtl`).
- **BiDi mixed clinical text:** Arabic UI labels mixed with LTR clinical tokens (drug names, numeric lab ranges, units like `54 mL/min/1.73m²`, ISO dates) render cleanly under Unicode Bidirectional Algorithm (UBA) without garbling numbers or units.
- **Bundled open-source fonts:** Because GPU-rendered clients cannot rely on CSS OS font fallback in browser Wasm, ship permissively licensed bundled fonts (e.g., SIL OFL 1.1) covering Latin, Arabic, CJK, Hangul, monospace, and complex Indic/Urdu shaping (Devanagari/Bengali/Tamil/Urdu verified in M1 text-shaping qualification).
- **Verbatim clinical record preservation:** Changing the UI locale translates interface chrome only; clinical chart records, quotes, and lab names remain verbatim in their original source language accompanied by the localized original-language provenance note.
