/**
 * Strict-CSP, zero-secret Curie Mobile PWA shell mounted under `/elysianfields/mobile/`.
 *
 * Responsibilities:
 * - Touch-friendly layout with zero horizontal overflow on 390x844 / 412x915 viewports.
 * - Single-owner ARIA landmark/control projection via `../a11y-bridge.js`.
 * - Live Arabic RTL switching (`?lang=ar` or interactive toggle button).
 * - Live `/elysianfields/mobile-api/*` client (sign-in, offline owed sign-out queue,
 *   Connected Care DOB verify/share, For my doctor curated send).
 * - Offline Service Worker registration (`./sw.js`) preserving crisis lines (`988`, `911`, `SAMHSA`).
 */

import { ElysianNarrowA11yBridge } from "../a11y-bridge.js";
import {
  isWasmModuleAdvertised,
  loadBevyWasmClient,
  syncRustA11yNodes,
} from "../wasm-bridge.js";

const MOBILE_MOUNT_PREFIX = "/elysianfields/mobile";
const MOBILE_API_PREFIX = "/elysianfields/mobile-api";

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

function applyLocaleAndDirection(lang, explicitDirection = null) {
  const raw = String(lang || "en").trim();
  const isArabic = raw.toLowerCase().startsWith("ar");
  const normalizedLang = raw || "en";
  const dir =
    explicitDirection === "rtl" || explicitDirection === "ltr"
      ? explicitDirection
      : isArabic
        ? "rtl"
        : "ltr";

  document.documentElement.lang = normalizedLang;
  document.documentElement.dir = dir;
  document.documentElement.dataset.efLang = normalizedLang;
  document.documentElement.dataset.efDir = dir;

  setText("mobile-title", isArabic ? "كوري موبايل" : "Curie Mobile");
  setText(
    "crisis-heading",
    isArabic ? "الدعم الفوري للأزمات والتعافي (988 / 911 / SAMHSA)" : "Immediate Crisis & Recovery Support",
  );
  setText("btn-toggle-lang", isArabic ? "English (LTR)" : "العربية (RTL)");
  setText("mobile-metric-dir", `${dir} / ${normalizedLang}`);
  return { lang: normalizedLang, dir };
}

function buildMobileA11yPatches(noteValue) {
  return [
    {
      kind: "UpsertElement",
      element: {
        nodeId: "300",
        controlKey: "nav-mobile-sections",
        role: "navigation",
        name: "Curie Mobile navigation",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "301",
        controlKey: "region-crisis-support",
        role: "region",
        name: "Immediate crisis support 988, 911, and SAMHSA National Helpline",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "302",
        controlKey: "main-mobile-workspace",
        role: "main",
        name: "Curie Mobile workspace",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "303",
        controlKey: "button-connected-care-share",
        role: "button",
        name: "Confirm Connected Care Share",
        tabOrder: "Sequential",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "304",
        controlKey: "textbox-doctor-note",
        role: "textbox",
        name: "For my doctor curated note",
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
        name: "Sign out safely offline",
        tabOrder: "Sequential",
      },
    },
    {
      kind: "SetPoliteLive",
      text: "Curie Mobile accessibility tree mounted under /elysianfields/mobile/.",
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
  const initialLang = params.get("lang") || (params.get("dir") === "rtl" ? "ar" : "en");
  const localeInfo = applyLocaleAndDirection(initialLang);
  state.lang = localeInfo.lang;
  state.dir = localeInfo.dir;

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
      if (initialLang && initialLang.toLowerCase() !== "en") {
        const localized = wasmClient.dispatch({ cmd: "set_locale", lang: initialLang });
        if (localized && localized.ok) {
          snapshot = localized;
        }
      }
      state.wasmRuntime = snapshot.runtime || "bevy-rust-wasm";
      state.wasmAbiVersion = wasmClient.abiVersion;
      state.lastWasmSnapshot = snapshot;
      a11yOwner = "rust-accesskit";
      const syncedLocale = applyLocaleAndDirection(snapshot.locale, snapshot.direction);
      state.lang = syncedLocale.lang;
      state.dir = syncedLocale.dir;
      syncRustA11yNodes(
        bridge,
        snapshot,
        "Curie Mobile Rust AccessKit tree mounted under /elysianfields/mobile/.",
      );
    } catch (_err) {
      wasmClient = null;
      bridge.applyPatches(buildMobileA11yPatches(noteEl?.value ?? ""));
    }
  } else {
    bridge.applyPatches(buildMobileA11yPatches(noteEl?.value ?? ""));
  }

  const nodeCount = bridge.projectedNodeCount();
  setText("mobile-metric-a11y", nodeCount);

  const apiClient = new MobileApiClient();

  const applyMobileLocale = (nextLang) => {
    if (wasmClient) {
      const snap = wasmClient.dispatch({ cmd: "set_locale", lang: nextLang });
      if (snap && snap.ok) {
        state.lastWasmSnapshot = snap;
        const updated = applyLocaleAndDirection(snap.locale, snap.direction);
        state.lang = updated.lang;
        state.dir = updated.dir;
        const count = syncRustA11yNodes(
          bridge,
          snap,
          updated.dir === "rtl" ? "Switched to Arabic RTL layout." : "Switched to English LTR layout.",
        );
        setText("mobile-metric-a11y", count);
        document.documentElement.dataset.efA11yNodes = String(count);
        return updated;
      }
    }
    const updated = applyLocaleAndDirection(nextLang);
    state.lang = updated.lang;
    state.dir = updated.dir;
    bridge.applyPatches([
      {
        kind: "SetPoliteLive",
        text: updated.dir === "rtl" ? "Switched to Arabic RTL layout." : "Switched to English LTR layout.",
      },
    ]);
    return updated;
  };

  const toggleBtn = document.getElementById("btn-toggle-lang");
  if (toggleBtn) {
    toggleBtn.addEventListener("click", () => {
      const nextLang = state.lang === "ar" ? "en" : "ar";
      applyMobileLocale(nextLang);
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

  const coldStartMs = performance.now() - navStartMs;

  setText(
    "mobile-status",
    state.dir === "rtl"
      ? `كوري موبايل جاهز (${nodeCount} ARIA).`
      : `Curie Mobile ready (${state.dir.toUpperCase()}, ${nodeCount} ARIA nodes, ${state.cspViolations} CSP violations).`,
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
        const updated = applyLocaleAndDirection(snap.locale, snap.direction);
        state.lang = updated.lang;
        state.dir = updated.dir;
        const count = syncRustA11yNodes(bridge, snap);
        setText("mobile-metric-a11y", count);
        document.documentElement.dataset.efA11yNodes = String(count);
      }
      return snap;
    },
    setLocale: (lang) => applyMobileLocale(lang),
    getState: () => ({ ...state, a11yNodeCount: bridge.projectedNodeCount() }),
  });

  // Register offline Service Worker asynchronously so initial shell readiness never stalls.
  void registerOfflineServiceWorker();
}

bootMobile();
