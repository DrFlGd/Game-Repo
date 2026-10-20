// How Game Repo Go's pages reach its backend (gamerepo_device::app, behind
// desktop/client/src/lib.rs): its commands through Tauri's "api" command, its changes as
// "go-changed" events (queue, sync, device, settings), and pictures through the app's own
// protocol, from the device's cache or the library. In the page test, tests/tauri_shim.js
// stands in for Tauri and `gamerepo-cli client-ui` for the app.

const tauri = globalThis.__TAURI__;

/** Opened as a plain web page: there's no backend. */
export const inApp = !!globalThis.__TAURI_INTERNALS__ && !!tauri;

/** One of the backend's commands (app.rs `App::call`), answered as JSON; a refusal
 *  throws an Error with the backend's words. */
export async function call(cmd, args = {}) {
  try {
    return await tauri.core.invoke("api", { cmd, args });
  } catch (e) {
    throw new Error(typeof e === "string" ? e : e?.message || String(e));
  }
}

/** The shell's own: its kind (windows, android, linux), version, where pictures come from,
 *  and whether it's in the Steam Deck's Game Mode. */
export let info = { kind: "", version: "", picture_url: "picture://localhost/", game_mode: false };

export async function start() {
  info = { ...info, ...(await tauri.core.invoke("app_info")) };
  return info;
}

/** `fn(what)` each time the backend says something changed. */
export function onChange(fn) {
  tauri.event?.listen?.("go-changed", (e) => fn(e.payload))?.catch?.(() => {});
}

/** A folder chosen with the system's dialog; null if cancelled. */
export const pickFolder = (title) => tauri.core.invoke("pick_folder", { title });

/** Whether Game Repo Go may write ES-DE's folders: on Android, All files access (D87), and
 *  with `ask` Android's screen for it first; elsewhere always { needed: false }. */
export const storage = (ask = false) => tauri.core.invoke("storage", { ask });

/** Game Repo Go to the background (Android's Back with nothing to go back from). */
export const leave = () => tauri.core.invoke("leave");

/** Steam's on-screen keyboard (the Steam Deck's Game Mode, where nothing else would bring
 *  one up for a text box). */
export const keyboard = () => tauri.core.invoke("keyboard").catch(() => {});

/** `fn()` on Android's Back (its button or gesture) instead of Android's own. */
export function onBack(fn) {
  tauri.app?.onBackButtonPress?.(fn)?.catch?.(() => {});
}

/** A game's picture, at least `size` pixels on its longest side. */
export const pictureUrl = (key, kind = "cover", size = 256) =>
  `${info.picture_url}${key.split("/").map(encodeURIComponent).join("/")}/${encodeURIComponent(kind)}?size=${size}`;
