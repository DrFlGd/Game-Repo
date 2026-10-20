// Interface state shared by the islands (menu, main view, status bar).
// Preferences persist through the platform's store (the app keeps them on this
// computer, in prefs.json, under "gr-ui").
import { createStore } from "../lib/store.js";

export const ui = createStore({
  ready: false,
  route: "home",           // home | library | game | audit | bios | collections | layouts | pc | metadata | documents | settings | help | about
  library: null,           // the open library's info (path, id, name, format, read_only)
  libraryError: null,
  recent: [],              // libraries opened before (paths), newest first
  firstRun: false,         // no library was ever opened: ask where it goes
  index: null,             // index_info for the open library (on this computer)
  home: null,              // what Home shows: platform bars, the last scan, what needs attention
  roots: [],               // the folders with games
  dats: [],                // the imported DATs
  platforms: [],           // the platforms the app knows
  scanning: null,          // a scan's progress while one runs
  dialog: null,            // { type: "ask" | "confirm", ... }
  menu: null,              // a right-click or ⋯ menu: { x, y, items, from }
  jobs: [],                // [{ id, label, started, cancel }]
  changes: 0,              // counts changes made to files (organising, Undo), so pages showing them read again
  skin: "classic",
  theme: "system",
  navOpen: false,          // the menu as a drawer in a narrow window
  navCollapsed: false,     // the desktop menu folded to its icons
  sidePanel: null,         // the page's list beside the menu: "open" | "folded" (null: by the window's width)
  sidePanelWidth: 240,
  inspector: {},           // the inspector on the right, by page: "open" | "folded"
  inspectorTab: {},        // its tab shown, by page
  inspectorWidth: 360,
});

let prefs = null;
const LAYOUT_KEY = "gr-ui";

/** Load saved preferences (call once the platform store exists). */
export function initState(store) {
  prefs = store.prefs;
  const saved = prefs.get(LAYOUT_KEY, {}) || {};
  ui.set({ skin: SKINS.some(([v]) => v === saved.skin) ? saved.skin : "classic", theme: THEMES.some(([v]) => v === saved.theme) ? saved.theme : "system", navCollapsed: !!saved.navCollapsed,
    sidePanel: ["open", "folded"].includes(saved.sidePanel) ? saved.sidePanel : null, sidePanelWidth: Number(saved.sidePanelWidth) || 240,
    inspector: saved.inspector || {}, inspectorTab: saved.inspectorTab || {}, inspectorWidth: Number(saved.inspectorWidth) || 360 });
  applySkin();
  applyTheme();
  matchMedia("(prefers-color-scheme: dark)").addEventListener?.("change", applyTheme);
}

function savePrefs() {
  const s = ui.get();
  prefs?.set(LAYOUT_KEY, { skin: s.skin, theme: s.theme, navCollapsed: s.navCollapsed, sidePanel: s.sidePanel, sidePanelWidth: s.sidePanelWidth,
    inspector: s.inspector, inspectorTab: s.inspectorTab, inspectorWidth: s.inspectorWidth });
}

export function setPref(patch) {
  ui.set(patch);
  savePrefs();
}

/** The skins: a skin is data-skin on the page's root, read by styles.css (Classic,
 *  from Model Library), skins.css and the icons drawn for each skin; the first is the
 *  default, and one saved but no longer offered (Drafting Sheet, Clean Slate) falls
 *  back to it. [value, name, what it looks like]. */
export const SKINS = [
  ["classic", "Classic", "Plain and clear: slate and amber, rounded corners."],
  ["neon", "Neon City", "Cyberpunk: hot pink and cyan on near black, cut corners and glowing headings. Always dark."],
  ["terminal", "Wasteland Terminal", "An old green screen in a fallout shelter: phosphor text, scan lines, inverse video. Always dark; amber in the Night theme."],
  ["tome", "Adventurer's Tome", "A fantasy role-playing game: parchment in a leather binding, carved capitals, crimson and gold."],
  ["pixel", "Pixel Arcade", "An 8-bit game: chunky outlines, hard shadows, pixel lettering."],
  ["cosy", "Cosy Pastel", "A cosy life-sim: cream and lavender, sugared pinks and mint, soft round shapes."],
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
export function setTheme(theme) {
  setPref({ theme });
  applyTheme();
}

let jobSeq = 0;
/** Background work shown in the status bar. Returns a function that removes it;
 *  its .update(label) changes the text shown. */
export function addJob(label, cancel, open) {
  const job = { id: ++jobSeq, label, started: Date.now(), cancel, open };
  ui.set((s) => ({ jobs: [...s.jobs, job] }));
  const done = () => ui.set((s) => ({ jobs: s.jobs.filter((j) => j.id !== job.id) }));
  done.update = (text) => ui.set((s) => ({ jobs: s.jobs.map((j) => (j.id === job.id ? { ...j, label: text } : j)) }));
  return done;
}
