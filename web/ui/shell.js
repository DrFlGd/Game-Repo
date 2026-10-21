// Mounts the interface islands into the page: the menu, the main view for the
// current route (each page starts with its command bar), the status bar, dialogs
// and menus.
import { html, render } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, initState } from "./state.js";
import { setContext, ctx } from "./context.js";
import { Sidebar } from "./sidebar.js";
import { StatusBar } from "./chrome.js";
import { Home } from "./home.js";
import { Dialogs } from "./dialogs.js";
import { Settings } from "./settings.js";
import { About } from "./about.js";
import { Help } from "./help.js";
import { Coming, COMING } from "./coming.js";
import { Audit } from "./audit.js";
import { Collections } from "./collections.js";
import { PcGames } from "./pcgames.js";
import { Metadata } from "./metadata.js";
import { Documents } from "./documents.js";
import { Magazines } from "./magazines.js";
import { Bios } from "./bios.js";
import { Layouts } from "./layouts.js";
import { Browse } from "./browse.js";
import { GamePage } from "./gamepage.js";
import { ImportPage, dropToImport } from "./importpage.js";
import { listenForDrops } from "./documents.js";
import { History } from "./history.js";
import { loadIndex } from "./library.js";
import { ContextMenu, onWindowKey } from "./actions.js";
import { startUpdates } from "./updates.js";

function Main() {
  const route = useStore(ui, (s) => s.route);
  const blocked = useStore(ui, (s) => s.firstRun && !s.library);
  if (route === "settings" && !blocked) return html`<${Settings} />`;
  if (route === "about") return html`<${About} />`;
  if (route === "help") return html`<${Help} />`;
  if (route === "library" && !blocked) return html`<${Browse} />`;
  if (route === "game" && !blocked) return html`<${GamePage} />`;
  if (route === "audit" && !blocked) return html`<${Audit} />`;
  if (route === "collections" && !blocked) return html`<${Collections} />`;
  if (route === "pc" && !blocked) return html`<${PcGames} />`;
  if (route === "metadata" && !blocked) return html`<${Metadata} />`;
  if (route === "documents" && !blocked) return html`<${Documents} />`;
  if (route === "magazines" && !blocked) return html`<${Magazines} />`;
  if (route === "bios" && !blocked) return html`<${Bios} />`;
  if (route === "import" && !blocked) return html`<${ImportPage} />`;
  if (route === "history" && !blocked) return html`<${History} />`;
  if (route === "layouts" && !blocked) return html`<${Layouts} />`;
  if (COMING[route] && !blocked) return html`<${Coming} key=${route} page=${route} />`;
  return html`<${Home} />`;
}

export function mountShell(context) {
  setContext(context);
  window.__gamerepo = ctx; // for tests
  initState(context.platform.store);
  const info = context.platform.info || {};
  ui.set({ library: info.library || null, libraryError: info.library_error || null, recent: info.recent_libraries || [], firstRun: !!info.first_run });
  const mount = (id, C) => { const el = document.getElementById(id); if (el) render(html`<${C} />`, el); };
  mount("sidebar-root", Sidebar);
  mount("main-root", Main);
  mount("statusbar", StatusBar);
  mount("dialog-root", Dialogs);
  mount("menu-root", ContextMenu);
  addEventListener("keydown", onWindowKey);
  // files dropped where no page takes them go to the Import page (D78)
  listenForDrops(dropToImport);
  loadIndex();
  // what the check made as the app started found (the status bar says when there's a newer version)
  if (context.platform.kind === "desktop") startUpdates(context.platform.api).catch(() => {});
}

export { ui };
