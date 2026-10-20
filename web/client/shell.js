// Game Repo Go's frame: the bar at the top, the four pages (Library, Queue, On this device,
// Settings) down the side on a PC and along the bottom on a phone, the notes every page
// shows (a removal cut short, ES-DE not found), the dialogs and the message at the bottom.
import { html, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call } from "./backend.js";
import { closeDialog, go, open, refreshLists, refreshSettings, refreshStatus, say } from "./state.js";
import { message } from "./format.js";
import { Dialog } from "./dialog.js";
import { PairDialog, PairPage } from "./pair.js";
import { LibraryPage } from "./library.js";
import { GamePage } from "./game.js";
import { AddDialog, AddMediaDialog } from "./add.js";
import { QueuePage } from "./queue.js";
import { DevicePage, PendingRemoval, RemoveDialog, RoomDialog } from "./device.js";
import { SettingsPage } from "./settings.js";

const PAGES = [
  ["library", "Library", Icon.gamepad],
  ["queue", "Queue", Icon.download],
  ["device", "On this device", Icon.monitor],
  ["settings", "Settings", Icon.cog],
];

function Mark() {
  return html`<svg class="brand-mark" viewBox="0 0 24 24" aria-hidden="true"><g data-sk="classic"><rect x="3" y="7" width="18" height="11" rx="3" /><path d="M8 11v3M6.5 12.5h3" /><circle cx="15.5" cy="11.5" r="1" class="accent" /><circle cx="17.5" cy="13.5" r="1" class="accent" /></g></svg>`;
}

function Bar({ status }) {
  const q = status?.queue;
  const busy = q && q.left > 0;
  const pct = busy && q.size ? Math.floor((q.done * 100) / q.size) : 0;
  return html`<header class="topbar go-bar">
    <a class="brand" href="#library">${Mark()}<span>Game Repo Go</span></a>
    ${status?.paired ? html`<span class="go-bar-library" id="go-bar-library">${status.library?.name || ""}</span>` : null}
    <span class="topbar-spacer"></span>
    ${busy ? html`<a class="go-bar-queue" href="#queue" id="go-bar-queue">${q.offline ? "Waiting for the library" : `Downloading ${q.left} · ${pct}%`}</a>` : null}
  </header>`;
}

function Nav({ route, status }) {
  const left = status?.queue?.left || 0;
  const here = route.page === "game" ? "library" : route.page;
  return html`<nav class="sidebar go-nav" aria-label="Pages">
    ${PAGES.map(([page, label, icon]) => html`<a class="go-nav-link" href=${`#${page}`} id=${`go-nav-${page}`} aria-current=${here === page ? "page" : null}>
      ${icon(18)}<span>${label}</span>${page === "queue" && left ? html`<span class="go-nav-n" aria-label=${`${left} to come`}>${left}</span>` : null}</a>`)}
  </nav>`;
}

function UnpairDialog({ library }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const unpair = async () => {
    setBusy(true);
    try {
      await call("unpair");
      await Promise.all([refreshStatus(), refreshSettings(), refreshLists()]);
      closeDialog();
      say("Unpaired. The games on this device stay.");
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title="Unpair from the library" id="unpair-dialog" onSubmit=${unpair} busy=${busy} error=${error} danger=${true} submitLabel="Unpair">
    <p>This device forgets ${library || "the library"} and its copy of the list. The games on this device stay, and the library is told it can forget this device.</p>
  <//>`;
}

function Dialogs({ dialog }) {
  if (!dialog) return null;
  switch (dialog.type) {
    case "add": return html`<${AddDialog} keys=${dialog.keys} />`;
    case "add-media": return html`<${AddMediaDialog} row=${dialog.row} />`;
    case "remove": return html`<${RemoveDialog} asks=${dialog.asks} media=${dialog.media} title=${dialog.title} done=${dialog.done} />`;
    case "room": return html`<${RoomDialog} run=${dialog.run} />`;
    case "pair": return html`<${PairDialog} />`;
    case "unpair": return html`<${UnpairDialog} library=${dialog.library} />`;
    default: return null;
  }
}

function Page({ route, paired }) {
  if (!paired && route.page !== "settings") return html`<${PairPage} />`;
  switch (route.page) {
    case "game": return html`<${GamePage} gameKey=${route.key} key=${route.key} />`;
    case "queue": return html`<${QueuePage} />`;
    case "device": return html`<${DevicePage} />`;
    case "settings": return html`<${SettingsPage} />`;
    default: return html`<${LibraryPage} />`;
  }
}

export function Shell() {
  const s = useStore(go, (x) => ({ ready: x.ready, route: x.route, status: x.status, dialog: x.dialog, toast: x.toast }));
  if (!s.ready) return html`<p class="go-opening muted">Opening Game Repo Go…</p>`;
  const st = s.status;
  const noEsde = st?.device?.error;
  return html`
    <${Bar} status=${st} />
    <div class="go-body">
      <${Nav} route=${s.route} status=${st} />
      <main class=${`go-main go-main-${s.route.page}`} id="go-main">
        ${st?.removal ? html`<${PendingRemoval} pending=${st.removal} />` : null}
        ${noEsde && s.route.page !== "settings" ? html`<div class="go-note warn-note" id="go-no-esde" role="alert">
            <span>${noEsde}</span><a class="button ghost" href="#settings">Settings</a></div>` : null}
        <${Page} route=${s.route} paired=${!!st?.paired} />
      </main>
    </div>
    <${Dialogs} dialog=${s.dialog} />
    ${s.toast ? html`<div class="toast go-toast" role="status" aria-live="polite" id="go-toast"><span>${s.toast.text}</span>
      ${s.toast.page ? html`<button type="button" class="ghost light" onClick=${() => { go.set({ toast: null }); open({ page: s.toast.page }); }}>See the ${s.toast.page}</button>` : null}</div>` : null}`;
}

