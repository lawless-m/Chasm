// Registers coi-sw.js when the server did not send the cross-origin
// isolation headers, and reloads once so the worker serves the page.
// Does nothing on a page that is already isolated.
(function () {
  const FLAG = "coi-reloaded";
  if (window.crossOriginIsolated) {
    sessionStorage.removeItem(FLAG);
    return;
  }
  if (!navigator.serviceWorker) return;
  if (sessionStorage.getItem(FLAG)) {
    console.warn("coi.js: still not cross-origin isolated after a reload");
    return;
  }
  function reload() {
    if (sessionStorage.getItem(FLAG)) return;
    sessionStorage.setItem(FLAG, "1");
    location.reload();
  }
  navigator.serviceWorker.addEventListener("controllerchange", reload);
  navigator.serviceWorker.register("coi-sw.js").then(() =>
    navigator.serviceWorker.ready.then(() => {
      if (!navigator.serviceWorker.controller) reload();
    }),
  );
})();
