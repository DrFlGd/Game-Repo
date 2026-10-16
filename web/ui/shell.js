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
import { Coming, COMING } from "./coming.js";
import { loadIndex } from "./library.js";
import { ContextMenu, onWindowKey } from "./actions.js";

function Main() {
  const route = useStore(ui, (s) => s.route);
  const blocked = useStore(ui, (s) => s.firstRun && !s.library);
  if (route === "settings" && !blocked) return html`<${Settings} />`;
  if (route === "about") return html`<${About} />`;
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
  loadIndex();
}

export { ui };
