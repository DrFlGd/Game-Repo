// Exit Game Repo Go (docs/PLAN.md, "Maps and Guides notes, part 11", D244): in the Steam
// Deck's Game Mode there's no window to close, so Settings and the big screen's Options (☰)
// have a button of their own, for a controller and a touch alike. It ends Game Repo Go as
// closing its window does: a download stops where it is and goes on when Game Repo Go opens
// again. When something's running (downloads, reading the library, a sync of saves) it asks
// first. Linux only: Windows has its window's close button, and Android ends apps itself.
import { html } from "../lib/html.js";
import { info, quit } from "./backend.js";
import { closeDialog, go, showDialog } from "./state.js";
import { Dialog } from "./dialog.js";
import { plural } from "./format.js";

/** Whether the Exit button is offered here: on Linux (the Steam Deck). */
export const exitOffered = () => info.kind === "linux";

/** What's running now that Exit would stop, in words; none: nothing is. */
export function running(status = go.get().status) {
  const q = status?.queue;
  return [
    q?.left > 0 && !q.offline ? `${plural(q.left, "download")} going` : null,
    status?.sync?.running ? "the library being read" : null,
    status?.checking?.running ? "games from elsewhere being checked" : null,
  ].filter(Boolean);
}

/** Exit pressed: ends Game Repo Go at once, or asks first when something's running. */
export function exitGo() {
  if (running().length) showDialog({ type: "exit" });
  else quit();
}

export function ExitDialog() {
  const now = running();
  return html`<${Dialog} title="Exit Game Repo Go?" id="exit-dialog" onSubmit=${() => { closeDialog(); quit(); }} submitLabel="Exit" cancelLabel="Not now">
    <p id="exit-running">${now.length ? `Still running: ${now.join(", ")}.` : "Nothing is running now."}</p>
    <p class="muted">Downloads stop where they are and carry on when you open Game Repo Go again.</p>
  <//>`;
}

/** The button, as Settings and the big screen's Options show it. */
export function ExitButton({ id = "go-exit", cls = "ghost" }) {
  if (!exitOffered()) return null;
  return html`<button type="button" class=${cls} id=${id} data-say="Ends Game Repo Go: Game Mode has no window to close" onClick=${exitGo}>Exit Game Repo Go</button>`;
}
