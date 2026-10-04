// Service worker that makes the page cross-origin isolated on a host that
// cannot send the two headers (web/README.md): it adds them to every
// response that does not already carry them. Registered by coi.js.
const HEADERS = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (e) => {
  const request = e.request;
  if (request.cache === "only-if-cached" && request.mode !== "same-origin") return;
  e.respondWith(
    fetch(request).then((response) => {
      if (response.status === 0) return response; // opaque: pass through
      const headers = new Headers(response.headers);
      for (const [name, value] of Object.entries(HEADERS)) {
        if (!headers.has(name)) headers.set(name, value);
      }
      return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
    }),
  );
});
