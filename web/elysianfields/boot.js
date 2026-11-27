/**
 * Strict-CSP, zero-secret browser shell for Elysian Fields mounted under `/elysianfields/`.
 *
 * Responsibilities:
 * - Enforces segment-bounded `/elysianfields` subpath mounting without domain-root assumptions.
 * - Mounts the single-owner ARIA tree via `./a11y-bridge.js` (`ElysianNarrowA11yBridge`).
 * - Connects to live `/elysianfields/api/...` HTTP/SSE endpoints when backed by `ef-clinic-server`.
 * - Supports optional compiled Bevy Wasm module loading under `/elysianfields/pkg/`.
 * - Handles WebGPU/WebGL2 capability detection and in-place `webglcontextlost`/`webglcontextrestored`.
 */

import { ElysianNarrowA11yBridge } from "./a11y-bridge.js";

const MOUNT_PREFIX = "/elysianfields";

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

function buildInitialA11yPatches(draftValue) {
  return [
    {
      kind: "UpsertElement",
      element: {
        nodeId: "100",
        controlKey: "nav-patient-roster",
        role: "navigation",
        name: "Patient roster navigation",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "101",
        controlKey: "btn-patient-1",
        role: "button",
        name: "Select synthetic patient Avery, Jordan",
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
        name: "Clinical workspace main region",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "201",
        controlKey: "region-transcript-stream",
        role: "region",
        name: "Clinical transcript stream",
      },
    },
    {
      kind: "UpsertElement",
      element: {
        nodeId: "202",
        controlKey: "textbox-composer",
        role: "textbox",
        name: "Clinical prompt composer",
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
        name: "Send clinical prompt",
        tabOrder: "Sequential",
      },
    },
    {
      kind: "SetPoliteLive",
      text: "Curie Clinic accessibility tree mounted under /elysianfields/.",
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
  li.textContent = text;
  list.appendChild(li);
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

  const appRoot = document.getElementById("elysian-app") ?? document.body;
  const composerEl = document.getElementById("composer-draft");
  const bridge = new ElysianNarrowA11yBridge(
    appRoot,
    (action) => {
      state.lastAction = action;
      if (action.type === "set_value" && composerEl && typeof action.value === "string") {
        composerEl.value = action.value;
      }
    },
    { mountSubpath: MOUNT_PREFIX },
  );

  bridge.applyPatches(buildInitialA11yPatches(composerEl?.value ?? ""));
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
      appendListItem("transcript-list", `Clinician: ${text}`);
      bridge.applyPatches([
        {
          kind: "SetPoliteLive",
          text: `Prompt queued for ${state.activeWorkspaceId}.`,
        },
      ]);
    });
  }

  const loadWasmBtn = document.getElementById("btn-load-wasm");
  if (loadWasmBtn) {
    loadWasmBtn.addEventListener("click", async () => {
      try {
        const { probeCode, elapsedMs } = await loadOptionalWasmBundle(capabilities);
        setText("metric-probe-code", probeCode);
        setText("metric-cold-start", `${elapsedMs.toFixed(1)} ms (wasm)`);
      } catch (_err) {
        setText("metric-probe-code", "not-packaged");
      }
    });
  }

  // Load compiled Bevy Wasm bundle automatically only when explicitly requested via ?wasm=1
  // so static-preview and live-API shell runs produce zero spurious 404 console errors.
  const searchParams = new URLSearchParams(window.location.search);
  let probeCode = "standby";
  let wasmInitMs = null;
  if (searchParams.get("wasm") === "1") {
    try {
      const bundleName = searchParams.get("bundle") || "m1_web_delivery";
      const result = await loadOptionalWasmBundle(capabilities, bundleName);
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
      : `Curie Clinic ready on ${backendLabel} under ${MOUNT_PREFIX}/ (${nodeCount} ARIA nodes, ${state.cspViolations} CSP violations).`,
  );

  document.documentElement.dataset.efReady = "true";
  document.documentElement.dataset.efSubpathValid = String(withinMount);
  document.documentElement.dataset.efBackend = backendLabel;
  document.documentElement.dataset.efA11yNodes = String(nodeCount);
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
    getState: () => ({ ...state, a11yNodeCount: bridge.projectedNodeCount() }),
    loadWasmBundle: (bundleName) => loadOptionalWasmBundle(capabilities, bundleName),
  });
}

boot();
