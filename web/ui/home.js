// Home: the place to start. In Phase 0 it shows the open library, its index on
// this computer and what's coming; from Phase 1 each platform's completeness,
// recent jobs and what's waiting (docs/PLAN.md, 3.7). Also the first-start
// screen, which asks where the library should go.
import { html } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { isDesktop, openLibrary, showLibraryFolder, checkIndex, size, plural } from "./library.js";
import { CommandBar } from "./layout.js";
import { COMING } from "./coming.js";
import { Icon } from "./icons.js";

/** The Library ⋯ menu: what's done to the whole library. */
function libraryItems() {
  return [
    ...(isDesktop() ? [{ id: "show-library", label: "Show in folder", icon: "folder", run: showLibraryFolder }] : []),
    { id: "check-index", label: "Check the index", icon: "database", title: "Ask SQLite to check the index file on this computer", run: checkIndex },
    ...(isDesktop() ? [{ sep: true }, { id: "open-another", label: "Open another library…", icon: "folder", run: () => openLibrary() }] : []),
  ];
}

/** The index on this computer: what it holds so far. */
export function IndexCard({ index }) {
  if (!index) return html`<section class="home-card" id="home-index"><h2>Index</h2><p class="muted">Reading the index…</p></section>`;
  if (index.error) return html`<section class="home-card" id="home-index"><h2>Index</h2><p class="warn-note" role="alert">${index.error}</p></section>`;
  const tables = Object.entries(index.tables || {});
  return html`<section class="home-card" id="home-index">
    <h2>Index</h2>
    <p>What the app has worked out about your files is kept in an index on this computer, so a library of hundreds of thousands of ROMs opens quickly. It can always be made again from the library and your folders.</p>
    <dl class="about-versions">
      <div><dt>Size</dt><dd id="index-size">${size(index.size)}</dd></div>
      <div><dt>Tables</dt><dd id="index-tables">${tables.map(([t, n]) => `${t} (${plural(n, "row", "rows")})`).join(", ")}</dd></div>
      <div><dt>Schema</dt><dd id="index-version">step ${index.version} of ${index.schema_version}</dd></div>
    </dl>
  </section>`;
}

function Coming() {
  return html`<section class="home-card" id="home-coming">
    <h2>Coming next</h2>
    <p>Game Repo is being built in phases. This first version sets up the library; the pages in the menu say what each phase brings.</p>
    <ul class="ls-list">${Object.entries(COMING).map(([route, p]) => html`<li key=${route}><span class="home-cat">${Icon[p.icon](15)}<a href=${routeHash(route)}>${p.title}</a></span><span class="muted">Phase ${p.phase}</span></li>`)}</ul>
  </section>`;
}

export function Home() {
  const s = useStore(ui, (st) => ({ library: st.library, error: st.libraryError, firstRun: st.firstRun, index: st.index }));
  const lib = s.library;
  if (s.firstRun && !lib) return html`<${FirstRun} />`;
  if (!lib) {
    return html`<${CommandBar} id="home-head" title="Game Repo" /><div class="home">
      <p class="page-lead">Audit your ROM sets, build collections, bring in your PC games and keep each game's details in one library.</p>
      ${s.error ? html`<p class="warn-note" role="alert">${s.error}</p>` : null}
      ${isDesktop() ? html`<div class="home-actions"><button type="button" class="primary" id="home-open-library" onClick=${() => openLibrary()}>Open a library…</button></div>` : null}
    </div>`;
  }
  return html`<${CommandBar} id="home-head" title=${lib.name}
      more=${{ id: "library-menu", title: "Library: show in folder, check the index, open another library", items: libraryItems }} />
  <div class="home">
    <p class="page-lead">Your library is in <b>${lib.path}</b>. How its folders are laid out is in <a href="#/about" id="home-layout-link">About</a>.</p>
    ${s.error ? html`<p class="warn-note" role="alert">${s.error}</p>` : null}
    ${lib.read_only ? html`<p class="warn-note" role="alert">${lib.read_only}</p>` : null}
    <div class="home-grid">
      <${IndexCard} index=${s.index} />
      <${Coming} />
    </div>
  </div>`;
}

/** First start: no library was ever opened, so ask where it should go. */
function FirstRun() {
  const def = ctx.platform?.info?.default_library || "";
  return html`<div class="first-run" id="first-run">
    <h1>Where should your library go?</h1>
    <p>The library is an ordinary folder. The app keeps its own files in a <code>_library</code> folder inside it: each game's details, scraped pictures and your documents. Your ROMs can stay where they are, or be moved into the library when you choose.</p>
    <p>Choose an empty folder for a new library, a folder that already is one, or the folder your games are in: nothing in it is changed.</p>
    <div class="home-actions">
      ${def ? html`<button type="button" class="primary" id="use-default" onClick=${() => openLibrary(def)}>Use ${def}</button>` : null}
      <button type="button" class=${def ? "ghost" : "primary"} id="choose-library" onClick=${() => openLibrary()}>Choose a folder…</button>
    </div>
    <p class="muted">You can open a different library at any time in Settings.</p>
  </div>`;
}
