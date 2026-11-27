const MOUNT_PREFIX = "/elysianfields";

function isWithinMount(pathname) {
  return pathname === MOUNT_PREFIX || pathname.startsWith(`${MOUNT_PREFIX}/`);
}

function resolveMountUrl(relativePath) {
  const cleaned = relativePath.replace(/^\/+/, "");
  return `${window.location.origin}${MOUNT_PREFIX}/${cleaned}`;
}

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = String(value);
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
  try {
    webgl2Context = canvas.getContext("webgl2", {
      antialias: true,
      powerPreference: "default",
    });
  } catch (_err) {
    webgl2Context = null;
  }

  return {
    webgpuAvailable,
    webgl2Available: webgl2Context !== null,
    webgl2Context,
  };
}

function attachContextLossRecovery(canvas, state) {
  canvas.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    state.contextState = "paused-for-recovery";
    state.preservedDraft = document.getElementById("composer-draft")?.value ?? "";
    setText("metric-context", state.contextState);
    setText("status-banner", "Graphics context lost — rendering paused, UI state preserved.");
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
  });
}

async function loadWasmProbe(capabilities) {
  const startMs = performance.now();
  const glueUrl = resolveMountUrl("pkg/m1_web_delivery.js");
  const wasmUrl = resolveMountUrl("pkg/m1_web_delivery_bg.wasm");

  const moduleNs = await import(glueUrl);
  const exports = await moduleNs.default({ module_or_path: wasmUrl });
  const probeCode =
    typeof exports.elysian_m1_delivery_probe === "function"
      ? exports.elysian_m1_delivery_probe(
          capabilities.webgpuAvailable ? 1 : 0,
          capabilities.webgl2Available ? 1 : 0,
        )
      : -1;
  const elapsedMs = performance.now() - startMs;
  return { probeCode, elapsedMs };
}

async function boot() {
  const navStartMs = performance.now();
  const state = {
    cspViolations: 0,
    contextState: "healthy",
    surfaceGeneration: 1,
    preservedDraft: "",
  };

  document.addEventListener("securitypolicyviolation", () => {
    state.cspViolations += 1;
    setText("metric-csp-violations", state.cspViolations);
  });

  const pathname = window.location.pathname;
  const withinMount = isWithinMount(pathname);
  setText("metric-subpath", withinMount ? `${pathname} (valid)` : `${pathname} (invalid-mount)`);

  const canvas = document.getElementById("elysian-canvas");
  attachContextLossRecovery(canvas, state);

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
    setText(
      "status-banner",
      "Hardware-accelerated browser graphics unavailable; displaying readable fallback.",
    );
  } else {
    setText("metric-context", `healthy (gen ${state.surfaceGeneration})`);
  }

  try {
    const { probeCode, elapsedMs } = await loadWasmProbe(capabilities);
    const totalColdStartMs = performance.now() - navStartMs;
    setText("metric-cold-start", `${totalColdStartMs.toFixed(1)} ms (wasm ${elapsedMs.toFixed(1)} ms)`);
    setText("metric-probe-code", probeCode);
    if (backendLabel !== "unsupported") {
      setText(
        "status-banner",
        `Ready on ${backendLabel} under ${MOUNT_PREFIX}/ with ${state.cspViolations} CSP violations.`,
      );
    }
    document.documentElement.dataset.m1Ready = "true";
    document.documentElement.dataset.m1Backend = backendLabel;
    document.documentElement.dataset.m1ProbeCode = String(probeCode);
    document.documentElement.dataset.m1CspViolations = String(state.cspViolations);
    document.documentElement.dataset.m1ColdStartMs = totalColdStartMs.toFixed(2);
    document.documentElement.dataset.m1WasmInitMs = elapsedMs.toFixed(2);
  } catch (err) {
    setText("metric-cold-start", "bundle-unavailable");
    setText("metric-probe-code", "error");
    setText("status-banner", `Wasm load failed: ${err instanceof Error ? err.message : String(err)}`);
    document.documentElement.dataset.m1Ready = "error";
    document.documentElement.dataset.m1Error = err instanceof Error ? err.message : String(err);
  }
}

boot();
