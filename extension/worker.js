/**
 * MV3 background service worker: strictly memory-only credentials in chrome.storage.session,
 * optional host permission checks, declared named read dispatch, and idempotent redacted outbox.
 */

import { redactAndReceipt, validateLeasedReadJob } from "./connector.js";

const SESSION_KEY = "ef_connector_session";
const outbox = new Map();

async function getSessionCredentials() {
  if (typeof chrome === "undefined" || !chrome.storage || !chrome.storage.session) {
    return null;
  }
  const stored = await chrome.storage.session.get(SESSION_KEY);
  const session = stored[SESSION_KEY] ?? null;
  if (!session) {
    return null;
  }
  if (typeof session.expiresAtMs === "number" && Date.now() >= session.expiresAtMs) {
    await chrome.storage.session.remove(SESSION_KEY);
    return null;
  }
  return session;
}

async function setSessionCredentials(session) {
  if (typeof chrome === "undefined" || !chrome.storage || !chrome.storage.session) {
    throw new Error("chrome.storage.session unavailable");
  }
  await chrome.storage.session.set({ [SESSION_KEY]: session });
}

async function clearSessionCredentials() {
  outbox.clear();
  if (typeof chrome !== "undefined" && chrome.storage && chrome.storage.session) {
    await chrome.storage.session.remove(SESSION_KEY);
  }
}

export async function handleLeasedJob(rawJob, rawSourcePayload) {
  const session = await getSessionCredentials();
  if (!session) {
    throw new Error("extension session is not paired or has expired");
  }
  const job = validateLeasedReadJob(rawJob);
  const outboxKey = `${job.id}:${job.lease}`;
  if (!outbox.has(outboxKey)) {
    const redacted = await redactAndReceipt(rawSourcePayload);
    outbox.set(outboxKey, {
      job,
      payload: redacted.payload,
      sha256Receipt: redacted.sha256Receipt,
      delivered: false,
    });
  }
  return outbox.get(outboxKey);
}

if (typeof chrome !== "undefined" && chrome.runtime && chrome.runtime.onMessage) {
  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    (async () => {
      if (!message || typeof message.type !== "string") {
        sendResponse({ ok: false, error: "invalid message" });
        return;
      }

      if (message.type === "connector.status") {
        const session = await getSessionCredentials();
        sendResponse({
          paired: session !== null,
          pairingId: session ? session.pairingId : null,
          outboxPending: Array.from(outbox.values()).filter((e) => !e.delivered).length,
        });
        return;
      }

      if (message.type === "connector.pair") {
        await setSessionCredentials({
          pairingId: String(message.pairingId ?? ""),
          bearerToken: String(message.bearerToken ?? ""),
          refreshToken: String(message.refreshToken ?? ""),
          expiresAtMs: Number(message.expiresAtMs ?? 0),
        });
        sendResponse({ ok: true });
        return;
      }

      if (message.type === "connector.clear_session") {
        await clearSessionCredentials();
        sendResponse({ ok: true });
        return;
      }

      sendResponse({ ok: false, error: "unsupported message type" });
    })();
    return true;
  });
}
