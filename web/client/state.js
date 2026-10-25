// Game Repo Go's page state: where it is (the address's #page), what the backend last said
// (status, settings, the library's systems and collections), the Library's place, search
// and what's selected, and the dialog and message showing. The backend's changes ("go-changed")
// bring the parts they touch up to date.
import { createStore } from "../lib/store.js";
import { SKINS, THEMES } from "../ui/state.js";
import { call, info, onChange, storage } from "./backend.js";
import { refreshUpdates } from "../ui/updates.js";

export const go = createStore({
  ready: false,
  route: { page: "library" },  // library | games (big screen: a place's games) | game (key) | queue | device | settings
  view: "classic",             // big (D113) | classic, from look()
  status: null,                // `status`: pairing, ES-DE's folders, the queue's and sync's progress
  storage: { needed: false, granted: true },  // Android's All files access (D87)
  settings: null,
  systems: [],                 // the library's systems shown (with ES-DE's folder on the device unless Show every system)
  collections: [],
  where: { system: "", collection: "", group: null },  // the Library's place ("" and "": every game)
  q: "",
  sort: "title",
  desc: false,
  picked: {},                  // the games selected in the Library: key -> { title, size, system }
  libraryRev: 0,               // counts the catalogue's changes, so lists read again
  deviceRev: 0,                // counts the device's changes (On this device)
  queueRev: 0,
  tailscaleRev: 0,             // counts what Tailscale's doing changing while a set-up runs (D179)
  deviceSort: "system",        // On this device's order (D115)
  roomFor: 0,                  // Make room: the bytes the Library's selected games need
  dialog: null,                // { type: "add" | "remove" | "pair" | …, … }
  toast: null,                 // { text, page? }
});

// ---- the address: #library, #games, #game/<key>, #queue, #device, #settings ------------

export function routeOf(hash) {
  const h = decodeURIComponent((hash || "").replace(/^#\/?/, ""));
  if (h.startsWith("game/")) return { page: "game", key: h.slice(5) };
  return { page: ["games", "queue", "device", "settings"].includes(h) ? h : "library" };
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

// the control a dialog was opened from, focused again once it closes (a controller would
// otherwise be left with nothing focused)
let opener = null;
export function showDialog(dialog) {
  if (!go.get().dialog) opener = document.activeElement;
  go.set({ dialog });
}
export function closeDialog() {
  const was = opener;
  opener = null;
  go.set({ dialog: null });
  requestAnimationFrame(() => {
    if (was?.isConnected && (!document.activeElement || document.activeElement === document.body)) was.focus({ preventScroll: true });
  });
}

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
  onChange(async (what) => {
    if (what === "sync") {
      const st = await refreshStatus();
      // the lists read again as the catalogue fills, and whenever it's not filling: the
      // status may only be asked after a short read has finished (a slow phone), so every
      // change then may be the last
      if (st?.sync?.running) listsSoon();
      else refreshLists();
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
    } else if (what === "update") {
      refreshUpdates();
    } else if (what === "tailscale") {
      go.set((s) => ({ tailscaleRev: s.tailscaleRev + 1 }));
    }
  });
  // the library brought up to date each time Game Repo Go opens (D98)
  if (go.get().status?.paired) call("sync").catch(() => {});
}

// ---- the look: Game Repo's skins and themes, and the view, kept on this device -----------

const LOOK = "go-look";

/** The view a device starts in (D113): big screen on Android and in Steam's Game Mode or
 *  Big Picture, classic elsewhere. Known once the shell has said what it is. */
export const usualView = () => (info.kind === "android" || info.game_mode ? "big" : "classic");

export function look() {
  let saved = {};
  try { saved = JSON.parse(localStorage.getItem(LOOK) || "{}") || {}; } catch { /* no storage: the defaults */ }
  return {
    // the first is the default, and one no longer offered falls back to it
    skin: SKINS.some(([v]) => v === saved.skin) ? saved.skin : SKINS[0][0],
    theme: THEMES.some(([v]) => v === saved.theme) ? saved.theme : "system",
    // "" until one is chosen in Settings: the usual one for the device
    view: ["big", "classic"].includes(saved.view) ? saved.view : "",
    // the big screen's size (D132): "" as it is, else smaller or larger
    size: ["smaller", "larger"].includes(saved.size) ? saved.size : "",
  };
}

export function applyLook(next = look()) {
  const root = document.documentElement;
  const view = next.view || usualView();
  root.dataset.skin = next.skin;
  // the big screen is dark unless a theme is chosen (D113)
  root.dataset.theme = next.theme === "system" ? (view === "big" || matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light") : next.theme;
  root.dataset.view = view;
  if (next.size) root.dataset.size = next.size;
  else delete root.dataset.size;
  if (go.get().view !== view) go.set({ view });
}

export function setLook(patch) {
  const next = { ...look(), ...patch };
  try { localStorage.setItem(LOOK, JSON.stringify(next)); } catch { /* kept for now only */ }
  applyLook(next);
}

matchMedia("(prefers-color-scheme: dark)").addEventListener?.("change", () => applyLook());
