// Game Repo Go (docs/PLAN.md, "Phase 9b design"): start-up. The pages are in client/
// (Preact + htm, no build step, with Game Repo's styles, skins and icons); the backend is
// reached through client/backend.js.
import { html, render } from "./lib/html.js";
import { call, inApp, leave, onBack, start } from "./client/backend.js";
import { applyLook, back, closeDialog, go, load, routeOf, say } from "./client/state.js";
import { startUpdates } from "./ui/updates.js";
import { gamepads, keys } from "./client/keys.js";
import { Shell } from "./client/shell.js";
import { choose, options, page, search } from "./client/big.js";

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
  // the view's known once the shell has said what it is (big screen on Android and in Game Mode)
  applyLook();
  const route = () => { if (go.get().dialog) closeDialog(); go.set({ route: routeOf(location.hash) }); };
  route();
  addEventListener("hashchange", () => { route(); document.getElementById("go-main")?.scrollTo?.(0, 0); });
  keys(() => (go.get().dialog ? closeDialog() : back()), { page, search, options, choose });
  // Android's Back as Esc: a dialog closes, a page goes back, and on the Library with
  // nothing to go back from Game Repo Go goes to the background
  onBack(() => {
    if (go.get().dialog) closeDialog();
    else if (go.get().route.page !== "library") back();
    else leave().catch(() => {});
  });
  gamepads();
  await load();
  go.set({ ready: true });
  // a newer Game Repo Go, found by the check it makes as it starts (D134)
  startUpdates(call).then((s) => { if (s?.available) say(`Game Repo Go ${s.available.version} is out: update it in Settings.`, "settings"); }).catch(() => {});
}

main().catch((e) => {
  window.__errors.push(String(e?.message || e));
  root.textContent = `Game Repo Go couldn't start: ${e?.message || e}`;
});
