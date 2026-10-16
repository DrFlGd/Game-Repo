// Interface state shared by the islands (menu, main view, status bar).
// Preferences persist through the platform's store (the app keeps them on this
// computer, in prefs.json, under "gr-ui").
import { createStore } from "../lib/store.js";

export const ui = createStore({
  ready: false,
  route: "home",           // home | audit | collections | import | scraper | settings | about
  library: null,           // the open library's info (path, id, name, format, read_only)
  libraryError: null,
  recent: [],              // libraries opened before (paths), newest first
  firstRun: false,         // no library was ever opened: ask where it goes
  index: null,             // index_info for the open library (on this computer)
  dialog: null,            // { type: "ask" | "confirm", ... }
  menu: null,              // a right-click or ⋯ menu: { x, y, items, from }
  jobs: [],                // [{ id, label, started, cancel }]
  skin: "drafting",
  theme: "system",
  navOpen: false,          // the menu as a drawer in a narrow window
  navCollapsed: false,     // the desktop menu folded to its icons
});

let prefs = null;
const LAYOUT_KEY = "gr-ui";

/** Load saved preferences (call once the platform store exists). */
export function initState(store) {
  prefs = store.prefs;
  const saved = prefs.get(LAYOUT_KEY, {}) || {};
  ui.set({ skin: SKINS.some(([v]) => v === saved.skin) ? saved.skin : "drafting", theme: THEMES.some(([v]) => v === saved.theme) ? saved.theme : "system", navCollapsed: !!saved.navCollapsed });
  applySkin();
  applyTheme();
  matchMedia("(prefers-color-scheme: dark)").addEventListener?.("change", applyTheme);
}

function savePrefs() {
  const s = ui.get();
  prefs?.set(LAYOUT_KEY, { skin: s.skin, theme: s.theme, navCollapsed: s.navCollapsed });
}

export function setPref(patch) {
  ui.set(patch);
  savePrefs();
}

/** The skins (from Model Library): a skin is data-skin on the page's root, read by
 *  styles.css and by the icons drawn for each skin. */
export const SKINS = [
  ["drafting", "Drafting Sheet"],
  ["classic", "Classic"],
];
export function applySkin() {
  document.documentElement.dataset.skin = (SKINS.find(([v]) => v === ui.get().skin) || SKINS[0])[0];
}
export function setSkin(skin) {
  setPref({ skin });
  applySkin();
}

export const THEMES = [["system", "Follow the system"], ["light", "Light"], ["dark", "Dark"], ["night", "Night (dim and warm)"]];

/** The theme in use: light, dark or night ("system" resolves to light or dark). */
export function resolvedTheme() {
  const t = ui.get().theme;
  if (t === "system") return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  return t;
}
export function applyTheme() {
  document.documentElement.dataset.theme = resolvedTheme();
  window.dispatchEvent(new CustomEvent("gr-theme"));
}
/** The menu's theme button goes through the same choices as Settings, in order. */
export const nextTheme = (theme) => THEMES[(THEMES.findIndex(([v]) => v === theme) + 1) % THEMES.length][0];
export function cycleTheme() {
  setTheme(nextTheme(ui.get().theme));
}
export function setTheme(theme) {
  setPref({ theme });
  applyTheme();
}

let jobSeq = 0;
/** Background work shown in the status bar. Returns a function that removes it;
 *  its .update(label) changes the text shown. */
export function addJob(label, cancel) {
  const job = { id: ++jobSeq, label, started: Date.now(), cancel };
  ui.set((s) => ({ jobs: [...s.jobs, job] }));
  const done = () => ui.set((s) => ({ jobs: s.jobs.filter((j) => j.id !== job.id) }));
  done.update = (text) => ui.set((s) => ({ jobs: s.jobs.map((j) => (j.id === job.id ? { ...j, label: text } : j)) }));
  return done;
}
