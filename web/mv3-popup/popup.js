const startMs = performance.now();
let cspViolations = 0;

function setText(id, value) {
  const el = document.getElementById(id);
  if (el) {
    el.textContent = String(value);
  }
}

document.addEventListener("securitypolicyviolation", () => {
  cspViolations += 1;
  setText("popup-csp-violations", cspViolations);
});

async function initializePopup() {
  const canvas = document.getElementById("popup-canvas");
  let backend = "unsupported";

  if ("gpu" in navigator && navigator.gpu && typeof navigator.gpu.requestAdapter === "function") {
    try {
      const adapter = await navigator.gpu.requestAdapter();
      if (adapter !== null) {
        backend = "webgpu";
      }
    } catch (_err) {
      backend = "unsupported";
    }
  }

  if (backend === "unsupported" && canvas) {
    const gl = canvas.getContext("webgl2");
    if (gl !== null) {
      backend = "webgl2";
    }
  }

  const elapsedMs = performance.now() - startMs;
  setText("popup-backend", backend);
  setText("popup-csp-violations", cspViolations);
  setText("popup-startup", `${elapsedMs.toFixed(1)} ms`);
  setText(
    "popup-status",
    backend === "unsupported"
      ? "Semantic HTML fallback active (no WebGPU/WebGL2)."
      : `MV3 strict-CSP popup ready (${backend}).`,
  );
  document.documentElement.dataset.mv3Ready = "true";
  document.documentElement.dataset.mv3Backend = backend;
  document.documentElement.dataset.mv3CspViolations = String(cspViolations);
  document.documentElement.dataset.mv3StartupMs = elapsedMs.toFixed(2);
}

initializePopup();
