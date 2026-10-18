// History (D71): every change the app has made to your files, newest first, with Undo, and what's set aside until
// it's emptied. It was a section of Settings; the list itself is settings.js' HistorySection.
import { html } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { CommandBar } from "./layout.js";
import { HistorySection } from "./settings.js";

export function History() {
  const lib = useStore(ui, (s) => s.library);
  const head = html`<${CommandBar} id="history-head" title="History" />`;
  if (!lib) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  return html`${head}<div class="pages"><div class="library-settings" id="history-page"><${HistorySection} lib=${lib} key=${lib.path} /></div></div>`;
}
