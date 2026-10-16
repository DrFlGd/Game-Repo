// Desktop app side of web/platform.js: the library folder, reached through the
// app's commands (desktop/core/src/api.rs, called through Tauri's "api" command
// in desktop/src-tauri/src/main.rs).

const tauri = globalThis.__TAURI__;
const invoke = (cmd, args, opts) => tauri.core.invoke(cmd, args, opts);
/** One of the app's commands, answered as JSON. */
const api = (cmd, args = {}) => invoke("api", { cmd, args });

export async function createPlatform() {
  const info = await api("app_info").catch((e) => ({ library_error: String(e), recent_libraries: [] }));
  let prefs = await api("prefs_get").catch(() => ({}));
  let timer = null;
  const store = {
    prefs: {
      get(key, fallback = null) { return key in prefs ? prefs[key] : fallback; },
      set(key, value) {
        prefs = { ...prefs, [key]: value };
        clearTimeout(timer);
        timer = setTimeout(() => api("prefs_set", { prefs }).catch(() => {}), 250);
      },
    },
  };
  const libraryUrl = info.library_url || "library://localhost/";
  // web links open in the default browser, not in the app's window
  document.addEventListener("click", (e) => {
    const a = e.target.closest?.("a[href]");
    if (!a || e.defaultPrevented || a.hasAttribute("download")) return;
    let url;
    try { url = new URL(a.href, location.href); } catch { return; }
    if (!/^https?:$/.test(url.protocol) || url.origin === location.origin || url.href.startsWith(libraryUrl)) return;
    e.preventDefault();
    invoke("open_url", { url: url.href }).catch((err) => console.warn("open_url", err));
  });
  return {
    kind: "desktop",
    info,
    store,
    api,
    /** One of the app's commands, answered as bytes (an ArrayBuffer). */
    apiBytes: (cmd, args = {}) => invoke("api_bytes", { cmd, args }),
    /** A web link in the default browser. */
    openUrl: (url) => invoke("open_url", { url }).catch((err) => console.warn("open_url", err)),
    /** Folders and files dropped on the window: `cb(paths, at)`, `at` the point
     *  in CSS pixels. `over(true|false)`: something is being dragged over it. */
    onDrop(cb, over) {
      const listen = (name, fn) => tauri.event?.listen?.(name, fn)?.catch?.(() => {});
      const at = (p) => (p && typeof p.x === "number" ? { x: p.x / (devicePixelRatio || 1), y: p.y / (devicePixelRatio || 1) } : null);
      listen("tauri://drag-drop", (e) => { over?.(false); cb(e.payload?.paths || [], at(e.payload?.position)); });
      listen("tauri://drag-enter", () => over?.(true));
      listen("tauri://drag-leave", () => over?.(false));
    },
    async refreshInfo() { Object.assign(info, await api("app_info")); return info; },
    library: {
      url: (rel) => libraryUrl + rel.split("/").map(encodeURIComponent).join("/"),
      pickFolder: (title) => invoke("pick_folder", { title }),
      pickFile: (title, extensions) => invoke("pick_file", { title, extensions }),
      pickFiles: (title) => invoke("pick_files", { title }),
      openPath: (path) => invoke("open_path", { path }),
    },
  };
}
