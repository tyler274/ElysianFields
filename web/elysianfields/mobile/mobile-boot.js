/**
 * Strict-CSP, zero-secret Curie Mobile PWA shell mounted under `/elysianfields/mobile/`.
 *
 * Responsibilities:
 * - Touch-friendly layout with zero horizontal overflow on 390x844 / 412x915 viewports.
 * - Single-owner ARIA landmark/control projection via `../a11y-bridge.js`.
 * - Full 12-locale / 7-script localization (`en`, `he`, `ar`, `zh-Hans`, `ja`, `vi`, `hi`)
 *   with live RTL switching for Hebrew (`he`) and Arabic (`ar`).
 * - Live `/elysianfields/mobile-api/*` client (sign-in, offline owed sign-out queue,
 *   Connected Care DOB verify/share, For my doctor curated send).
 * - Offline Service Worker registration (`./sw.js`) preserving crisis lines (`988`, `911`, `SAMHSA`).
 */

import { ElysianNarrowA11yBridge } from "../a11y-bridge.js";
import { getLocaleMeta, mergeChrome, normalizeLocaleTag } from "../locales.js";
import {
  isWasmModuleAdvertised,
  loadBevyWasmClient,
  syncRustA11yNodes,
} from "../wasm-bridge.js";

const MOBILE_MOUNT_PREFIX = "/elysianfields/mobile";
const MOBILE_API_PREFIX = "/elysianfields/mobile-api";
const THEME_STORAGE_KEY = "curie.theme.v1";

function resolveInitialTheme(params) {
  const fromQuery = params.get("theme");
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

function applyMobileTheme(theme, chrome, persist = false) {
  const normalized = theme === "light" ? "light" : "dark";
  document.documentElement.dataset.theme = normalized;
  document.documentElement.dataset.efTheme = normalized;
  const metaTheme = document.querySelector('meta[name="theme-color"]');
  if (metaTheme) {
    metaTheme.setAttribute("content", normalized === "light" ? "#f8fafc" : "#0f172a");
  }
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

export function isWithinMobileMount(pathname) {
  return pathname === MOBILE_MOUNT_PREFIX || pathname.startsWith(`${MOBILE_MOUNT_PREFIX}/`);
}

function resolveMobileApiUrl(relativePath) {
  const cleaned = String(relativePath).replace(/^\/+/, "");
  return `${window.location.origin}${MOBILE_API_PREFIX}/${cleaned}`;
}

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = String(value);
  }
}

export class MobileApiClient {
  constructor() {
    this.sessionToken = null;
    this.owedSignOuts = [];
  }

  #headers(extra = {}) {
    const headers = { ...extra };
    if (this.sessionToken) {
      headers.Authorization = `Bearer ${this.sessionToken}`;
    }
    return headers;
  }

  async requestSignInLink(email) {
    const resp = await fetch(resolveMobileApiUrl("auth/request-link"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify({ email }),
    });
    return { status: resp.status, body: await resp.json() };
  }

  async verifySignIn(email, token) {
    const resp = await fetch(resolveMobileApiUrl("auth/verify"), {
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

  async signOutOrQueueOwed() {
    const tokenToEnd = this.sessionToken;
    // SignInHoldsNothing: clear active local session immediately even if offline.
    this.sessionToken = null;
    if (!tokenToEnd) {
      return { queued: false, flushed: true };
    }
    try {
      const resp = await fetch(resolveMobileApiUrl("auth/sign-out"), {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${tokenToEnd}`,
        },
        body: JSON.stringify({ session: tokenToEnd }),
      });
      if (resp.ok) {
        return { queued: false, flushed: true };
      }
    } catch (_offlineErr) {
      // Fall through to durable offline owed queue.
    }
    this.owedSignOuts.push(tokenToEnd);
    return { queued: true, flushed: false };
  }

  async flushOwedSignOuts() {
    const remaining = [];
    for (const token of this.owedSignOuts) {
      try {
        const resp = await fetch(resolveMobileApiUrl("auth/sign-out"), {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Authorization: `Bearer ${token}`,
          },
          body: JSON.stringify({ session: token }),
        });
        if (!resp.ok) {
          remaining.push(token);
        }
      } catch (_err) {
        remaining.push(token);
      }
    }
    this.owedSignOuts = remaining;
    return remaining.length;
  }

  async verifyConnectedCareDob(invitationId, dobIso) {
    const resp = await fetch(resolveMobileApiUrl("connected-care/verify-dob"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify({ invitation_id: invitationId, dob: dobIso }),
    });
    return { status: resp.status, body: await resp.json() };
  }

  async confirmConnectedCareShare(invitationId) {
    const resp = await fetch(resolveMobileApiUrl("connected-care/share"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify({ invitation_id: invitationId }),
    });
    return { status: resp.status, body: await resp.json() };
  }

  async sendForMyDoctor(payload) {
    const resp = await fetch(resolveMobileApiUrl("connected-care/transfer"), {
      method: "POST",
      headers: this.#headers({ "Content-Type": "application/json" }),
      body: JSON.stringify(payload),
    });
    return { status: resp.status, body: await resp.json() };
  }
}

function applyLocaleAndDirection(lang, explicitDirection = null, wasmSnapshot = null) {
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

  setText("mobile-title", chrome.mobile_brand);
  setText("crisis-heading", chrome.crisis_heading);
  setText("crisis-988-label", chrome.crisis_988_label);
  setText("crisis-988-prefix", chrome.crisis_988_prefix);
  setText("crisis-988-suffix", chrome.crisis_988_suffix);
  setText("crisis-911-label", chrome.crisis_911_label);
  setText("crisis-911-prefix", chrome.crisis_911_prefix);
  setText("crisis-911-suffix", chrome.crisis_911_suffix);
  setText("crisis-samhsa-label", chrome.crisis_samhsa_label);
  setText("crisis-samhsa-prefix", chrome.crisis_samhsa_prefix);
  setText("crisis-samhsa-suffix", chrome.crisis_samhsa_suffix);
  setText("btn-toggle-lang", dir === "rtl" ? "English (LTR)" : "العربية (RTL)");
  setText("nav-account", chrome.nav_account);
  setText("nav-connected-care", chrome.nav_connected_care);
  setText("nav-for-my-doctor", chrome.nav_for_my_doctor);
  setText("heading-account", chrome.account_heading);
  setText("label-mobile-email", chrome.patient_email);
  setText("btn-mobile-signin", chrome.sign_in);
  setText("btn-mobile-signout", chrome.sign_out);
  setText("heading-connected-care", chrome.connected_care_heading);
  setText("label-invite-code", chrome.invite_code_label);
  setText("label-verify-dob", chrome.verify_dob_label);
  setText("btn-verify-dob", chrome.verify_dob);
  setText("btn-share-consent", chrome.share);
  setText("heading-for-my-doctor", chrome.for_my_doctor_heading);
  setText("label-doctor-note", chrome.doctor_note_label);
  setText("btn-send-doctor", chrome.review_and_send);
  setText("heading-mobile-diagnostics", chrome.mobile_diagnostics);
  setText("mobile-metric-dir", `${dir} / ${normalizedLang} (${meta.script})`);

  const activeTheme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
  const themeBtn = document.getElementById("btn-toggle-theme");
  if (themeBtn) {
    themeBtn.textContent = activeTheme === "light" ? chrome.theme_to_dark : chrome.theme_to_light;
  }

  const noteEl = document.getElementById("doctor-note");
  if (noteEl) {
    noteEl.placeholder = chrome.doctor_note_placeholder;
  }

  return { lang: normalizedLang, dir, script: meta.script, chrome };
}

function buildMobileA11yPatches(noteValue, chrome) {
  return [
    {
      kind: "UpsertElement",
      element: {
        nodeId: "300",
        controlKey: "nav-mobile-sections",
        role: "navigation",
        name: `${chrome.mobile_brand} — ${chrome.nav_account}`,
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "301",
        controlKey: "region-crisis-support",
        role: "region",
        name: chrome.crisis_heading,
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "302",
        controlKey: "main-mobile-workspace",
        role: "main",
        name: chrome.mobile_brand,
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "303",
        controlKey: "button-connected-care-share",
        role: "button",
        name: chrome.share,
        tabOrder: "Sequential",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "304",
        controlKey: "textbox-doctor-note",
        role: "textbox",
        name: chrome.doctor_note_label,
        value: noteValue,
        tabOrder: "Sequential",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "305",
        controlKey: "button-offline-signout",
        role: "button",
        name: chrome.sign_out,
        tabOrder: "Sequential",
      },
    },
    {
      kind: "SetPoliteLive",
      text: `${chrome.mobile_status_ready} (${MOBILE_MOUNT_PREFIX}/).`,
    },
  ];
}

async function registerOfflineServiceWorker() {
  if (!("serviceWorker" in navigator) || typeof navigator.serviceWorker.register !== "function") {
    setText("mobile-metric-sw", "unsupported");
    document.documentElement.dataset.efSwState = "unsupported";
    return "unsupported";
  }
  try {
    const reg = await navigator.serviceWorker.register("./sw.js", { scope: "./" });
    await navigator.serviceWorker.ready;
    const state = navigator.serviceWorker.controller ? "controlling" : reg.active ? "active" : "registered";
    setText("mobile-metric-sw", state);
    document.documentElement.dataset.efSwState = state;
    return state;
  } catch (_err) {
    setText("mobile-metric-sw", "unavailable");
    document.documentElement.dataset.efSwState = "unavailable";
    return "unavailable";
  }
}

async function bootMobile() {
  const navStartMs = performance.now();
  const state = {
    cspViolations: 0,
    lang: "en",
    dir: "ltr",
    script: "Latn",
    theme: "dark",
    lastAction: null,
    wasmRuntime: "static-shell",
    wasmAbiVersion: null,
    lastWasmSnapshot: null,
  };

  document.addEventListener("securitypolicyviolation", () => {
    state.cspViolations += 1;
    setText("mobile-metric-csp", state.cspViolations);
    document.documentElement.dataset.efCspViolations = String(state.cspViolations);
  });

  const params = new URLSearchParams(window.location.search);
  state.theme = applyMobileTheme(resolveInitialTheme(params), null, false);
  const initialLang = params.get("lang") || (params.get("dir") === "rtl" ? "ar" : "en");
  let localeInfo = applyLocaleAndDirection(initialLang);
  state.lang = localeInfo.lang;
  state.dir = localeInfo.dir;
  state.script = localeInfo.script;
  state.theme = applyMobileTheme(state.theme, localeInfo.chrome, false);

  const pathname = window.location.pathname;
  const withinMount = isWithinMobileMount(pathname);
  setText(
    "mobile-metric-subpath",
    withinMount ? `${pathname} (valid)` : `${pathname} (invalid-mount)`,
  );

  const appRoot = document.getElementById("mobile-app") ?? document.body;
  const noteEl = document.getElementById("doctor-note");
  let wasmClient = null;
  let a11yOwner = "shell";
  const bridge = new ElysianNarrowA11yBridge(
    appRoot,
    (action) => {
      state.lastAction = action;
      if (action.type === "set_value" && noteEl && typeof action.value === "string") {
        noteEl.value = action.value;
        if (wasmClient) {
          const updated = wasmClient.dispatch({ cmd: "edit_draft", text: action.value });
          state.lastWasmSnapshot = updated;
          syncRustA11yNodes(bridge, updated);
          setText("mobile-metric-a11y", bridge.projectedNodeCount());
        }
      }
    },
    { mountSubpath: MOBILE_MOUNT_PREFIX },
  );

  const shouldLoadMobileWasm =
    params.get("wasm") !== "0" &&
    (params.get("wasm") === "1" ||
      (await isWasmModuleAdvertised("./wasm-manifest.json", "mobileWasm")));

  if (shouldLoadMobileWasm) {
    try {
      wasmClient = await loadBevyWasmClient({
        wasmUrl: "./ef_mobile_client.wasm",
        initFn: "ef_mobile_wasm_init",
        versionFn: "ef_mobile_wasm_version",
        dispatchFn: "ef_mobile_wasm_dispatch_json",
      });
      let snapshot = wasmClient.initialSnapshot;
      const normalizedInit = normalizeLocaleTag(initialLang);
      if (normalizedInit !== "en") {
        const localized = wasmClient.dispatch({ cmd: "set_locale", lang: normalizedInit });
        if (localized && localized.ok) {
          snapshot = localized;
        }
      }
      state.wasmRuntime = snapshot.runtime || "bevy-rust-wasm";
      state.wasmAbiVersion = wasmClient.abiVersion;
      state.lastWasmSnapshot = snapshot;
      a11yOwner = "rust-accesskit";
      localeInfo = applyLocaleAndDirection(
        snapshot.locale || normalizedInit,
        snapshot.direction,
        snapshot,
      );
      state.lang = localeInfo.lang;
      state.dir = localeInfo.dir;
      state.script = localeInfo.script;
      syncRustA11yNodes(
        bridge,
        snapshot,
        "Curie Mobile Rust AccessKit tree mounted under /elysianfields/mobile/.",
      );
    } catch (_err) {
      wasmClient = null;
      bridge.applyPatches(buildMobileA11yPatches(noteEl?.value ?? "", localeInfo.chrome));
    }
  } else {
    bridge.applyPatches(buildMobileA11yPatches(noteEl?.value ?? "", localeInfo.chrome));
  }

  const nodeCount = bridge.projectedNodeCount();
  setText("mobile-metric-a11y", nodeCount);

  const apiClient = new MobileApiClient();

  const applyMobileLocale = (nextLang) => {
    const normalized = normalizeLocaleTag(nextLang);
    if (wasmClient) {
      const snap = wasmClient.dispatch({ cmd: "set_locale", lang: normalized });
      if (snap && snap.ok) {
        state.lastWasmSnapshot = snap;
        localeInfo = applyLocaleAndDirection(snap.locale || normalized, snap.direction, snap);
        state.lang = localeInfo.lang;
        state.dir = localeInfo.dir;
        state.script = localeInfo.script;
        const count = syncRustA11yNodes(
          bridge,
          snap,
          `${localeInfo.chrome.mobile_status_ready} (${localeInfo.dir.toUpperCase()} · ${localeInfo.lang}).`,
        );
        setText("mobile-metric-a11y", count);
        document.documentElement.dataset.efA11yNodes = String(count);
        setText("mobile-status", `${localeInfo.chrome.mobile_status_ready} \u2066(${localeInfo.dir.toUpperCase()} · ${localeInfo.lang})\u2069.`);
        return localeInfo;
      }
    }
    localeInfo = applyLocaleAndDirection(normalized);
    state.lang = localeInfo.lang;
    state.dir = localeInfo.dir;
    state.script = localeInfo.script;
    bridge.applyPatches(buildMobileA11yPatches(noteEl?.value ?? "", localeInfo.chrome));
    setText("mobile-status", `${localeInfo.chrome.mobile_status_ready} \u2066(${localeInfo.dir.toUpperCase()} · ${localeInfo.lang})\u2069.`);
    return localeInfo;
  };

  const localeSelect = document.getElementById("locale-select");
  if (localeSelect) {
    localeSelect.addEventListener("change", (event) => {
      applyMobileLocale(event.target.value);
    });
  }

  const toggleBtn = document.getElementById("btn-toggle-lang");
  if (toggleBtn) {
    toggleBtn.addEventListener("click", () => {
      const nextLang = state.dir === "rtl" ? "en" : "ar";
      applyMobileLocale(nextLang);
    });
  }

  const themeToggleBtn = document.getElementById("btn-toggle-theme");
  if (themeToggleBtn) {
    themeToggleBtn.addEventListener("click", () => {
      const nextTheme = state.theme === "light" ? "dark" : "light";
      state.theme = applyMobileTheme(nextTheme, localeInfo.chrome, true);
    });
  }

  const signOutBtn = document.getElementById("btn-mobile-signout");
  if (signOutBtn) {
    signOutBtn.addEventListener("click", async () => {
      if (noteEl) noteEl.value = "";
      if (wasmClient) {
        const snap = wasmClient.dispatch({ cmd: "edit_draft", text: "" });
        state.lastWasmSnapshot = snap;
        syncRustA11yNodes(bridge, snap);
      }
      await apiClient.signOutOrQueueOwed();
      setText("mobile-metric-owed", apiClient.owedSignOuts.length);
      setText(
        "mobile-status",
        apiClient.owedSignOuts.length > 0
          ? "Signed out locally; owed sign-out queued for reconnect."
          : "Signed out cleanly.",
      );
    });
  }

  window.addEventListener("online", async () => {
    if (wasmClient) {
      state.lastWasmSnapshot = wasmClient.dispatch({ cmd: "set_connectivity", online: true });
    }
    const remaining = await apiClient.flushOwedSignOuts();
    setText("mobile-metric-owed", remaining);
  });

  const authForm = document.getElementById("mobile-auth-form");
  if (authForm) {
    authForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const email = document.getElementById("mobile-email")?.value?.trim() ?? "";
      if (!email) return;
      try {
        const res = await apiClient.requestSignInLink(email);
        setText("mobile-status", `Sign-in link requested (HTTP ${res.status}).`);
      } catch (_err) {
        setText("mobile-status", "Offline preview: sign-in requires network connectivity.");
      }
    });
  }

  const ccForm = document.getElementById("connected-care-form");
  if (ccForm) {
    ccForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const code = document.getElementById("invite-code")?.value?.trim() ?? "";
      const dob = document.getElementById("verify-dob")?.value?.trim() ?? "";
      if (!code || !dob) return;
      try {
        const res = await apiClient.verifyConnectedCareDob(code, dob);
        setText("mobile-status", `Connected Care DOB check (HTTP ${res.status}).`);
      } catch (_err) {
        setText("mobile-status", "Offline preview: DOB verification requires connectivity.");
      }
    });
  }

  const doctorNoteForm = document.getElementById("doctor-note-form");
  if (doctorNoteForm) {
    doctorNoteForm.addEventListener("submit", (event) => {
      event.preventDefault();
      const text = noteEl?.value?.trim() ?? "";
      if (!text) return;
      const list = document.getElementById("doctor-sent-list");
      if (list) {
        const li = document.createElement("li");
        li.className = "sent-item";
        li.dir = "auto";
        li.textContent = text;
        list.appendChild(li);
      }
      if (wasmClient) {
        const snap = wasmClient.dispatch({ cmd: "edit_draft", text });
        state.lastWasmSnapshot = snap;
        syncRustA11yNodes(bridge, snap);
      }
      setText("mobile-status", `${localeInfo.chrome.for_my_doctor_heading}: ${text}`);
    });
  }

  const coldStartMs = performance.now() - navStartMs;

  setText(
    "mobile-status",
    `${localeInfo.chrome.mobile_status_ready} \u2066(${state.dir.toUpperCase()} · ${state.lang} · ${nodeCount} ARIA · ${state.cspViolations} CSP)\u2069.`,
  );

  document.documentElement.dataset.efMobileReady = "true";
  document.documentElement.dataset.efReady = "true";
  document.documentElement.dataset.efSubpathValid = String(withinMount);
  document.documentElement.dataset.efA11yNodes = String(nodeCount);
  document.documentElement.dataset.efA11yOwner = a11yOwner;
  document.documentElement.dataset.efWasmRuntime = state.wasmRuntime;
  if (state.wasmAbiVersion !== null) {
    document.documentElement.dataset.efWasmAbiVersion = String(state.wasmAbiVersion);
  }
  document.documentElement.dataset.efCspViolations = String(state.cspViolations);
  document.documentElement.dataset.efColdStartMs = coldStartMs.toFixed(2);
  document.documentElement.dataset.efSwState = "registering";

  window.__elysianMobileShell = Object.freeze({
    mountPrefix: MOBILE_MOUNT_PREFIX,
    apiClient,
    bridge,
    dispatchWasm: (cmd) => {
      if (!wasmClient) return null;
      const snap = wasmClient.dispatch(cmd);
      state.lastWasmSnapshot = snap;
      if (snap && snap.ok) {
        localeInfo = applyLocaleAndDirection(snap.locale, snap.direction, snap);
        state.lang = localeInfo.lang;
        state.dir = localeInfo.dir;
        state.script = localeInfo.script;
        const count = syncRustA11yNodes(bridge, snap);
        setText("mobile-metric-a11y", count);
        document.documentElement.dataset.efA11yNodes = String(count);
      }
      return snap;
    },
    setLocale: (lang) => applyMobileLocale(lang),
    setTheme: (theme) => {
      state.theme = applyMobileTheme(theme, localeInfo.chrome, true);
      return state.theme;
    },
    getState: () => ({ ...state, a11yNodeCount: bridge.projectedNodeCount() }),
  });

  // Register offline Service Worker asynchronously so initial shell readiness never stalls.
  void registerOfflineServiceWorker();
}

bootMobile();
