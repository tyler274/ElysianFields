/**
 * Strict-CSP, zero-secret browser shell for Elysian Fields mounted under `/elysianfields/`.
 *
 * Responsibilities:
 * - Enforces segment-bounded `/elysianfields` subpath mounting without domain-root assumptions.
 * - Mounts the single-owner ARIA tree via `./a11y-bridge.js` (`ElysianNarrowA11yBridge`).
 * - Supports 12 locales / 7 complex writing systems (`en`, `he`, `ar`, `zh-Hans`, `ja`, `vi`, `hi`),
 *   reading Rust-owned `snapshot.chrome` from Wasm when loaded with full fallback coverage.
 * - Connects to live `/elysianfields/api/...` HTTP/SSE endpoints when backed by `ef-clinic-server`.
 * - Handles WebGPU/WebGL2 capability detection and in-place `webglcontextlost`/`webglcontextrestored`.
 */

import { ElysianNarrowA11yBridge } from "./a11y-bridge.js";
import { getLocaleMeta, mergeChrome, normalizeLocaleTag } from "./locales.js";
import {
  isWasmModuleAdvertised,
  loadBevyWasmClient,
  syncRustA11yNodes,
} from "./wasm-bridge.js";

const MOUNT_PREFIX = "/elysianfields";
const THEME_STORAGE_KEY = "curie.theme.v1";

function resolveInitialTheme(searchParams) {
  const fromQuery = searchParams.get("theme");
  if (fromQuery === "light" || fromQuery === "dark") {
    return fromQuery;
  }
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (stored === "light" || stored === "dark") {
      return stored;
    }
  } catch (_err) {
    // Ignore restricted storage contexts.
  }
  if (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: light)").matches
  ) {
    return "light";
  }
  return "dark";
}

function applyThemeToDocument(theme, chrome, persist = false) {
  const normalized = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = normalized;
  document.documentElement.dataset.efTheme = normalized;
  if (persist) {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, normalized);
    } catch (_err) {
      // Ignore storage quota or privacy restrictions.
    }
  }
  const toggleBtn = document.getElementById("btn-toggle-theme");
  if (toggleBtn) {
    toggleBtn.setAttribute("aria-pressed", normalized === "light" ? "true" : "false");
    if (chrome) {
      toggleBtn.textContent = normalized === "light" ? chrome.theme_to_dark : chrome.theme_to_light;
    }
  }
  return normalized;
}

function isEditableFocusTarget(target) {
  if (!target || typeof target !== "object") return false;
  const tag = String(target.tagName || "").toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") {
    return true;
  }
  return Boolean(target.isContentEditable);
}

export function isWithinMount(pathname) {
  return pathname === MOUNT_PREFIX || pathname.startsWith(`${MOUNT_PREFIX}/`);
}

export function resolveMountUrl(relativePath) {
  const cleaned = String(relativePath).replace(/^\/+/, "");
  return `${window.location.origin}${MOUNT_PREFIX}/${cleaned}`;
}

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = String(value);
  }
}

function parseSseEvents(rawText) {
  const events = [];
  const blocks = rawText.split(/\r?\n\r?\n/);
  for (const block of blocks) {
    const trimmed = block.trim();
    if (!trimmed) continue;
    let eventName = "message";
    let eventId = null;
    const dataLines = [];
    for (const line of trimmed.split(/\r?\n/)) {
      if (line.startsWith("event:")) {
        eventName = line.slice(6).trim();
      } else if (line.startsWith("id:")) {
        eventId = line.slice(3).trim();
      } else if (line.startsWith("data:")) {
        dataLines.push(line.slice(5).trim());
      }
    }
    const rawData = dataLines.join("\n");
    let data = rawData;
    try {
      data = JSON.parse(rawData);
    } catch (_err) {
      // Keep raw text when frame data is non-JSON.
    }
    events.push({ event: eventName, id: eventId, data });
  }
  return events;
}

export class ClinicApiClient {
  constructor() {
    this.sessionToken = null;
  }

  #headers(extra = {}) {
    const headers = { ...extra };
    if (this.sessionToken) {
      headers.Authorization = `Bearer ${this.sessionToken}`;
    }
    return headers;
  }

  async requestSignInLink(email) {
    const resp = await fetch(resolveMountUrl("api/auth/request-link"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify({ email }),
    });
    return { status: resp.status, body: await resp.json() };
  }

  async verifySignIn(email, token) {
    const resp = await fetch(resolveMountUrl("api/auth/verify"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify({ email, token }),
    });
    const body = await resp.json();
    if (resp.ok && body && typeof body.session_id === "string") {
      this.sessionToken = body.session_id;
    }
    return { status: resp.status, body };
  }

  async listPatients(query = "") {
    const suffix = query ? `api/patients?q=${encodeURIComponent(query)}` : "api/patients";
    const resp = await fetch(resolveMountUrl(suffix), {
      headers: this.#headers(),
    });
    return { status: resp.status, body: await resp.json() };
  }

  async getWorkspace(workspaceId) {
    const resp = await fetch(
      resolveMountUrl(`api/patients/${encodeURIComponent(workspaceId)}/workspace`),
      { headers: this.#headers() },
    );
    return { status: resp.status, body: await resp.json() };
  }

  async getCitation(workspaceId, sourceKey) {
    const resp = await fetch(
      resolveMountUrl(
        `api/patients/${encodeURIComponent(workspaceId)}/citations/${encodeURIComponent(sourceKey)}`,
      ),
      { headers: this.#headers() },
    );
    return { status: resp.status, body: await resp.json() };
  }

  async getChatSnapshot(workspaceId, chatId) {
    const resp = await fetch(
      resolveMountUrl(
        `api/workspaces/${encodeURIComponent(workspaceId)}/chats/${encodeURIComponent(chatId)}`,
      ),
      { headers: this.#headers() },
    );
    return { status: resp.status, body: await resp.json() };
  }

  async submitChatCommand(workspaceId, chatId, envelope) {
    const resp = await fetch(
      resolveMountUrl(
        `api/workspaces/${encodeURIComponent(workspaceId)}/chats/${encodeURIComponent(chatId)}/commands`,
      ),
      {
        method: "POST",
        headers: this.#headers({ "Content-Type": "application/json" }),
        body: JSON.stringify(envelope),
      },
    );
    return { status: resp.status, body: await resp.json() };
  }

  async readChatStream(workspaceId, chatId, afterEventId = null) {
    const query = afterEventId ? `?after=${encodeURIComponent(afterEventId)}` : "";
    const resp = await fetch(
      resolveMountUrl(
        `api/workspaces/${encodeURIComponent(workspaceId)}/chats/${encodeURIComponent(chatId)}/stream${query}`,
      ),
      { headers: this.#headers({ Accept: "text/event-stream" }) },
    );
    const text = await resp.text();
    return { status: resp.status, events: parseSseEvents(text) };
  }
}

async function detectGraphicsCapabilities(canvas) {
  let webgpuAvailable = false;
  if ("gpu" in navigator && navigator.gpu && typeof navigator.gpu.requestAdapter === "function") {
    try {
      const adapter = await navigator.gpu.requestAdapter();
      webgpuAvailable = adapter !== null;
    } catch (_err) {
      webgpuAvailable = false;
    }
  }

  let webgl2Context = null;
  if (canvas) {
    try {
      webgl2Context = canvas.getContext("webgl2", {
        antialias: true,
        powerPreference: "default",
      });
    } catch (_err) {
      webgl2Context = null;
    }
  }

  return {
    webgpuAvailable,
    webgl2Available: webgl2Context !== null,
    webgl2Context,
  };
}

function attachContextLossRecovery(canvas, state, bridge) {
  if (!canvas) return;
  canvas.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    state.contextState = "paused-for-recovery";
    state.preservedDraft = document.getElementById("composer-draft")?.value ?? "";
    setText("metric-context", state.contextState);
    setText("status-banner", "Graphics context lost — rendering paused, UI state preserved.");
    bridge.applyPatches([
      {
        kind: "SetAssertiveLive",
        text: "Graphics context lost. Rendering paused and draft preserved.",
      },
    ]);
  });

  canvas.addEventListener("webglcontextrestored", () => {
    state.contextState = "restored-without-reload";
    state.surfaceGeneration += 1;
    const composer = document.getElementById("composer-draft");
    if (composer && typeof state.preservedDraft === "string") {
      composer.value = state.preservedDraft;
    }
    setText("metric-context", `${state.contextState} (gen ${state.surfaceGeneration})`);
    setText("status-banner", "Graphics context restored in-place without page reload.");
    bridge.applyPatches([
      {
        kind: "SetPoliteLive",
        text: "Graphics surface restored without reloading the page.",
      },
    ]);
  });
}

function buildInitialA11yPatches(draftValue, chrome) {
  return [
    {
      kind: "UpsertElement",
      element: {
        nodeId: "100",
        controlKey: "nav-patient-roster",
        role: "navigation",
        name: chrome.patient_roster,
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "101",
        controlKey: "btn-patient-1",
        role: "button",
        name: chrome.sample_patient,
        selected: true,
        tabOrder: "Sequential",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "200",
        controlKey: "main-clinical-workspace",
        role: "main",
        name: chrome.workspace_heading,
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "201",
        controlKey: "region-transcript-stream",
        role: "region",
        name: chrome.transcript_heading,
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "202",
        controlKey: "textbox-composer",
        role: "textbox",
        name: chrome.composer_label,
        value: draftValue,
        tabOrder: "Sequential",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "203",
        controlKey: "button-send-prompt",
        role: "button",
        name: chrome.send,
        tabOrder: "Sequential",
      },
    },
    {
      kind: "SetPoliteLive",
      text: `${chrome.status_ready} (${MOUNT_PREFIX}/).`,
    },
  ];
}

async function loadOptionalWasmBundle(capabilities, bundleBaseName = "m1_web_delivery") {
  const startMs = performance.now();
  const glueUrl = resolveMountUrl(`pkg/${bundleBaseName}.js`);
  const wasmUrl = resolveMountUrl(`pkg/${bundleBaseName}_bg.wasm`);

  const moduleNs = await import(glueUrl);
  const exports = await moduleNs.default({ module_or_path: wasmUrl });
  const probeCode =
    typeof exports.elysian_m1_delivery_probe === "function"
      ? exports.elysian_m1_delivery_probe(
          capabilities.webgpuAvailable ? 1 : 0,
          capabilities.webgl2Available ? 1 : 0,
        )
      : 0;
  const elapsedMs = performance.now() - startMs;
  return { probeCode, elapsedMs };
}

function appendListItem(listId, text) {
  const list = document.getElementById(listId);
  if (!list) return;
  const li = document.createElement("li");
  li.className = "transcript-item";
  li.dir = "auto";
  li.textContent = text;
  list.appendChild(li);
}

function applyClinicDocumentLocale(
  lang,
  explicitDirection = null,
  wasmSnapshot = null,
  replaceDraft = false,
) {
  const normalizedLang = normalizeLocaleTag(lang);
  const meta = getLocaleMeta(normalizedLang);
  const dir =
    explicitDirection === "rtl" || explicitDirection === "ltr"
      ? explicitDirection
      : meta.dir;
  const chrome = mergeChrome(normalizedLang, wasmSnapshot);

  document.documentElement.lang = normalizedLang;
  document.documentElement.dir = dir;
  document.documentElement.dataset.efLang = normalizedLang;
  document.documentElement.dataset.efDir = dir;
  document.documentElement.dataset.efScript = meta.script;
  document.documentElement.dataset.efAutonym = wasmSnapshot?.autonym || meta.autonym;

  const selectEl = document.getElementById("locale-select");
  if (selectEl && selectEl.value !== normalizedLang) {
    selectEl.value = normalizedLang;
  }

  setText("clinic-title", chrome.app_brand);
  setText("label-locale", chrome.language_label);
  setText("heading-roster", chrome.patient_roster);
  setText("label-signin-email", chrome.clinician_email);
  setText("btn-signin", chrome.sign_in);
  setText("btn-refresh-roster", chrome.reload_roster);
  setText("sample-patient-btn", chrome.sample_patient);
  setText("heading-workspace", chrome.workspace_heading);
  setText("heading-evidence", chrome.evidence_heading);
  setText("sample-evidence-item", chrome.sample_evidence);
  setText("heading-transcript", chrome.transcript_heading);
  setText("sample-transcript-item", chrome.sample_transcript);
  setText("label-composer", chrome.composer_label);
  setText("btn-send-prompt", chrome.send);
  setText("btn-load-wasm", chrome.load_wasm);
  setText("heading-diagnostics", chrome.diagnostics_heading);
  setText("btn-privacy-cover", chrome.privacy_cover_btn);
  setText("privacy-cover-title", chrome.privacy_cover_title);
  setText("privacy-cover-hint", chrome.privacy_cover_hint);
  setText("btn-dismiss-privacy", chrome.privacy_cover_dismiss);
  setText("btn-shortcuts", chrome.shortcuts_btn);
  setText("shortcuts-title", chrome.shortcuts_title);
  setText("shortcut-privacy-desc", chrome.shortcuts_privacy_desc);
  setText("shortcut-help-desc", chrome.shortcuts_help_desc);
  setText("shortcut-escape-desc", chrome.shortcuts_escape_desc);
  setText("btn-close-shortcuts", chrome.shortcuts_close);
  setText("metric-locale", `${dir} / ${normalizedLang} (${meta.script})`);

  const activeTheme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
  const themeBtn = document.getElementById("btn-toggle-theme");
  if (themeBtn) {
    themeBtn.textContent = activeTheme === "light" ? chrome.theme_to_dark : chrome.theme_to_light;
  }

  const composerEl = document.getElementById("composer-draft");
  if (composerEl) {
    composerEl.placeholder = chrome.composer_placeholder;
    if (replaceDraft) {
      composerEl.value = chrome.composer_placeholder;
    }
  }

  return { lang: normalizedLang, dir, script: meta.script, chrome };
}

async function boot() {
  const navStartMs = performance.now();
  const state = {
    cspViolations: 0,
    contextState: "healthy",
    surfaceGeneration: 1,
    preservedDraft: "",
    activeWorkspaceId: "ws-demo-1",
    lastAction: null,
    lang: "en",
    dir: "ltr",
    script: "Latn",
    theme: "dark",
    privacyCoverActive: false,
    shortcutsOpen: false,
    wasmRuntime: "static-shell",
    wasmAbiVersion: null,
    lastWasmSnapshot: null,
    userEditedDraft: false,
  };

  document.addEventListener("securitypolicyviolation", () => {
    state.cspViolations += 1;
    setText("metric-csp-violations", state.cspViolations);
    document.documentElement.dataset.efCspViolations = String(state.cspViolations);
    document.documentElement.dataset.m1CspViolations = String(state.cspViolations);
  });

  const pathname = window.location.pathname;
  const withinMount = isWithinMount(pathname);
  setText("metric-subpath", withinMount ? `${pathname} (valid)` : `${pathname} (invalid-mount)`);

  const searchParams = new URLSearchParams(window.location.search);
  const initialTheme = resolveInitialTheme(searchParams);
  state.theme = applyThemeToDocument(initialTheme, null, false);
  const requestedLang = searchParams.get("lang") || "en";
  let localeInfo = applyClinicDocumentLocale(requestedLang, null, null, true);
  state.lang = localeInfo.lang;
  state.dir = localeInfo.dir;
  state.script = localeInfo.script;
  state.theme = applyThemeToDocument(state.theme, localeInfo.chrome, false);

  const workspaceGrid = document.querySelector(".workspace-grid");
  const privacyCoverEl = document.getElementById("privacy-cover");
  const dismissPrivacyBtn = document.getElementById("btn-dismiss-privacy");
  const shortcutsDialogEl = document.getElementById("shortcuts-dialog");
  const closeShortcutsBtn = document.getElementById("btn-close-shortcuts");
  let priorFocusEl = null;

  const restorePriorFocus = () => {
    const target = priorFocusEl;
    priorFocusEl = null;
    if (target && target.isConnected && typeof target.focus === "function") {
      target.focus();
    }
  };

  const closeShortcutsDialog = (restoreFocus = true) => {
    if (!shortcutsDialogEl || shortcutsDialogEl.hidden) return;
    shortcutsDialogEl.hidden = true;
    state.shortcutsOpen = false;
    document.documentElement.dataset.efShortcutsOpen = "false";
    if (restoreFocus) {
      restorePriorFocus();
    }
  };

  const openShortcutsDialog = () => {
    if (!shortcutsDialogEl || state.privacyCoverActive) return;
    if (shortcutsDialogEl.hidden) {
      priorFocusEl = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    }
    shortcutsDialogEl.hidden = false;
    state.shortcutsOpen = true;
    document.documentElement.dataset.efShortcutsOpen = "true";
    if (closeShortcutsBtn) {
      closeShortcutsBtn.focus();
    }
  };

  const setPrivacyCoverActive = (active) => {
    if (!privacyCoverEl) return;
    const next = Boolean(active);
    if (next) {
      closeShortcutsDialog(false);
      if (privacyCoverEl.hidden) {
        priorFocusEl = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      }
      privacyCoverEl.hidden = false;
      state.privacyCoverActive = true;
      document.documentElement.dataset.efPrivacyCover = "active";
      if (workspaceGrid) {
        workspaceGrid.setAttribute("aria-hidden", "true");
        workspaceGrid.classList.add("is-concealed");
      }
      if (dismissPrivacyBtn) {
        dismissPrivacyBtn.focus();
      }
    } else if (!privacyCoverEl.hidden || state.privacyCoverActive) {
      privacyCoverEl.hidden = true;
      state.privacyCoverActive = false;
      document.documentElement.dataset.efPrivacyCover = "inactive";
      if (workspaceGrid) {
        workspaceGrid.removeAttribute("aria-hidden");
        workspaceGrid.classList.remove("is-concealed");
      }
      restorePriorFocus();
    }
  };

  document.documentElement.dataset.efPrivacyCover = "inactive";
  document.documentElement.dataset.efShortcutsOpen = "false";

  const themeToggleBtn = document.getElementById("btn-toggle-theme");
  if (themeToggleBtn) {
    themeToggleBtn.addEventListener("click", () => {
      const nextTheme = state.theme === "light" ? "dark" : "light";
      state.theme = applyThemeToDocument(nextTheme, localeInfo.chrome, true);
    });
  }

  const privacyCoverBtn = document.getElementById("btn-privacy-cover");
  if (privacyCoverBtn) {
    privacyCoverBtn.addEventListener("click", () => {
      setPrivacyCoverActive(!state.privacyCoverActive);
    });
  }

  if (dismissPrivacyBtn) {
    dismissPrivacyBtn.addEventListener("click", () => {
      setPrivacyCoverActive(false);
    });
  }

  if (privacyCoverEl) {
    privacyCoverEl.addEventListener("click", (event) => {
      if (event.target === privacyCoverEl) {
        setPrivacyCoverActive(false);
      }
    });
  }

  const shortcutsBtn = document.getElementById("btn-shortcuts");
  if (shortcutsBtn) {
    shortcutsBtn.addEventListener("click", () => {
      if (state.shortcutsOpen) {
        closeShortcutsDialog(true);
      } else {
        openShortcutsDialog();
      }
    });
  }

  if (closeShortcutsBtn) {
    closeShortcutsBtn.addEventListener("click", () => {
      closeShortcutsDialog(true);
    });
  }

  if (shortcutsDialogEl) {
    shortcutsDialogEl.addEventListener("click", (event) => {
      if (event.target === shortcutsDialogEl) {
        closeShortcutsDialog(true);
      }
    });
  }

  document.addEventListener("keydown", (event) => {
    const key = String(event.key || "");
    if ((event.ctrlKey || event.metaKey) && event.shiftKey && key.toLowerCase() === "l") {
      event.preventDefault();
      setPrivacyCoverActive(!state.privacyCoverActive);
      return;
    }
    if (key === "Escape") {
      if (state.privacyCoverActive) {
        event.preventDefault();
        setPrivacyCoverActive(false);
        return;
      }
      if (state.shortcutsOpen) {
        event.preventDefault();
        closeShortcutsDialog(true);
        return;
      }
    }
    if (
      key === "?" &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !isEditableFocusTarget(event.target)
    ) {
      event.preventDefault();
      if (state.shortcutsOpen) {
        closeShortcutsDialog(true);
      } else {
        openShortcutsDialog();
      }
    }
  });

  const appRoot = document.getElementById("elysian-app") ?? document.body;
  const composerEl = document.getElementById("composer-draft");
  if (composerEl) {
    composerEl.addEventListener("input", () => {
      state.userEditedDraft = true;
    });
  }

  let wasmClient = null;
  const bridge = new ElysianNarrowA11yBridge(
    appRoot,
    (action) => {
      state.lastAction = action;
      if (action.type === "set_value" && composerEl && typeof action.value === "string") {
        composerEl.value = action.value;
        state.userEditedDraft = true;
        if (wasmClient) {
          const updated = wasmClient.dispatch({
            cmd: "set_composer_text",
            text: action.value,
          });
          state.lastWasmSnapshot = updated;
          syncRustA11yNodes(bridge, updated);
          setText("metric-a11y-nodes", bridge.projectedNodeCount());
        }
      }
    },
    { mountSubpath: MOUNT_PREFIX },
  );

  let probeCode = "standby";
  let wasmInitMs = null;
  let a11yOwner = "shell";

  const explicitLegacyBundle = searchParams.get("bundle");
  const shouldLoadClinicWasm =
    !explicitLegacyBundle &&
    searchParams.get("wasm") !== "0" &&
    (searchParams.get("wasm") === "1" ||
      (await isWasmModuleAdvertised(resolveMountUrl("wasm-manifest.json"), "clinicWasm")));

  if (shouldLoadClinicWasm) {
    try {
      wasmClient = await loadBevyWasmClient({
        wasmUrl: resolveMountUrl("ef_clinic_client.wasm"),
        initFn: "ef_clinic_wasm_init",
        versionFn: "ef_clinic_wasm_version",
        dispatchFn: "ef_clinic_wasm_dispatch_json",
      });
      let snapshot = wasmClient.initialSnapshot;
      if (requestedLang) {
        const localized = wasmClient.dispatch({
          cmd: "set_locale",
          lang: normalizeLocaleTag(requestedLang),
        });
        if (localized && localized.ok) {
          snapshot = localized;
        }
      }
      if (composerEl && composerEl.value) {
        const synced = wasmClient.dispatch({
          cmd: "set_composer_text",
          text: composerEl.value,
        });
        if (synced && synced.ok) {
          snapshot = synced;
        }
      }
      state.wasmRuntime = snapshot.runtime || "bevy-rust-wasm";
      state.wasmAbiVersion = wasmClient.abiVersion;
      state.lastWasmSnapshot = snapshot;
      a11yOwner = "rust-accesskit";
      probeCode = `${state.wasmRuntime}-v${wasmClient.abiVersion}`;
      wasmInitMs = wasmClient.loadMs;
      localeInfo = applyClinicDocumentLocale(
        snapshot.locale || requestedLang,
        snapshot.direction,
        snapshot,
        !state.userEditedDraft,
      );
      state.lang = localeInfo.lang;
      state.dir = localeInfo.dir;
      state.script = localeInfo.script;
      syncRustA11yNodes(
        bridge,
        snapshot,
        "Curie Clinic Rust AccessKit tree mounted under /elysianfields/.",
      );
    } catch (_err) {
      wasmClient = null;
      bridge.applyPatches(buildInitialA11yPatches(composerEl?.value ?? "", localeInfo.chrome));
    }
  } else {
    bridge.applyPatches(buildInitialA11yPatches(composerEl?.value ?? "", localeInfo.chrome));
  }

  const switchClinicLocale = (nextLang) => {
    const normalized = normalizeLocaleTag(nextLang);
    if (wasmClient) {
      const snap = wasmClient.dispatch({ cmd: "set_locale", lang: normalized });
      if (snap && snap.ok) {
        state.lastWasmSnapshot = snap;
        localeInfo = applyClinicDocumentLocale(
          snap.locale || normalized,
          snap.direction,
          snap,
          !state.userEditedDraft,
        );
        state.lang = localeInfo.lang;
        state.dir = localeInfo.dir;
        state.script = localeInfo.script;
        const count = syncRustA11yNodes(bridge, snap);
        setText("metric-a11y-nodes", count);
        document.documentElement.dataset.efA11yNodes = String(count);
        setText("status-banner", `${localeInfo.chrome.status_ready} \u2066(${localeInfo.dir.toUpperCase()} · ${localeInfo.lang})\u2069.`);
        return localeInfo;
      }
    }
    localeInfo = applyClinicDocumentLocale(normalized, null, null, !state.userEditedDraft);
    state.lang = localeInfo.lang;
    state.dir = localeInfo.dir;
    state.script = localeInfo.script;
    bridge.applyPatches(buildInitialA11yPatches(composerEl?.value ?? "", localeInfo.chrome));
    setText("status-banner", `${localeInfo.chrome.status_ready} \u2066(${localeInfo.dir.toUpperCase()} · ${localeInfo.lang})\u2069.`);
    return localeInfo;
  };

  const localeSelect = document.getElementById("locale-select");
  if (localeSelect) {
    localeSelect.addEventListener("change", (event) => {
      switchClinicLocale(event.target.value);
    });
  }

  const nodeCount = bridge.projectedNodeCount();
  setText("metric-a11y-nodes", nodeCount);

  const canvas = document.getElementById("elysian-canvas");
  attachContextLossRecovery(canvas, state, bridge);

  const capabilities = await detectGraphicsCapabilities(canvas);
  const backendLabel = capabilities.webgpuAvailable
    ? "webgpu"
    : capabilities.webgl2Available
      ? "webgl2"
      : "unsupported";
  setText("metric-backend", backendLabel);

  if (backendLabel === "unsupported") {
    state.contextState = "unsupported-fallback";
    setText("metric-context", state.contextState);
    const fallbackPanel = document.getElementById("fallback-panel");
    if (fallbackPanel) {
      fallbackPanel.hidden = false;
    }
  } else {
    setText("metric-context", `healthy (gen ${state.surfaceGeneration})`);
  }

  const apiClient = new ClinicApiClient();

  const authForm = document.getElementById("auth-form");
  if (authForm) {
    authForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const emailInput = document.getElementById("signin-email");
      const email = emailInput?.value?.trim() ?? "";
      if (!email) return;
      try {
        const res = await apiClient.requestSignInLink(email);
        setText("metric-api-state", `sign-in requested (${res.status})`);
      } catch (_err) {
        setText("metric-api-state", "offline-preview");
      }
    });
  }

  const composerForm = document.getElementById("composer-form");
  if (composerForm) {
    composerForm.addEventListener("submit", (event) => {
      event.preventDefault();
      const text = composerEl?.value?.trim() ?? "";
      if (!text) return;
      appendListItem("transcript-list", text);
      if (wasmClient) {
        const updated = wasmClient.dispatch({ cmd: "set_composer_text", text });
        state.lastWasmSnapshot = updated;
        syncRustA11yNodes(bridge, updated, `Prompt queued for ${state.activeWorkspaceId}.`);
      } else {
        bridge.applyPatches([
          {
            kind: "SetPoliteLive",
            text: `Prompt queued for ${state.activeWorkspaceId}.`,
          },
        ]);
      }
    });
  }

  const loadWasmBtn = document.getElementById("btn-load-wasm");
  if (loadWasmBtn) {
    loadWasmBtn.addEventListener("click", async () => {
      try {
        const { probeCode: loadedCode, elapsedMs } = await loadOptionalWasmBundle(capabilities);
        setText("metric-probe-code", loadedCode);
        setText("metric-cold-start", `${elapsedMs.toFixed(1)} ms (wasm)`);
      } catch (_err) {
        setText("metric-probe-code", "not-packaged");
      }
    });
  }

  // Optional legacy M1 bundle loader when explicitly requested via ?wasm=1&bundle=...
  if (searchParams.get("wasm") === "1" && explicitLegacyBundle) {
    try {
      const result = await loadOptionalWasmBundle(capabilities, explicitLegacyBundle);
      probeCode = String(result.probeCode);
      wasmInitMs = result.elapsedMs;
    } catch (err) {
      probeCode = "error";
      document.documentElement.dataset.m1Error = err instanceof Error ? err.message : String(err);
    }
  }

  const totalColdStartMs = performance.now() - navStartMs;
  setText(
    "metric-cold-start",
    wasmInitMs === null
      ? `${totalColdStartMs.toFixed(1)} ms (shell)`
      : `${totalColdStartMs.toFixed(1)} ms (wasm ${wasmInitMs.toFixed(1)} ms)`,
  );
  setText("metric-probe-code", probeCode);
  setText(
    "status-banner",
    backendLabel === "unsupported"
      ? "Hardware-accelerated browser graphics unavailable; accessible shell active."
      : `${localeInfo.chrome.status_ready} \u2066(${backendLabel} · ${localeInfo.dir.toUpperCase()} · ${localeInfo.lang} · ${nodeCount} ARIA · ${state.cspViolations} CSP)\u2069.`,
  );

  document.documentElement.dataset.efReady = "true";
  document.documentElement.dataset.efSubpathValid = String(withinMount);
  document.documentElement.dataset.efBackend = backendLabel;
  document.documentElement.dataset.efA11yNodes = String(nodeCount);
  document.documentElement.dataset.efA11yOwner = a11yOwner;
  document.documentElement.dataset.efWasmRuntime = state.wasmRuntime;
  if (state.wasmAbiVersion !== null) {
    document.documentElement.dataset.efWasmAbiVersion = String(state.wasmAbiVersion);
  }
  document.documentElement.dataset.efCspViolations = String(state.cspViolations);
  document.documentElement.dataset.efColdStartMs = totalColdStartMs.toFixed(2);

  // Preserve M1 diagnostic dataset compatibility.
  document.documentElement.dataset.m1Ready = probeCode === "error" ? "error" : "true";
  document.documentElement.dataset.m1Backend = backendLabel;
  document.documentElement.dataset.m1ProbeCode = probeCode;
  document.documentElement.dataset.m1CspViolations = String(state.cspViolations);
  document.documentElement.dataset.m1ColdStartMs = totalColdStartMs.toFixed(2);
  if (wasmInitMs !== null) {
    document.documentElement.dataset.m1WasmInitMs = wasmInitMs.toFixed(2);
  }

  window.__elysianShell = Object.freeze({
    mountPrefix: MOUNT_PREFIX,
    apiClient,
    bridge,
    setLocale: (lang) => switchClinicLocale(lang),
    setTheme: (theme) => {
      state.theme = applyThemeToDocument(theme, localeInfo.chrome, true);
      return state.theme;
    },
    setPrivacyCover: (active) => {
      setPrivacyCoverActive(active);
      return state.privacyCoverActive;
    },
    openShortcuts: () => openShortcutsDialog(),
    closeModals: () => {
      setPrivacyCoverActive(false);
      closeShortcutsDialog(true);
    },
    dispatchWasm: (cmd) => {
      if (!wasmClient) return null;
      const snap = wasmClient.dispatch(cmd);
      state.lastWasmSnapshot = snap;
      if (snap && snap.ok) {
        localeInfo = applyClinicDocumentLocale(snap.locale, snap.direction, snap, false);
        state.lang = localeInfo.lang;
        state.dir = localeInfo.dir;
        state.script = localeInfo.script;
        syncRustA11yNodes(bridge, snap);
        const count = bridge.projectedNodeCount();
        setText("metric-a11y-nodes", count);
        document.documentElement.dataset.efA11yNodes = String(count);
      }
      return snap;
    },
    getState: () => ({ ...state, a11yNodeCount: bridge.projectedNodeCount() }),
    loadWasmBundle: (bundleName) => loadOptionalWasmBundle(capabilities, bundleName),
  });
}

boot();
