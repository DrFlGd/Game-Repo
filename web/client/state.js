// Game Repo Go's page state: where it is (the address's #page), what the backend last said
// (status, settings, the library's systems and collections), the Library's place, search
// and ticks, and the dialog and message showing. The backend's changes ("go-changed")
// bring the parts they touch up to date.
import { createStore } from "../lib/store.js";
import { SKINS, THEMES } from "../ui/state.js";
import { call, onChange, storage } from "./backend.js";

export const go = createStore({
  ready: false,
  route: { page: "library" },  // library | game (key) | queue | device | settings
  status: null,                // `status`: pairing, ES-DE's folders, the queue's and sync's progress
  storage: { needed: false, granted: true },  // Android's All files access (D87)
  settings: null,
  systems: [],                 // the library's systems shown (with ES-DE's folder on the device unless Show every system)
  collections: [],
  where: { system: "", collection: "", group: null },  // the Library's place ("" and "": every game)
  q: "",
  sort: "title",
  desc: false,
  picked: {},                  // the games ticked in the Library: key -> { title, size, system }
  libraryRev: 0,               // counts the catalogue's changes, so lists read again
  deviceRev: 0,                // counts the device's changes (On this device)
  queueRev: 0,
  roomFor: 0,                  // Make room: the bytes the Library's ticks need
  dialog: null,                // { type: "add" | "remove" | "pair" | …, … }
  toast: null,                 // { text, page? }
});

// ---- the address: #library, #game/<key>, #queue, #device, #settings -------------------

export function routeOf(hash) {
  const h = decodeURIComponent((hash || "").replace(/^#\/?/, ""));
  if (h.startsWith("game/")) return { page: "game", key: h.slice(5) };
  return { page: ["queue", "device", "settings"].includes(h) ? h : "library" };
}

export function open(route) {
  const hash = route.page === "game" ? `#game/${encodeURIComponent(route.key)}` : `#${route.page}`;
  if (location.hash !== hash) location.hash = hash;
  else go.set({ route: routeOf(hash) });
}

/** Back where it came from (the game page's Back, Esc, a controller's B). */
export function back() {
  if (go.get().route.page === "library") return;
  if (history.length > 1) history.back();
  else open({ page: "library" });
}

// ---- messages and dialogs ------------------------------------------------------------

let toastTimer = null;
/** A short message at the bottom; `page` adds a link to it ("See the queue"). */
export function say(text, page = null) {
  clearTimeout(toastTimer);
  go.set({ toast: { text, page } });
  toastTimer = setTimeout(() => go.set({ toast: null }), 6000);
}

export const showDialog = (dialog) => go.set({ dialog });
export const closeDialog = () => go.set({ dialog: null });

// ---- what the backend says -------------------------------------------------------------

export async function refreshStatus() {
  const status = await call("status").catch(() => null);
  if (status) go.set({ status });
  return status;
}

export async function refreshLists() {
  const [systems, collections] = await Promise.all([call("systems").catch(() => []), call("collections").catch(() => [])]);
  go.set((s) => ({ systems, collections, libraryRev: s.libraryRev + 1 }));
}

/** Android's All files access; with `ask`, its screen first (answered on coming back). */
export async function refreshStorage(ask = false) {
  const before = go.get().storage;
  const s = await storage(ask).catch(() => null);
  if (!s) return before;
  go.set({ storage: s });
  if (s.granted && !before.granted) refreshStatus();
  return s;
}

export async function refreshSettings() {
  const settings = await call("settings").catch(() => null);
  if (settings) go.set({ settings });
}

/** A call made at most once per `ms`, the last one always made. */
function throttle(fn, ms) {
  let timer = null, again = false;
  return () => {
    if (timer) { again = true; return; }
    fn();
    timer = setTimeout(function next() { timer = null; if (again) { again = false; fn(); timer = setTimeout(next, ms); } }, ms);
  };
}

const statusSoon = throttle(refreshStatus, 300);
const listsSoon = throttle(refreshLists, 2000);

export async function load() {
  await Promise.all([refreshStorage(), refreshStatus(), refreshSettings(), refreshLists()]);
  // allowed in Android's Settings while Game Repo Go was in the background
  document.addEventListener("visibilitychange", () => { if (!document.hidden && !go.get().storage.granted) refreshStorage(); });
  let syncing = !!go.get().status?.sync?.running;
  onChange(async (what) => {
    if (what === "sync") {
      const st = await refreshStatus();
      const now = !!st?.sync?.running;
      // the lists read again as the catalogue fills, and when it's done
      if (syncing && !now) refreshLists();
      else if (now) listsSoon();
      syncing = now;
    } else if (what === "queue") {
      statusSoon();
      go.set((s) => ({ queueRev: s.queueRev + 1 }));
    } else if (what === "device") {
      statusSoon();
      go.set((s) => ({ deviceRev: s.deviceRev + 1 }));
    } else if (what === "settings") {
      await refreshSettings();
      refreshLists();
      statusSoon();
    }
  });
  // the library brought up to date each time Game Repo Go opens (D98)
  if (go.get().status?.paired) call("sync").catch(() => {});
}

// ---- the look: Game Repo's skins and themes, kept on this device ------------------------

const LOOK = "go-look";

export function look() {
  let saved = {};
  try { saved = JSON.parse(localStorage.getItem(LOOK) || "{}") || {}; } catch { /* no storage: the defaults */ }
  return {
    // the first is the default, and one no longer offered falls back to it
    skin: SKINS.some(([v]) => v === saved.skin) ? saved.skin : SKINS[0][0],
    theme: THEMES.some(([v]) => v === saved.theme) ? saved.theme : "system",
  };
}

export function applyLook(next = look()) {
  const root = document.documentElement;
  root.dataset.skin = next.skin;
  root.dataset.theme = next.theme === "system" ? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : next.theme;
}

export function setLook(patch) {
  const next = { ...look(), ...patch };
  try { localStorage.setItem(LOOK, JSON.stringify(next)); } catch { /* kept for now only */ }
  applyLook(next);
}

matchMedia("(prefers-color-scheme: dark)").addEventListener?.("change", () => applyLook());
