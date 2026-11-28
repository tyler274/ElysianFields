const RTL_LOCALES = new Set(["ar", "he"]);
const KEYS = "lang|dark|light|pairing|storage|outbox|triage|capture|submit|idle|empty|done|refresh|clear|unpaired|paired".split("|");
const CATALOG = {
  en: "Language|Dark|Light|Pairing|Storage|Outbox|Quick-Capture Triage|Capture (local hash)|Hash & triage|Ready (zero raw text).|Enter text.|Triaged|Refresh|Clear|Not paired.|Connected.".split("|"),
  es: "Idioma|Oscuro|Claro|Enlace|Sesión|Cola|Triaje rápido|Captura (hash local)|Resumir|Listo (sin texto).|Texto.|Triado|Actualizar|Borrar|Sin enlace.|Conectado.".split("|"),
  fr: "Langue|Sombre|Clair|Lien|Session|File|Triage rapide|Capture (hash local)|Hacher|Prêt (zéro texte).|Texte.|Trié|Actualiser|Effacer|Non lié.|Connecté.".split("|"),
  pt: "Idioma|Escuro|Claro|Vínculo|Sessão|Fila|Triagem rápida|Captura (hash local)|Hashear|Pronto (zero texto).|Texto.|Triado|Atualizar|Limpar|Sem vínculo.|Conectado.".split("|"),
  de: "Sprache|Dunkel|Hell|Kopplung|Speicher|Ausgang|Schnell-Triage|Hash lokal|Hashen|Bereit (kein Text).|Text.|Triagiert|Prüfen|Leeren|Ungekoppelt.|Verbunden.".split("|"),
  "zh-Hans": "语言|深色|浅色|配对|存储|队列|快速分诊|本地哈希（不存）|分诊|就绪。|请输入。|已分诊|刷新|清除|未配对。|已连接。".split("|"),
  ja: "言語|ダーク|ライト|接続|保存|送信|簡易トリアージ|ローカルハッシュ|実行|準備完了。|入力。|完了|更新|消去|未接続。|接続済。".split("|"),
  ko: "언어|다크|라이트|연결|저장|대기|빠른 트리아지|로컬 해시(미저장)|분류|준비됨.|입력.|완료|새로고침|지우기|미연결.|연결됨.".split("|"),
  ar: "اللغة|داكن|فاتح|اقتران|تخزين|معلقة|فرز سريع|تجزئة محلية|تجزئة|جاهز.|أدخل نصاً.|مفروز|تحديث|مسح|غير مقترن.|متصل.".split("|"),
  he: "שפה|כהה|בהיר|צימוד|אחסון|תור|מיון מהיר|גיבוב מקומי|גבב|מוכן.|הזן טקסט.|מוין|רענן|נקה|לא מצומד.|מחובר.".split("|"),
  vi: "Ngôn ngữ|Tối|Sáng|Ghép|Lưu|Hàng đợi|Phân loại nhanh|Băm cục bộ|Băm|Sẵn sàng.|Nhập.|Đã lọc|Làm mới|Xóa|Chưa ghép.|Đã kết nối.".split("|"),
  hi: "भाषा|डार्क|लाइट|पेयर|स्टोर|कतार|त्वरित ट्राइएज|स्थानीय हैश|हैश|तैयार।|लिखें।|पूर्ण|ताज़ा|साफ़|युग्मित नहीं।|कनेक्टेड।".split("|"),
};
const el = (id) => document.getElementById(id);
const statusEl = el("connector-status"), pairingEl = el("detail-pairing"), outboxEl = el("detail-outbox");
const localeSelect = el("locale-select"), themeBtn = el("btn-theme"), triageForm = el("triage-form");
const triageKindEl = el("triage-kind"), triagePriorityEl = el("triage-priority");
const triageInputEl = el("triage-input"), triageStatusEl = el("triage-status");
const refreshBtn = el("btn-refresh"), disconnectBtn = el("btn-disconnect");
let currentLocale = "en", currentTheme = "light", lastSnapshot = null, lastTriageMeta = null;

function resolveLocale(raw) {
  if (!raw) return "en";
  const norm = String(raw).trim().replace("_", "-").toLowerCase();
  if (norm.startsWith("zh")) return "zh-Hans";
  for (const c of Object.keys(CATALOG)) {
    const l = c.toLowerCase();
    if (norm === l || norm.startsWith(`${l}-`)) return c;
  }
  return "en";
}
const msg = (k) => (k === "title" ? "Elysian Fields" : (CATALOG[currentLocale]?.[KEYS.indexOf(k)] ?? CATALOG.en[KEYS.indexOf(k)] ?? k));
function updateThemeBtn() {
  if (!themeBtn) return;
  const dark = currentTheme === "dark";
  themeBtn.setAttribute("aria-pressed", String(dark));
  themeBtn.textContent = dark ? msg("light") : msg("dark");
}
function applyTheme(t) {
  currentTheme = t === "dark" ? "dark" : "light";
  document.documentElement.dataset.theme = currentTheme;
  updateThemeBtn();
}
function applyLocale(raw) {
  currentLocale = resolveLocale(raw);
  document.documentElement.lang = currentLocale;
  document.documentElement.dir = RTL_LOCALES.has(currentLocale) ? "rtl" : "ltr";
  if (localeSelect) { localeSelect.value = currentLocale; localeSelect.setAttribute("aria-label", msg("lang")); }
  for (const n of document.querySelectorAll("[data-i18n]")) {
    const k = n.getAttribute("data-i18n");
    if (k) n.textContent = msg(k);
  }
  updateThemeBtn(); renderState(lastSnapshot); renderTriageMeta();
}
function redactForHash(raw) {
  let redactions = 0;
  const s = (re, tag, t) => t.replace(re, () => { redactions += 1; return tag; });
  let c = String(raw).trim();
  c = s(/\b\d{3}-\d{2}-\d{4}\b/g, "[REDACTED-ID]", c);
  c = s(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, "[REDACTED-EMAIL]", c);
  c = s(/\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]\d{3}[-.\s]\d{4}\b/g, "[REDACTED-PHONE]", c);
  c = s(/\$\s?\d+(?:,\d{3})*(?:\.\d{2})?/g, "[REDACTED-AMOUNT]", c);
  return { cleaned: c, redactions };
}
function sha256FallbackHex(bytes) {
  const ps = [];
  for (let n = 2; ps.length < 64; n += 1) {
    let ok = true;
    for (let d = 2; d * d <= n; d += 1) if (n % d === 0) { ok = false; break; }
    if (ok) ps.push(n);
  }
  const f32 = (x) => Math.floor((x - Math.floor(x)) * 0x100000000) >>> 0;
  const K = ps.map((p) => f32(Math.cbrt(p))), H = ps.slice(0, 8).map((p) => f32(Math.sqrt(p)));
  const bits = bytes.length * 8, padLen = (bytes.length + 9 + 63) & ~63, pad = new Uint8Array(padLen);
  pad.set(bytes); pad[bytes.length] = 0x80;
  const dv = new DataView(pad.buffer);
  dv.setUint32(padLen - 8, Math.floor(bits / 0x100000000), false);
  dv.setUint32(padLen - 4, bits >>> 0, false);
  const w = new Uint32Array(64), r = (x, n) => (x >>> n) | (x << (32 - n));
  for (let o = 0; o < padLen; o += 64) {
    for (let i = 0; i < 16; i += 1) w[i] = dv.getUint32(o + i * 4, false);
    for (let i = 16; i < 64; i += 1) w[i] = (w[i - 16] + (r(w[i - 15], 7) ^ r(w[i - 15], 18) ^ (w[i - 15] >>> 3)) + w[i - 7] + (r(w[i - 2], 17) ^ r(w[i - 2], 19) ^ (w[i - 2] >>> 10))) >>> 0;
    let [a, b, c, d, e, f, g, h] = H;
    for (let i = 0; i < 64; i += 1) {
      const t1 = (h + (r(e, 6) ^ r(e, 11) ^ r(e, 25)) + ((e & f) ^ (~e & g)) + K[i] + w[i]) >>> 0;
      const t2 = ((r(a, 2) ^ r(a, 13) ^ r(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) >>> 0;
      h = g; g = f; f = e; e = (d + t1) >>> 0; d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    H[0] = (H[0] + a) >>> 0; H[1] = (H[1] + b) >>> 0; H[2] = (H[2] + c) >>> 0; H[3] = (H[3] + d) >>> 0;
    H[4] = (H[4] + e) >>> 0; H[5] = (H[5] + f) >>> 0; H[6] = (H[6] + g) >>> 0; H[7] = (H[7] + h) >>> 0;
  }
  return H.map((v) => v.toString(16).padStart(8, "0")).join("");
}
async function computeSha256Hex(text) {
  const bytes = new TextEncoder().encode(text);
  if (typeof crypto !== "undefined" && crypto.subtle?.digest) {
    try {
      const buf = await crypto.subtle.digest("SHA-256", bytes);
      return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, "0")).join("");
    } catch (_e) { return sha256FallbackHex(bytes); }
  }
  return sha256FallbackHex(bytes);
}
function renderTriageMeta() {
  if (!triageStatusEl) return;
  if (!lastTriageMeta) { triageStatusEl.textContent = msg("idle"); return; }
  const { kind, priority, redactions, digestPrefix } = lastTriageMeta;
  triageStatusEl.textContent = `${msg("done")}: [${priority}/${kind}] sha256:${digestPrefix} (redacted:${redactions})`;
}
async function handleTriageSubmit(ev) {
  ev.preventDefault();
  if (!triageInputEl || !triageStatusEl) return;
  const raw = triageInputEl.value;
  triageInputEl.value = "";
  if (!raw.trim()) { lastTriageMeta = null; triageStatusEl.textContent = msg("empty"); return; }
  const kind = triageKindEl ? triageKindEl.value : "follow_up";
  const priority = triagePriorityEl ? triagePriorityEl.value : "routine";
  const { cleaned, redactions } = redactForHash(raw);
  const digest = await computeSha256Hex(`${priority}:${kind}:${cleaned}`);
  lastTriageMeta = { kind, priority, redactions, digestPrefix: digest.slice(0, 12) };
  renderTriageMeta();
}
function renderState(snap) {
  lastSnapshot = snap;
  if (!snap || !snap.paired) {
    if (statusEl) statusEl.textContent = msg("unpaired");
    if (pairingEl) pairingEl.textContent = "unpaired";
    if (outboxEl) outboxEl.textContent = "0";
    return;
  }
  if (statusEl) statusEl.textContent = msg("paired");
  if (pairingEl) pairingEl.textContent = `paired (${snap.pairingId})`;
  if (outboxEl) outboxEl.textContent = String(snap.outboxPending ?? 0);
}
async function refreshStatus() {
  if (typeof chrome !== "undefined" && chrome.runtime?.sendMessage) {
    try { renderState(await chrome.runtime.sendMessage({ type: "connector.status" })); return; }
    catch (_e) { renderState(null); return; }
  }
  renderState(null);
}
async function clearSession() {
  lastTriageMeta = null;
  if (triageInputEl) triageInputEl.value = "";
  renderTriageMeta();
  if (typeof chrome !== "undefined" && chrome.runtime?.sendMessage) {
    try { await chrome.runtime.sendMessage({ type: "connector.clear_session" }); } catch (_e) { /* ignore */ }
  }
  await refreshStatus();
}
const params = new URLSearchParams(window.location.search);
const prefersDark = typeof window.matchMedia === "function" && window.matchMedia("(prefers-color-scheme: dark)").matches;
applyTheme(params.get("theme") || (prefersDark ? "dark" : "light"));
applyLocale(params.get("lang") || navigator.language || "en");
if (localeSelect) localeSelect.addEventListener("change", () => applyLocale(localeSelect.value));
if (themeBtn) themeBtn.addEventListener("click", () => applyTheme(currentTheme === "dark" ? "light" : "dark"));
if (triageForm) triageForm.addEventListener("submit", handleTriageSubmit);
if (refreshBtn) refreshBtn.addEventListener("click", () => refreshStatus());
if (disconnectBtn) disconnectBtn.addEventListener("click", () => clearSession());
refreshStatus();
