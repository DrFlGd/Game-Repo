// Chooses how the page reaches the app's backend. Everything else in web/ talks
// only to the platform object, so the same front end runs in the desktop app and
// against `gamerepo-cli serve` in the tests (tests/tauri_shim.js stands in for Tauri).
//
// createPlatform() -> { kind, info, store, api(cmd, args), openUrl(url), library?, refreshInfo() }
//   info:  app_info (version, os, the open library, recent libraries)
//   api:   one of the app's commands (desktop/core/src/api.rs), answered as JSON
//   store: prefs.get(key, fallback) / prefs.set(key, value), small UI preferences, synchronous
//   library (desktop): pickFolder(title), pickFile(s), openPath(path), reveal(path), url(rel) for files in the library

export async function createPlatform() {
  if (globalThis.__TAURI_INTERNALS__) {
    const desktop = await import("./platform-desktop.js");
    return desktop.createPlatform();
  }
  // opened as a plain web page: there's no backend
  const prefs = new Map();
  return {
    kind: "browser",
    info: { version: "", library: null, library_error: "Open Game Repo in the desktop app to use a library.", recent_libraries: [] },
    store: { prefs: { get: (k, fallback = null) => (prefs.has(k) ? prefs.get(k) : fallback), set: (k, v) => prefs.set(k, v) } },
    api: async () => { throw new Error("No library here: open Game Repo in the desktop app."); },
    openUrl: (url) => window.open(url, "_blank", "noopener"),
    async refreshInfo() { return this.info; },
  };
}
