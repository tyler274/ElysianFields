const statusEl = document.getElementById("connector-status");
const pairingEl = document.getElementById("detail-pairing");
const outboxEl = document.getElementById("detail-outbox");
const refreshBtn = document.getElementById("btn-refresh");
const disconnectBtn = document.getElementById("btn-disconnect");

const queryTheme = new URLSearchParams(window.location.search).get("theme");
if (queryTheme === "light" || queryTheme === "dark") {
  document.documentElement.dataset.theme = queryTheme;
}

function renderState(snapshot) {
  if (!snapshot || !snapshot.paired) {
    if (statusEl) statusEl.textContent = "Not paired. Request optional host permission from clinic workspace.";
    if (pairingEl) pairingEl.textContent = "unpaired";
    if (outboxEl) outboxEl.textContent = "0";
    return;
  }

  if (statusEl) statusEl.textContent = "Connected (read-only, memory-only session active).";
  if (pairingEl) pairingEl.textContent = `paired (${snapshot.pairingId})`;
  if (outboxEl) outboxEl.textContent = String(snapshot.outboxPending ?? 0);
}

async function refreshStatus() {
  if (typeof chrome !== "undefined" && chrome.runtime && typeof chrome.runtime.sendMessage === "function") {
    try {
      const response = await chrome.runtime.sendMessage({ type: "connector.status" });
      renderState(response);
      return;
    } catch (_err) {
      renderState(null);
      return;
    }
  }
  renderState(null);
}

async function clearSession() {
  if (typeof chrome !== "undefined" && chrome.runtime && typeof chrome.runtime.sendMessage === "function") {
    try {
      await chrome.runtime.sendMessage({ type: "connector.clear_session" });
    } catch (_err) {
      // Fall through to status refresh.
    }
  }
  await refreshStatus();
}

if (refreshBtn) {
  refreshBtn.addEventListener("click", () => {
    refreshStatus();
  });
}

if (disconnectBtn) {
  disconnectBtn.addEventListener("click", () => {
    clearSession();
  });
}

refreshStatus();
