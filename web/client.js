// Game Repo Go (docs/PLAN.md, "Phase 9b design"): start-up. The pages are in client/
// (Preact + htm, no build step, with Game Repo's styles, skins and icons); the backend is
// reached through client/backend.js.
import { html, render } from "./lib/html.js";
import { inApp, leave, onBack, start } from "./client/backend.js";
import { applyLook, back, go, load, routeOf } from "./client/state.js";
import { gamepads, keys } from "./client/keys.js";
import { Shell } from "./client/shell.js";

// errors the tests read back (tests/client_page.py)
window.__errors = [];
window.addEventListener("error", (e) => window.__errors.push(String(e.message)));
window.addEventListener("unhandledrejection", (e) => window.__errors.push(String(e.reason?.message || e.reason)));

applyLook();
const root = document.getElementById("go-root");

async function main() {
  if (!inApp) {
    root.innerHTML = '<p class="go-opening">Game Repo Go runs as an app on Windows and Android. Install it from Game Repo\'s releases.</p>';
    return;
  }
  render(html`<${Shell} />`, root);
  await start();
  const route = () => go.set({ route: routeOf(location.hash), dialog: null });
  route();
  addEventListener("hashchange", () => { route(); document.getElementById("go-main")?.scrollTo?.(0, 0); });
  keys(() => (go.get().dialog ? go.set({ dialog: null }) : back()));
  // Android's Back as Esc: a dialog closes, a page goes back, and on the Library with
  // nothing to go back from Game Repo Go goes to the background
  onBack(() => {
    if (go.get().dialog) go.set({ dialog: null });
    else if (go.get().route.page !== "library") back();
    else leave().catch(() => {});
  });
  gamepads();
  await load();
  go.set({ ready: true });
}

main().catch((e) => {
  window.__errors.push(String(e?.message || e));
  root.textContent = `Game Repo Go couldn't start: ${e?.message || e}`;
});
