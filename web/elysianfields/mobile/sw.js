// Strict-CSP, zero-secret Service Worker for Curie Mobile PWA under `/elysianfields/mobile/`.
// Precaches the mobile shell and crisis-line resources (988, 911, SAMHSA) while never caching
// authenticated `/elysianfields/mobile-api/` responses.

const CACHE_NAME = "ef-mobile-pwa-v1";
const PRECACHE_ASSETS = [
  "./",
  "./index.html",
  "./mobile.css",
  "./mobile-boot.js",
  "./manifest.webmanifest",
  "../a11y-bridge.js",
];

const OFFLINE_FALLBACK_HTML = `<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Curie Mobile (Offline)</title></head>
<body>
  <main id="mobile-app">
    <h1>Curie Mobile (Offline Mode)</h1>
    <section id="crisis-resources" aria-label="Immediate crisis support">
      <h2>Immediate Crisis &amp; Recovery Support</h2>
      <ul>
        <li>988 Suicide &amp; Crisis Lifeline: Call or text 988</li>
        <li>Emergency: Call 911</li>
        <li>SAMHSA National Helpline: 1-800-662-4357</li>
      </ul>
    </section>
    <p>You are offline. Cached support information remains available.</p>
  </main>
</body>
</html>`;

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(PRECACHE_ASSETS))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((key) => key !== CACHE_NAME).map((key) => caches.delete(key))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") {
    return;
  }

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) {
    return;
  }

  // Never cache clinical or authentication API payloads.
  if (url.pathname.startsWith("/elysianfields/mobile-api/")) {
    return;
  }

  if (
    !url.pathname.startsWith("/elysianfields/mobile") &&
    url.pathname !== "/elysianfields/a11y-bridge.js"
  ) {
    return;
  }

  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE_NAME);
      try {
        const networkResp = await fetch(req);
        if (networkResp && networkResp.ok) {
          await cache.put(url.pathname, networkResp.clone());
        }
        return networkResp;
      } catch (_offlineErr) {
        const exactMatch = await cache.match(url.pathname);
        if (exactMatch) {
          return exactMatch;
        }
        const rawMatch = await cache.match(req, { ignoreSearch: true });
        if (rawMatch) {
          return rawMatch;
        }
        if (req.mode === "navigate" || url.pathname.endsWith("/mobile/") || url.pathname.endsWith("/mobile")) {
          const indexResp = await cache.match("./index.html");
          if (indexResp) {
            return indexResp;
          }
          return new Response(OFFLINE_FALLBACK_HTML, {
            status: 200,
            headers: { "Content-Type": "text/html; charset=utf-8" },
          });
        }
        return new Response("Offline", { status: 503 });
      }
    })(),
  );
});
