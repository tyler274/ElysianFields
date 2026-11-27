// Narrow browser accessibility semantic overlay for Elysian Fields Wasm clients.
//
// Design invariants:
// - Single source of truth: every DOM node is keyed 1:1 by `accesskit::NodeId`
//   from `AccessibilityTreeSnapshot` (`data-ef-node-id`).
// - Semantic-only projection: zero CSS layout mirroring, zero theme duplication,
//   and zero duplicate editable text store. All user interactions dispatch back
//   to the Rust/Bevy `FocusManager` and `ScreenState`.
// - Subpath-safe: mounts strictly under `/elysianfields` without assuming `/`.

const OVERLAY_ROOT_ID = "ef-a11y-overlay";
const SUBPATH_PREFIX = "/elysianfields";

const ROLE_TO_TAG = Object.freeze({
  navigation: "nav",
  main: "main",
  form: "form",
  region: "section",
  heading: "h2",
  link: "a",
  button: "button",
  textbox: "textarea",
  checkbox: "input",
  dialog: "div",
  alert: "div",
  status: "div",
});

/**
 * Visually-hidden container styles that keep controls reachable by VoiceOver,
 * TalkBack, NVDA, and Orca without mirroring GPU canvas layout or visuals.
 */
function applySrOnlyStyles(element) {
  element.style.position = "fixed";
  element.style.width = "1px";
  element.style.height = "1px";
  element.style.padding = "0";
  element.style.margin = "-1px";
  element.style.overflow = "hidden";
  element.style.clip = "rect(0, 0, 0, 0)";
  element.style.whiteSpace = "nowrap";
  element.style.border = "0";
  element.style.pointerEvents = "none";
}

export class ElysianNarrowA11yBridge {
  /**
   * @param {HTMLElement} hostContainer - Container under the `/elysianfields` app root.
   * @param {(event: { type: string, nodeId: string, value?: string }) => void} onAction
   * @param {{ mountSubpath?: string }} [options]
   */
  constructor(hostContainer, onAction, options = {}) {
    const mountSubpath = options.mountSubpath ?? SUBPATH_PREFIX;
    if (!mountSubpath.startsWith(SUBPATH_PREFIX)) {
      throw new Error(
        `ElysianNarrowA11yBridge mountSubpath must start with ${SUBPATH_PREFIX}`
      );
    }

    this.mountSubpath = mountSubpath;
    this.onAction = onAction;
    /** @type {Map<string, HTMLElement>} */
    this.nodesById = new Map();

    this.root = document.createElement("div");
    this.root.id = OVERLAY_ROOT_ID;
    this.root.dataset.efSubpath = this.mountSubpath;
    applySrOnlyStyles(this.root);

    this.treeContainer = document.createElement("div");
    this.treeContainer.dataset.efLayer = "semantic-controls";

    this.politeLiveRegion = document.createElement("div");
    this.politeLiveRegion.setAttribute("role", "status");
    this.politeLiveRegion.setAttribute("aria-live", "polite");
    this.politeLiveRegion.setAttribute("aria-atomic", "false");

    this.assertiveLiveRegion = document.createElement("div");
    this.assertiveLiveRegion.setAttribute("role", "alert");
    this.assertiveLiveRegion.setAttribute("aria-live", "assertive");
    this.assertiveLiveRegion.setAttribute("aria-atomic", "true");

    this.root.append(
      this.treeContainer,
      this.politeLiveRegion,
      this.assertiveLiveRegion
    );
    hostContainer.appendChild(this.root);
  }

  /**
   * Applies differential `BridgePatch` messages emitted by Rust `NarrowSemanticBridge`.
   * @param {Array<Record<string, unknown>>} patches
   */
  applyPatches(patches) {
    for (const patch of patches) {
      switch (patch.kind) {
        case "UpsertElement":
          this.#upsertElement(patch.element);
          break;
        case "RemoveElement":
          this.#removeElement(String(patch.nodeId));
          break;
        case "SetFocus":
          this.#syncFocus(patch.nodeId == null ? null : String(patch.nodeId));
          break;
        case "SetPoliteLive":
          this.politeLiveRegion.textContent = patch.text ?? "";
          break;
        case "SetAssertiveLive":
          this.assertiveLiveRegion.textContent = patch.text ?? "";
          break;
        case "SetModalScope":
          this.#syncModalInert(patch.dialogNodeId ?? null);
          break;
        default:
          throw new Error(`Unknown accessibility bridge patch kind: ${patch.kind}`);
      }
    }
  }

  /** Returns the current projected DOM node count (must equal visible AccessKit nodes). */
  projectedNodeCount() {
    return this.nodesById.size;
  }

  destroy() {
    this.nodesById.clear();
    this.root.remove();
  }

  #upsertElement(spec) {
    const nodeId = String(spec.nodeId);
    const desiredTag = ROLE_TO_TAG[spec.role] ?? "div";
    let el = this.nodesById.get(nodeId);

    if (!el || el.tagName.toLowerCase() !== desiredTag) {
      if (el) {
        el.remove();
      }
      el = document.createElement(desiredTag);
      el.dataset.efNodeId = nodeId;
      el.dataset.efControl = spec.controlKey;
      this.#wireInteractions(el, nodeId, spec.role);
      this.nodesById.set(nodeId, el);
      this.treeContainer.appendChild(el);
    }

    el.setAttribute("role", spec.role);
    el.setAttribute("aria-label", spec.name);

    if (spec.role === "checkbox") {
      el.setAttribute("type", "checkbox");
      el.checked = spec.toggled === "True";
      el.setAttribute("aria-checked", String(spec.toggled === "True"));
    } else if (spec.role === "textbox") {
      if (el.value !== (spec.value ?? "")) {
        el.value = spec.value ?? "";
      }
    } else if (spec.role === "link") {
      el.setAttribute("href", "#");
      el.textContent = spec.name;
    } else {
      el.textContent = spec.value ?? spec.name;
    }

    if (spec.disabled) {
      el.setAttribute("aria-disabled", "true");
    } else {
      el.removeAttribute("aria-disabled");
    }

    if (spec.expanded !== undefined) {
      el.setAttribute("aria-expanded", String(spec.expanded));
    } else {
      el.removeAttribute("aria-expanded");
    }

    if (spec.selected !== undefined) {
      el.setAttribute("aria-current", spec.selected ? "page" : "false");
    } else {
      el.removeAttribute("aria-current");
    }

    if (spec.modal) {
      el.setAttribute("aria-modal", "true");
    } else {
      el.removeAttribute("aria-modal");
    }

    if (spec.invalid) {
      el.setAttribute("aria-invalid", "true");
    } else {
      el.removeAttribute("aria-invalid");
    }

    if (spec.tabOrder === "Sequential") {
      el.tabIndex = 0;
    } else if (spec.tabOrder === "ProgrammaticOnly") {
      el.tabIndex = -1;
    } else {
      el.removeAttribute("tabindex");
    }
  }

  #removeElement(nodeId) {
    const existing = this.nodesById.get(nodeId);
    if (existing) {
      existing.remove();
      this.nodesById.delete(nodeId);
    }
  }

  #syncFocus(nodeId) {
    if (nodeId == null) {
      return;
    }
    const target = this.nodesById.get(nodeId);
    if (target && document.activeElement !== target) {
      target.focus({ preventScroll: true });
    }
  }

  #syncModalInert(dialogNodeId) {
    for (const [nodeId, el] of this.nodesById.entries()) {
      const isOutsideModal =
        dialogNodeId !== null &&
        nodeId !== String(dialogNodeId) &&
        !el.dataset.efControl?.startsWith("button-modal-") &&
        !el.dataset.efControl?.startsWith("heading-sign-off");
      el.inert = isOutsideModal;
    }
  }

  #wireInteractions(el, nodeId, role) {
    el.addEventListener("click", (event) => {
      event.preventDefault();
      this.onAction({ type: "activate", nodeId });
    });
    el.addEventListener("focus", () => {
      this.onAction({ type: "focus", nodeId });
    });
    if (role === "textbox") {
      el.addEventListener("input", (event) => {
        this.onAction({
          type: "set_value",
          nodeId,
          value: event.target.value,
        });
      });
    }
  }
}
