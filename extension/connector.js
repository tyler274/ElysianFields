/**
 * Vendor-neutral read-only connector helpers: strict allowlisted named job validation,
 * billing/price/payment redaction, and SHA-256 payload receipt computation.
 */

export const ALLOWED_JOB_KEYS = new Set([
  "id",
  "lease",
  "source",
  "read",
  "patient_id",
  "item_id",
]);

export const DECLARED_READ_OPERATIONS = new Set([
  "patient_summary",
  "document_index",
  "document_item",
  "encounter_list",
  "result_list",
  "medication_list",
]);

const FORBIDDEN_KEY_SUBSTRINGS = [
  "billing",
  "invoice",
  "price",
  "cost",
  "copay",
  "co_pay",
  "coinsurance",
  "deductible",
  "charge",
  "amount_due",
  "balance_due",
  "unit_price",
  "medication_price",
  "rx_price",
  "claim_amount",
  "payer_amount",
  "payment",
  "credit_card",
  "card_number",
  "bank_account",
  "routing_number",
  "ssn",
  "social_security",
];

function isSafeIdentifier(value) {
  if (typeof value !== "string" || value.length === 0 || value.trim() !== value) {
    return false;
  }
  const lower = value.toLowerCase();
  return !(
    lower.includes("://") ||
    lower.startsWith("//") ||
    lower.startsWith("http:") ||
    lower.startsWith("https:") ||
    lower.startsWith("javascript:") ||
    lower.startsWith("data:") ||
    lower.includes("..") ||
    lower.includes("\\") ||
    lower.includes("?") ||
    lower.includes("#")
  );
}

export function validateLeasedReadJob(job) {
  if (!job || typeof job !== "object" || Array.isArray(job)) {
    throw new Error("job payload must be an object");
  }
  for (const key of Object.keys(job)) {
    if (!ALLOWED_JOB_KEYS.has(key)) {
      throw new Error(`forbidden job field '${key}'`);
    }
  }
  for (const required of ["id", "lease", "source", "read", "patient_id"]) {
    if (!isSafeIdentifier(job[required])) {
      throw new Error(`invalid or unsafe field '${required}'`);
    }
  }
  if (!DECLARED_READ_OPERATIONS.has(job.read)) {
    throw new Error(`undeclared read operation '${job.read}'`);
  }
  if (job.item_id !== undefined && job.item_id !== null && !isSafeIdentifier(job.item_id)) {
    throw new Error("invalid or unsafe field 'item_id'");
  }
  return job;
}

function isForbiddenKey(key) {
  const normalized = key.toLowerCase().replace(/[-\s]/g, "_");
  return FORBIDDEN_KEY_SUBSTRINGS.some((token) => normalized.includes(token));
}

export function redactPayload(value) {
  if (Array.isArray(value)) {
    return value.map((item) => redactPayload(item));
  }
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) {
      if (isForbiddenKey(key)) {
        continue;
      }
      out[key] = redactPayload(value[key]);
    }
    return out;
  }
  if (typeof value === "string") {
    return value.replace(/\b\d{3}-\d{2}-\d{4}\b/g, "[REDACTED-SSN]");
  }
  return value;
}

export async function redactAndReceipt(rawPayload) {
  const payload = redactPayload(rawPayload);
  const canonical = JSON.stringify(payload);
  const encoded = new TextEncoder().encode(canonical);
  const digestBuf = await crypto.subtle.digest("SHA-256", encoded);
  const sha256Receipt = Array.from(new Uint8Array(digestBuf))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return { payload, canonical, sha256Receipt };
}
