// Game Repo Go's frame: the bar at the top, the four pages (Library, Downloads, On this device,
// Settings) down the side on a PC and along the bottom on a phone, the notes every page
// shows (a removal cut short, ES-DE not found), the dialogs and the message at the bottom.
// In the big-screen view (D113, big.js) the pages are tabs along the top and the button
// hints run along the bottom.
import { html, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call } from "./backend.js";
import { closeDialog, go, open, refreshLists, refreshSettings, refreshStatus, refreshStorage, say } from "./state.js";
import { message } from "./format.js";
import { Dialog } from "./dialog.js";
import { PairDialog, PairPage } from "./pair.js";
import { LibraryPage } from "./library.js";
import { GamePage } from "./game.js";
import { AddDialog, AddMediaDialog } from "./add.js";
import { QueuePage, StartDialog } from "./queue.js";
import { DevicePage, PendingRemoval, RemoveDialog, RoomDialog } from "./device.js";
import { SettingsPage } from "./settings.js";
import { BigBar, BigGame, ChooseDialog, Hints, MediaDialog, OptionsDialog, Shelves, useFocus } from "./big.js";

const PAGES = [
  ["library", "Library", Icon.gamepad],
  ["queue", "Downloads", Icon.download],
  ["device", "On this device", Icon.monitor],
  ["settings", "Settings", Icon.cog],
];

function Mark() {
  return html`<svg class="brand-mark" viewBox="0 0 24 24" aria-hidden="true"><g data-sk="classic"><path d="M3 7a1.5 1.5 0 0 1 1.5-1.5h4.5l1.5 2h9a1.5 1.5 0 0 1 1.5 1.5v8.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z" /><path d="M6.6 12h1.6v-1.6h1.8v1.6h1.6v1.8h-1.6v1.6h-1.8v-1.6h-1.6z" class="accent" /><circle cx="15" cy="11.6" r="1.15" class="accent" /><circle cx="17.2" cy="14" r="1.15" class="accent" /></g></svg>`;
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
  const left = status?.queue?.left || status?.queue?.queued || 0;
  const here = route.page === "game" ? "library" : route.page;
  return html`<nav class="sidebar go-nav" aria-label="Pages">
    ${PAGES.map(([page, label, icon]) => html`<a class="go-nav-link" href=${`#${page}`} id=${`go-nav-${page}`} aria-current=${here === page ? "page" : null}>
      ${icon(18)}<span>${label}</span>${page === "queue" && left ? html`<span class="go-nav-n" aria-label=${status.queue.left ? `${left} to come` : `${left} queued`}>${left}</span>` : null}</a>`)}
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
    <p>This device forgets ${library || "the library"} and its list of games. The games on this device stay, and the library is told it can forget this device.</p>
  <//>`;
}

function Dialogs({ dialog }) {
  if (!dialog) return null;
  switch (dialog.type) {
    case "add": return html`<${AddDialog} keys=${dialog.keys} />`;
    case "start": return html`<${StartDialog} />`;
    case "add-media": return html`<${AddMediaDialog} row=${dialog.row} />`;
    case "remove": return html`<${RemoveDialog} asks=${dialog.asks} media=${dialog.media} title=${dialog.title} done=${dialog.done} />`;
    case "room": return html`<${RoomDialog} run=${dialog.run} />`;
    case "pair": return html`<${PairDialog} />`;
    case "unpair": return html`<${UnpairDialog} library=${dialog.library} />`;
    case "options": return html`<${OptionsDialog} />`;
    case "choose": return html`<${ChooseDialog} select=${dialog.select} />`;
    case "media": return html`<${MediaDialog} title=${dialog.title} media=${dialog.media} files=${dialog.files} />`;
    default: return null;
  }
}

function Page({ route, paired, big }) {
  if (!paired && route.page !== "settings") return html`<${PairPage} />`;
  switch (route.page) {
    case "game": return big ? html`<${BigGame} gameKey=${route.key} key=${route.key} />` : html`<${GamePage} gameKey=${route.key} key=${route.key} />`;
    case "library": return big ? html`<${Shelves} />` : html`<${LibraryPage} />`;
    case "queue": return html`<${QueuePage} />`;
    case "device": return html`<${DevicePage} />`;
    case "settings": return html`<${SettingsPage} />`;
    default: return html`<${LibraryPage} />`;
  }
}

/** Android: games can't be added until Game Repo Go may write ES-DE's folders (D87). */
function StorageNote() {
  const [busy, setBusy] = useState(false);
  const allow = async () => {
    setBusy(true);
    const s = await refreshStorage(true);
    setBusy(false);
    if (!s.granted) say("Game Repo Go still can't reach ES-DE's folders: turn on Allow access to manage all files for it.");
  };
  return html`<div class="go-note warn-note" id="go-storage" role="alert">
    <span>To put games in ES-DE's folders, Game Repo Go needs access to all files, as ES-DE has. You can browse without it.</span>
    <button type="button" class="primary" id="go-storage-allow" disabled=${busy} onClick=${allow}>Allow access</button></div>`;
}

/** The big-screen view's focus kept from page to page (big.js). */
function BigFocus({ route }) {
  useFocus(route);
  return null;
}

export function Shell() {
  const s = useStore(go, (x) => ({ ready: x.ready, route: x.route, status: x.status, storage: x.storage, dialog: x.dialog, toast: x.toast, view: x.view }));
  if (!s.ready) return html`<p class="go-opening muted">Opening Game Repo Go…</p>`;
  const st = s.status;
  const noEsde = st?.device?.error;
  const big = s.view === "big";
  return html`
    ${big ? html`<${BigBar} route=${s.route} status=${st} /><${BigFocus} route=${s.route} />` : html`<${Bar} status=${st} />`}
    <div class="go-body">
      ${big ? null : html`<${Nav} route=${s.route} status=${st} />`}
      <main class=${`go-main go-main-${s.route.page}`} id="go-main">
        ${st?.removal ? html`<${PendingRemoval} pending=${st.removal} />` : null}
        ${s.storage.needed && !s.storage.granted ? html`<${StorageNote} />` : null}
        ${noEsde && s.storage.granted && s.route.page !== "settings" ? html`<div class="go-note warn-note" id="go-no-esde" role="alert">
            <span>${noEsde}</span><a class="button ghost" href="#settings">Settings</a></div>` : null}
        <${Page} route=${s.route} paired=${!!st?.paired} big=${big} />
      </main>
    </div>
    ${big ? html`<${Hints} route=${s.route} dialog=${s.dialog} />` : null}
    <${Dialogs} dialog=${s.dialog} />
    ${s.toast ? html`<div class="toast go-toast" role="status" aria-live="polite" id="go-toast"><span>${s.toast.text}</span>
      ${s.toast.page ? html`<button type="button" class="ghost light" tabindex=${big ? "-1" : null} onClick=${() => { go.set({ toast: null }); open({ page: s.toast.page }); }}>${s.toast.page === "queue" ? "See Downloads" : s.toast.page === "settings" ? "Open Settings" : `See the ${s.toast.page}`}</button>` : null}</div>` : null}`;
}

