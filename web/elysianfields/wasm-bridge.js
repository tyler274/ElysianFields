/**
 * Shared zero-secret WebAssembly ABI loader and Rust AccessKit -> ARIA bridge projector
 * for Curie Clinic (`/elysianfields/ef_clinic_client.wasm`) and Curie Mobile
 * (`/elysianfields/mobile/ef_mobile_client.wasm`).
 *
 * Works under strict CSP (`script-src 'self' 'wasm-unsafe-eval'`, zero `'unsafe-eval'`)
 * without requiring generated `wasm-bindgen` JS glue.
 */

export async function isWasmModuleAdvertised(manifestUrl, flagKey) {
  try {
    const resp = await fetch(manifestUrl, { cache: "no-store" });
    if (!resp.ok) {
      return false;
    }
    const manifest = await resp.json();
    return Boolean(manifest && manifest[flagKey] === true);
  } catch (_err) {
    return false;
  }
}

export async function loadBevyWasmClient({ wasmUrl, initFn, versionFn, dispatchFn }) {
  const startMs = performance.now();
  const resp = await fetch(wasmUrl);
  if (!resp.ok) {
    throw new Error(`Failed to fetch Wasm module ${wasmUrl}: HTTP ${resp.status}`);
  }

  let wasmModule;
  if (typeof WebAssembly.compileStreaming === "function") {
    try {
      wasmModule = await WebAssembly.compileStreaming(resp.clone());
    } catch (_streamingErr) {
      const bytes = await resp.arrayBuffer();
      wasmModule = await WebAssembly.compile(bytes);
    }
  } else {
    const bytes = await resp.arrayBuffer();
    wasmModule = await WebAssembly.compile(bytes);
  }

  let instanceRef = null;
  const decoder = new TextDecoder("utf-8");
  const encoder = new TextEncoder();

  const readUtf8 = (ptr, len) => {
    if (!instanceRef || len === 0) {
      return "";
    }
    const view = new Uint8Array(instanceRef.exports.memory.buffer, ptr >>> 0, len >>> 0);
    return decoder.decode(view);
  };

  // Construct deterministic host imports for any transitive descriptor symbols linked into
  // `cdylib` output by Bevy's wasm32 crates (e.g. `web-time` / `wasm-bindgen` metadata).
  const importObject = {};
  for (const imp of WebAssembly.Module.imports(wasmModule)) {
    if (!importObject[imp.module]) {
      importObject[imp.module] = {};
    }
    if (imp.kind === "function") {
      if (imp.name.includes("now")) {
        importObject[imp.module][imp.name] = () => performance.now();
      } else if (imp.name.includes("throw")) {
        importObject[imp.module][imp.name] = (ptr, len) => {
          throw new Error(readUtf8(ptr, len) || "Bevy Wasm runtime trap");
        };
      } else if (imp.name.includes("is_undefined")) {
        importObject[imp.module][imp.name] = (idx) => (idx === 0 ? 1 : 0);
      } else {
        importObject[imp.module][imp.name] = () => 0;
      }
    }
  }

  const instance = await WebAssembly.instantiate(wasmModule, importObject);
  instanceRef = instance;
  const exports = instance.exports;

  for (const symbol of ["memory", "ef_alloc", "ef_dealloc", initFn, versionFn, dispatchFn]) {
    if (!(symbol in exports)) {
      throw new Error(`Missing required Bevy Wasm ABI export '${symbol}' in ${wasmUrl}`);
    }
  }

  const decodePackedJson = (packed) => {
    const bits = typeof packed === "bigint" ? packed : BigInt(packed);
    const ptr = Number((bits >> 32n) & 0xffffffffn);
    const len = Number(bits & 0xffffffffn);
    if (ptr === 0 || len === 0) {
      throw new Error(`Bevy Wasm ABI returned empty snapshot pointer from ${wasmUrl}`);
    }
    try {
      return JSON.parse(readUtf8(ptr, len));
    } finally {
      exports.ef_dealloc(ptr, len);
    }
  };

  const abiVersion = Number(exports[versionFn]());
  const initialSnapshot = decodePackedJson(exports[initFn]());
  const loadMs = performance.now() - startMs;

  const dispatch = (commandObj) => {
    const encoded = encoder.encode(JSON.stringify(commandObj));
    const inPtr = exports.ef_alloc(encoded.byteLength);
    if (inPtr === 0) {
      throw new Error("ef_alloc returned null pointer");
    }
    try {
      new Uint8Array(exports.memory.buffer, inPtr >>> 0, encoded.byteLength).set(encoded);
      const packed = exports[dispatchFn](inPtr, encoded.byteLength);
      return decodePackedJson(packed);
    } finally {
      exports.ef_dealloc(inPtr, encoded.byteLength);
    }
  };

  return {
    abiVersion,
    loadMs,
    initialSnapshot,
    dispatch,
  };
}

/**
 * Projects Rust-owned AccessKit ARIA node descriptors 1:1 into `ElysianNarrowA11yBridge`,
 * pruning any stale node IDs so zero duplicate semantic controls ever exist.
 */
export function syncRustA11yNodes(bridge, snapshot, fallbackPoliteText = null) {
  const nodes = Array.isArray(snapshot?.a11y_nodes) ? snapshot.a11y_nodes : [];
  const nextIds = new Set(nodes.map((node) => String(node.nodeId)));
  const patches = [];

  for (const existingId of bridge.nodesById.keys()) {
    if (!nextIds.has(String(existingId))) {
      patches.push({ kind: "RemoveElement", nodeId: String(existingId) });
    }
  }

  for (const node of nodes) {
    patches.push({
      kind: "UpsertElement",
      element: {
        nodeId: String(node.nodeId),
        controlKey: node.controlKey,
        role: node.role,
        name: node.name,
        value: node.value,
        disabled: Boolean(node.disabled),
        modal: Boolean(node.modal),
        tabOrder: node.tabOrder ?? "None",
      },
    });
  }

  const politeText = snapshot?.polite_live || fallbackPoliteText;
  if (politeText) {
    patches.push({ kind: "SetPoliteLive", text: politeText });
  }

  bridge.applyPatches(patches);
  return bridge.projectedNodeCount();
}
