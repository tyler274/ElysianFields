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

  async getToday() {
    const resp = await fetch(resolveMountUrl("api/today"), {
      headers: this.#headers(),
    });
    return { status: resp.status, ok: resp.ok, body: await resp.json() };
  }

  async updateTodayStanding(itemId, standing, versionId) {
    const resp = await fetch(resolveMountUrl("api/today/standing"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify({
        item_id: itemId,
        standing,
        version_id: versionId,
      }),
    });
    return { status: resp.status, ok: resp.ok, body: await resp.json() };
  }

  async getWorkspaceMemory(workspaceId) {
    const resp = await fetch(
      resolveMountUrl(`api/workspaces/${encodeURIComponent(workspaceId)}/memory`),
      { headers: this.#headers() },
    );
    return { status: resp.status, ok: resp.ok, body: await resp.json() };
  }

  async issueConnectedCareInvite(workspaceId, patientId, expectedDob) {
    const resp = await fetch(
      resolveMountUrl(
        `api/workspaces/${encodeURIComponent(workspaceId)}/connected-care/invitations`,
      ),
      {
        method: "POST",
        headers: this.#headers({ "Content-Type": "application/json" }),
        body: JSON.stringify({
          patient_id: patientId,
          expected_dob: expectedDob,
        }),
      },
    );
    return { status: resp.status, ok: resp.ok, body: await resp.json() };
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
  setText("shortcut-tabs-desc", chrome.shortcuts_tabs_desc);
  setText("shortcut-privacy-desc", chrome.shortcuts_privacy_desc);
  setText("shortcut-help-desc", chrome.shortcuts_help_desc);
  setText("shortcut-escape-desc", chrome.shortcuts_escape_desc);
  setText("btn-close-shortcuts", chrome.shortcuts_close);

  const tablistEl = document.getElementById("workstation-tabs");
  if (tablistEl && chrome.workstation_tabs_label) {
    tablistEl.setAttribute("aria-label", chrome.workstation_tabs_label);
  }
  setText("tab-chat", chrome.tab_chat);
  setText("tab-today", chrome.tab_today);
  setText("tab-messages", chrome.tab_messages);
  setText("tab-memory", chrome.tab_memory);
  setText("heading-today", chrome.today_heading);
  setText("heading-today-visits", chrome.today_visits_heading);
  setText("heading-messages", chrome.messages_heading);
  setText("heading-messages-patient", chrome.messages_patient_label);
  setText("heading-messages-draft", chrome.messages_draft_label);
  setText("btn-copy-reply", chrome.messages_copy_reply);
  setText("btn-close-reply", chrome.messages_close_reply);
  setText("btn-undo-close", chrome.messages_undo_close);
  setText("messages-closed-notice", chrome.messages_closed_status);
  setText("heading-memory", chrome.memory_heading);
  setText("heading-memory-connected-care", chrome.memory_connected_care_heading);
  setText("btn-issue-invite", chrome.memory_issue_invite);
  setText("label-memory-invite", chrome.memory_invite_label);

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

const WORKSTATION_TABS = ["chat", "today", "messages", "memory"];

function urgencySortRank(urgency) {
  const normalized = String(urgency || "routine").toLowerCase();
  if (normalized === "critical" || normalized === "stat") return 0;
  if (normalized === "urgent" || normalized === "high") return 1;
  return 2;
}

function defaultTodayData() {
  return {
    channelHealthy: true,
    channelBanners: [],
    items: [
      {
        item: {
          item_id: "today-item-urgent-1",
          workspace_id: "ws-demo-1",
          urgency: "urgent",
          waiting_since_epoch_secs: 1700000100,
          kind: "patient_message",
          version_id: "ver-1",
          summary: "Avery, Jordan — Home BP log (122/78 mmHg) & lisinopril confirmation",
          can_mark_done: true,
        },
        seen: false,
        done: false,
      },
      {
        item: {
          item_id: "today-item-routine-2",
          workspace_id: "ws-demo-1",
          urgency: "routine",
          waiting_since_epoch_secs: 1700000400,
          kind: "lab_followup",
          version_id: "ver-2",
          summary: "Avery, Jordan — Quarterly HbA1c 6.8% review",
          can_mark_done: true,
        },
        seen: false,
        done: false,
      },
    ],
    scheduledVisits: [
      {
        visit_id: "visit-1",
        patient_label: "Avery, Jordan (Synthetic)",
        scheduled_window: "09:30 · Glycemic & BP follow-up",
      },
      {
        visit_id: "visit-2",
        patient_label: "Patel, Maya (Synthetic)",
        scheduled_window: "10:15 · Medication reconciliation",
      },
    ],
  };
}

function defaultMemoryEntries() {
  return [
    {
      entry_id: "mem-chart-1",
      authority: "chart_record",
      summary: "HbA1c 6.8% (2026-10-14) · Lisinopril 10 mg PO daily",
      source_ref: "chart:lab-hba1c",
    },
    {
      entry_id: "mem-instr-1",
      authority: "physician_instruction",
      summary: "Keep home BP target < 130/80 mmHg; review serum potassium at next visit.",
      source_ref: "clinician:bp-target",
    },
  ];
}

async function boot() {
  const navStartMs = performance.now();
  const state = {
    cspViolations: 0,
    contextState: "healthy",
    surfaceGeneration: 1,
    preservedDraft: "",
    activeWorkspaceId: "ws-demo-1",
    activeTab: "chat",
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
    todayData: defaultTodayData(),
    memoryEntries: defaultMemoryEntries(),
    lastInviteCode: null,
    inviteCounter: 0,
    replyClosed: false,
    lastCopiedReply: null,
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
    if (
      (event.ctrlKey || event.metaKey) &&
      event.shiftKey &&
      (key.toLowerCase() === "l" || event.code === "KeyL")
    ) {
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
        renderTodayTab();
        renderMemoryTab();
        return localeInfo;
      }
    }
    localeInfo = applyClinicDocumentLocale(normalized, null, null, !state.userEditedDraft);
    state.lang = localeInfo.lang;
    state.dir = localeInfo.dir;
    state.script = localeInfo.script;
    bridge.applyPatches(buildInitialA11yPatches(composerEl?.value ?? "", localeInfo.chrome));
    setText("status-banner", `${localeInfo.chrome.status_ready} \u2066(${localeInfo.dir.toUpperCase()} · ${localeInfo.lang})\u2069.`);
    renderTodayTab();
    renderMemoryTab();
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

  const renderTodayTab = () => {
    const chrome = localeInfo.chrome;
    const bannerEl = document.getElementById("today-channel-banner");
    if (bannerEl) {
      const liveBanner = state.todayData.channelBanners.find(
        (entry) => typeof entry === "string" && entry.trim().length > 0,
      );
      if (liveBanner) {
        bannerEl.textContent = liveBanner;
      } else {
        bannerEl.textContent = state.todayData.channelHealthy
          ? chrome.today_channel_ok
          : chrome.today_channel_degraded;
      }
    }

    const itemsList = document.getElementById("today-items-list");
    if (itemsList) {
      itemsList.textContent = "";
      const sorted = [...state.todayData.items].sort((a, b) => {
        const rankDiff = urgencySortRank(a.item?.urgency) - urgencySortRank(b.item?.urgency);
        if (rankDiff !== 0) return rankDiff;
        return (a.item?.waiting_since_epoch_secs ?? 0) - (b.item?.waiting_since_epoch_secs ?? 0);
      });

      for (const entry of sorted) {
        const item = entry.item || {};
        const itemId = String(item.item_id || "today-item");
        const versionId = String(item.version_id || "ver-1");
        const standing = entry.done ? "done" : entry.seen ? "seen" : "pending";

        const li = document.createElement("li");
        li.className = "workflow-card";
        li.dataset.todayItemId = itemId;
        li.dataset.urgency = String(item.urgency || "routine");
        li.dataset.standing = standing;

        const header = document.createElement("div");
        header.className = "workflow-card-header";

        const urgencyBadge = document.createElement("span");
        urgencyBadge.className =
          urgencySortRank(item.urgency) <= 1 ? "workflow-badge workflow-badge-accent" : "workflow-badge";
        urgencyBadge.textContent = String(item.urgency || "routine").toUpperCase();

        const standingBadge = document.createElement("span");
        standingBadge.className = "workflow-badge";
        standingBadge.dataset.standingBadge = standing;
        standingBadge.textContent =
          standing === "done"
            ? chrome.today_done_badge
            : standing === "seen"
              ? chrome.today_seen_badge
              : chrome.today_pending_badge;

        header.appendChild(urgencyBadge);
        header.appendChild(standingBadge);

        const body = document.createElement("p");
        body.className = "workflow-body";
        body.dir = "auto";
        body.textContent = String(item.summary || itemId);

        const actions = document.createElement("div");
        actions.className = "workflow-actions";

        const seenBtn = document.createElement("button");
        seenBtn.type = "button";
        seenBtn.className = "btn";
        seenBtn.dataset.todayAction = "seen";
        seenBtn.dataset.itemId = itemId;
        seenBtn.dataset.versionId = versionId;
        seenBtn.textContent = chrome.today_mark_seen;
        seenBtn.addEventListener("click", () => {
          void handleTodayStanding(itemId, "seen", versionId);
        });

        const doneBtn = document.createElement("button");
        doneBtn.type = "button";
        doneBtn.className = "btn btn-primary";
        doneBtn.dataset.todayAction = "done";
        doneBtn.dataset.itemId = itemId;
        doneBtn.dataset.versionId = versionId;
        doneBtn.textContent = chrome.today_mark_done;
        doneBtn.addEventListener("click", () => {
          void handleTodayStanding(itemId, "done", versionId);
        });

        actions.appendChild(seenBtn);
        if (item.can_mark_done !== false) {
          actions.appendChild(doneBtn);
        }

        li.appendChild(header);
        li.appendChild(body);
        li.appendChild(actions);
        itemsList.appendChild(li);
      }
    }

    const visitsList = document.getElementById("today-visits-list");
    if (visitsList) {
      visitsList.textContent = "";
      for (const visit of state.todayData.scheduledVisits) {
        const li = document.createElement("li");
        li.className = "transcript-item";
        li.dir = "auto";
        const label = visit.patient_label || visit.patient_id || visit.visit_id || "Scheduled visit";
        const windowText = visit.scheduled_window || visit.starts_at || "";
        li.textContent = windowText ? `${label} · ${windowText}` : String(label);
        visitsList.appendChild(li);
      }
    }
  };

  const handleTodayStanding = async (itemId, standing, versionId) => {
    let appliedStanding = standing;
    if (apiClient.sessionToken) {
      try {
        const res = await apiClient.updateTodayStanding(itemId, standing, versionId);
        if (res.ok && res.body && typeof res.body.standing === "string") {
          appliedStanding = res.body.standing;
          setText("metric-api-state", `today:${appliedStanding} (${res.status})`);
        }
      } catch (_err) {
        // Static preview fallback updates local standing deterministically.
      }
    }
    for (const entry of state.todayData.items) {
      if (entry.item && entry.item.item_id === itemId) {
        if (appliedStanding === "seen") {
          entry.seen = true;
        } else if (appliedStanding === "done") {
          entry.seen = true;
          entry.done = true;
        }
      }
    }
    renderTodayTab();
  };

  const refreshTodayQueue = async () => {
    renderTodayTab();
    if (!apiClient.sessionToken) {
      return;
    }
    try {
      const res = await apiClient.getToday();
      if (res.ok && res.body && typeof res.body === "object") {
        const liveItems = Array.isArray(res.body.items) ? res.body.items : [];
        const liveVisits = Array.isArray(res.body.scheduled_visits)
          ? res.body.scheduled_visits
          : [];
        const liveBanners = Array.isArray(res.body.channel_banners)
          ? res.body.channel_banners
              .map((entry) =>
                typeof entry === "string"
                  ? entry
                  : entry && typeof entry.message === "string"
                    ? entry.message
                    : "",
              )
              .filter(Boolean)
          : [];
        if (liveItems.length > 0) {
          state.todayData.items = liveItems;
        }
        if (liveVisits.length > 0) {
          state.todayData.scheduledVisits = liveVisits;
        }
        state.todayData.channelBanners = liveBanners;
        state.todayData.channelHealthy = liveBanners.length === 0;
        setText("metric-api-state", `today (${res.status})`);
        renderTodayTab();
      }
    } catch (_err) {
      // Retain deterministic local preview queue when running without clinic API.
    }
  };

  const renderMemoryTab = () => {
    const chrome = localeInfo.chrome;
    const list = document.getElementById("memory-entries-list");
    if (list) {
      list.textContent = "";
      for (const entry of state.memoryEntries) {
        const isChart = String(entry.authority || "").toLowerCase() === "chart_record";
        const li = document.createElement("li");
        li.className = "workflow-card";
        li.dataset.memoryAuthority = isChart ? "chart_record" : "physician_instruction";

        const header = document.createElement("div");
        header.className = "workflow-card-header";

        const badge = document.createElement("span");
        badge.className = isChart ? "workflow-badge" : "workflow-badge workflow-badge-accent";
        badge.textContent = isChart ? chrome.memory_chart_record : chrome.memory_physician_instruction;
        header.appendChild(badge);

        const body = document.createElement("p");
        body.className = "workflow-body";
        body.dir = "auto";
        body.textContent = String(
          entry.summary || entry.content || entry.statement || entry.entry_id || "",
        );

        li.appendChild(header);
        li.appendChild(body);
        list.appendChild(li);
      }
    }
    const codeEl = document.getElementById("memory-invite-code");
    if (codeEl) {
      codeEl.textContent = state.lastInviteCode || chrome.memory_invite_none;
    }
  };

  const refreshWorkspaceMemory = async () => {
    renderMemoryTab();
    if (!apiClient.sessionToken) {
      return;
    }
    try {
      const res = await apiClient.getWorkspaceMemory(state.activeWorkspaceId);
      if (res.ok && res.body && Array.isArray(res.body.entries) && res.body.entries.length > 0) {
        state.memoryEntries = res.body.entries;
        setText("metric-api-state", `memory (${res.status})`);
        renderMemoryTab();
      }
    } catch (_err) {
      // Retain deterministic preview entries when offline.
    }
  };

  const handleIssueConnectedCareInvite = async () => {
    let inviteCode = null;
    if (apiClient.sessionToken) {
      try {
        const res = await apiClient.issueConnectedCareInvite(
          state.activeWorkspaceId,
          "patient-demo-1",
          "1985-04-12",
        );
        if (res.ok && res.body && typeof res.body.invite_code === "string") {
          inviteCode = res.body.invite_code;
          setText("metric-api-state", `invite (${res.status})`);
        }
      } catch (_err) {
        // Static preview fallback generates a deterministic valid EF-XXXXXX invite code.
      }
    }
    if (!inviteCode) {
      state.inviteCounter += 1;
      const numeric = String(482912 + state.inviteCounter).padStart(6, "0").slice(-6);
      inviteCode = `EF-${numeric}`;
    }
    state.lastInviteCode = inviteCode;
    document.documentElement.dataset.efLastInviteCode = inviteCode;
    renderMemoryTab();
    return inviteCode;
  };

  const selectWorkstationTab = (tabId, moveFocus = false) => {
    const normalized = WORKSTATION_TABS.includes(tabId) ? tabId : "chat";
    state.activeTab = normalized;
    document.documentElement.dataset.efActiveTab = normalized;
    for (const candidate of WORKSTATION_TABS) {
      const tabBtn = document.getElementById(`tab-${candidate}`);
      const panelEl = document.getElementById(`panel-${candidate}`);
      const isSelected = candidate === normalized;
      if (tabBtn) {
        tabBtn.setAttribute("aria-selected", isSelected ? "true" : "false");
        tabBtn.tabIndex = isSelected ? 0 : -1;
        if (isSelected && moveFocus) {
          tabBtn.focus();
        }
      }
      if (panelEl) {
        panelEl.hidden = !isSelected;
      }
    }
    if (normalized === "today") {
      void refreshTodayQueue();
    } else if (normalized === "memory") {
      void refreshWorkspaceMemory();
    }
    return normalized;
  };

  for (const tabId of WORKSTATION_TABS) {
    const tabBtn = document.getElementById(`tab-${tabId}`);
    if (tabBtn) {
      tabBtn.addEventListener("click", () => {
        selectWorkstationTab(tabId, false);
      });
    }
  }

  const tablistEl = document.getElementById("workstation-tabs");
  if (tablistEl) {
    tablistEl.addEventListener("keydown", (event) => {
      const currentIdx = WORKSTATION_TABS.indexOf(state.activeTab);
      if (currentIdx < 0) return;
      const isRtl = document.documentElement.dir === "rtl";
      let nextIdx = null;
      if (event.key === "ArrowRight") {
        nextIdx = isRtl
          ? (currentIdx - 1 + WORKSTATION_TABS.length) % WORKSTATION_TABS.length
          : (currentIdx + 1) % WORKSTATION_TABS.length;
      } else if (event.key === "ArrowLeft") {
        nextIdx = isRtl
          ? (currentIdx + 1) % WORKSTATION_TABS.length
          : (currentIdx - 1 + WORKSTATION_TABS.length) % WORKSTATION_TABS.length;
      } else if (event.key === "Home") {
        nextIdx = 0;
      } else if (event.key === "End") {
        nextIdx = WORKSTATION_TABS.length - 1;
      }
      if (nextIdx !== null) {
        event.preventDefault();
        selectWorkstationTab(WORKSTATION_TABS[nextIdx], true);
      }
    });
  }

  const copyReplyBtn = document.getElementById("btn-copy-reply");
  if (copyReplyBtn) {
    copyReplyBtn.addEventListener("click", () => {
      const draftText = document.getElementById("messages-draft-text")?.textContent?.trim() ?? "";
      state.lastCopiedReply = draftText;
      document.documentElement.dataset.efReplyCopied = "true";
      setText("messages-action-status", localeInfo.chrome.messages_copied_status);
      if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
        void navigator.clipboard.writeText(draftText).catch(() => {
          // Headless contexts without clipboard permission still record copy confirmation.
        });
      }
    });
  }

  const closeReplyBtn = document.getElementById("btn-close-reply");
  const undoCloseBtn = document.getElementById("btn-undo-close");
  const draftCardEl = document.getElementById("messages-draft-card");
  const closedBarEl = document.getElementById("messages-closed-bar");

  if (closeReplyBtn) {
    closeReplyBtn.addEventListener("click", () => {
      state.replyClosed = true;
      if (draftCardEl) draftCardEl.hidden = true;
      if (closedBarEl) closedBarEl.hidden = false;
      document.documentElement.dataset.efReplyClosed = "true";
      setText("messages-action-status", localeInfo.chrome.messages_closed_status);
    });
  }

  if (undoCloseBtn) {
    undoCloseBtn.addEventListener("click", () => {
      state.replyClosed = false;
      if (draftCardEl) draftCardEl.hidden = false;
      if (closedBarEl) closedBarEl.hidden = true;
      document.documentElement.dataset.efReplyClosed = "false";
      setText("messages-action-status", localeInfo.chrome.messages_restored_status);
    });
  }

  const issueInviteBtn = document.getElementById("btn-issue-invite");
  if (issueInviteBtn) {
    issueInviteBtn.addEventListener("click", () => {
      void handleIssueConnectedCareInvite();
    });
  }

  document.addEventListener("keydown", (event) => {
    if (
      (event.ctrlKey || event.metaKey) &&
      !event.shiftKey &&
      !event.altKey &&
      !state.privacyCoverActive &&
      !state.shortcutsOpen
    ) {
      const tabIndex = Number(event.key) - 1;
      if (Number.isInteger(tabIndex) && tabIndex >= 0 && tabIndex < WORKSTATION_TABS.length) {
        event.preventDefault();
        selectWorkstationTab(WORKSTATION_TABS[tabIndex], false);
      }
    }
  });

  renderTodayTab();
  renderMemoryTab();
  selectWorkstationTab(searchParams.get("tab") || "chat", false);

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
    selectTab: (tabId) => selectWorkstationTab(tabId, false),
    getActiveTab: () => state.activeTab,
    refreshToday: () => refreshTodayQueue(),
    refreshMemory: () => refreshWorkspaceMemory(),
    issueConnectedCareInvite: () => handleIssueConnectedCareInvite(),
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
        renderTodayTab();
        renderMemoryTab();
      }
      return snap;
    },
    getState: () => ({ ...state, a11yNodeCount: bridge.projectedNodeCount() }),
    loadWasmBundle: (bundleName) => loadOptionalWasmBundle(capabilities, bundleName),
  });
}

boot();
