// Home: the place to start. Each platform's completeness against its DAT, the
// last scan and what needs attention (docs/PLAN.md, 3.7), the index on this
// computer and what's coming. Also the first-start screen, which asks where the
// library should go.
import { html, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { isDesktop, openLibrary, showLibraryFolder, checkIndex, size, plural } from "./library.js";
import { refreshGames, scan, importDats, starterDats, repointRoot, addRoot } from "./games.js";
import { CommandBar } from "./layout.js";
import { NEXT } from "./about.js";
import { Icon } from "./icons.js";

/** The Library ⋯ menu: what's done to the whole library. */
function libraryItems() {
  return [
    ...(isDesktop() ? [{ id: "show-library", label: "Show in folder", icon: "folder", run: showLibraryFolder }] : []),
    { id: "scan-full", label: "Scan, unpacking ZIPs", icon: "refresh", title: "Read every file inside ZIPs in full this time", run: () => scan(true) },
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

const when = (iso) => (iso ? new Date(iso).toLocaleString() : "");

/** A bar per platform with a DAT: games had of the DAT's, and 1G1R groups. */
function PlatformsCard({ home }) {
  const bars = home.platforms || [];
  return html`<section class="home-card home-wide" id="home-platforms">
    <h2>Platforms</h2>
    ${bars.length ? html`<ul class="platform-bars">${bars.map((b) => html`<li key=${b.platform} data-platform=${b.platform}>
      <a class="pbar-name" href=${routeHash("audit")} onClick=${() => ctx.platform.store.prefs.set("gr-audit", { ...(ctx.platform.store.prefs.get("gr-audit", {}) || {}), platform: b.platform })}>${b.platform_name}</a>
      <span class="audit-bar" role="img" aria-label=${`${b.have} of ${b.games}`}><span style=${`width:${b.games ? (100 * b.have) / b.games : 0}%`}></span></span>
      <span class="pbar-count"><b>${b.have}</b> of ${b.games}</span>
      <span class="muted pbar-1g1r" title="1G1R: groups of a game and its clones with at least one of them">1G1R ${b.groups_have} of ${b.groups}</span>
      <span class="muted pbar-dat" title=${b.dat_name}>${b.dat_name}</span>
    </li>`)}</ul>` : html`<p class="muted" id="home-no-dats">${home.dats ? "No DAT is in use yet." : "Import the DATs for your systems to see what you have and what's missing."}</p>`}
    <div class="home-actions">
      ${isDesktop() ? html`<button type="button" class="ghost" id="home-import" onClick=${() => importDats()}>Import DATs…</button>` : null}
      ${bars.length ? null : html`<button type="button" class="primary" id="home-starter" onClick=${starterDats}>Get starter DATs…</button>`}
      <a class="ghost button-link" href=${routeHash("audit")}>Open the audit</a>
    </div>
  </section>`;
}

/** The last scan, and what needs attention: folders not found here, files no DAT or platform takes. */
function AttentionCard({ home }) {
  const last = home.last_scan;
  const missing = (home.roots || []).filter((r) => !r.found);
  const loose = home.loose || [];
  return html`<section class="home-card" id="home-scan">
    <h2>Folders with games</h2>
    ${(home.roots || []).length ? html`<p>${plural(home.roots.length, "folder", "folders")}, ${plural(home.files || 0, "file", "files")} found.
      ${last ? html` Last scan ${when(last.finished || last.started)}${last.stopped ? " (stopped)" : ""}.` : " Not scanned yet."}</p>`
      : html`<p>Add the folders your games are in, then scan them.${isDesktop() ? html` <button type="button" class="ghost" id="home-add-root" onClick=${() => addRoot()}>Add a folder…</button>` : null}</p>`}
    ${home.unhashed ? html`<p class="warn-note">${plural(home.unhashed, "file hasn't", "files haven't")} been read yet: scan to finish.</p>` : null}
    ${home.needs_tool ? html`<p class="warn-note" id="home-needs-tool">${plural(home.needs_tool, "RVZ file waits", "RVZ files wait")} for dolphin-tool: set where it is in <a href=${routeHash("settings")}>Settings › Tools</a>, then scan.</p>` : null}
    ${missing.map((r) => html`<p class="warn-note" key=${r.id}>${r.dir} isn't on this computer.
      ${isDesktop() ? html` <button type="button" class="ghost" onClick=${() => repointRoot(r)}>Point it elsewhere…</button>` : null}</p>`)}
    ${loose.length ? html`<h3>Without a DAT</h3><ul class="ls-list" id="home-loose">${loose.map((l) => html`<li key=${l.platform || "none"}>
      <span>${l.platform_name || "No platform worked out"}</span><span class="muted">${plural(l.files, "file", "files")}</span></li>`)}</ul>
      <small class="muted">Files whose platform has no DAT yet, or whose platform couldn't be worked out (set it on their folder in Settings).</small>` : null}
  </section>`;
}

function Coming() {
  return html`<section class="home-card" id="home-coming">
    <h2>Coming next</h2>
    <p>Game Repo is being built in phases; <a href=${routeHash("about")}>About</a> has the whole plan.</p>
    <ul class="ls-list">${NEXT.slice(0, 3).map(([phase, what]) => html`<li key=${phase}><span>${what}</span><span class="muted">${phase}</span></li>`)}</ul>
  </section>`;
}

export function Home() {
  const s = useStore(ui, (st) => ({ library: st.library, error: st.libraryError, firstRun: st.firstRun, index: st.index, home: st.home, scanning: st.scanning }));
  const lib = s.library;
  useEffect(() => { if (lib) refreshGames(); }, [lib?.path]);
  if (s.firstRun && !lib) return html`<${FirstRun} />`;
  if (!lib) {
    return html`<${CommandBar} id="home-head" title="Game Repo" /><div class="home">
      <p class="page-lead">Audit your ROM sets, build collections, bring in your PC games and keep each game's details in one library.</p>
      ${s.error ? html`<p class="warn-note" role="alert">${s.error}</p>` : null}
      ${isDesktop() ? html`<div class="home-actions"><button type="button" class="primary" id="home-open-library" onClick=${() => openLibrary()}>Open a library…</button></div>` : null}
    </div>`;
  }
  const sc = s.scanning;
  const scanLabel = sc ? (sc.items ? `Scanning ${sc.item} of ${sc.items}` : `${sc.text || "Scanning"}…`) : "Scan";
  return html`<${CommandBar} id="home-head" title=${lib.name}
      primary=${html`<button type="button" class="primary compact" id="home-scan-button" disabled=${!!sc || !!lib.read_only} title="Read what's new or changed in the folders with games"
        onClick=${() => scan()}>${Icon.refresh(15)} ${scanLabel}</button>`}
      more=${{ id: "library-menu", title: "Library: show in folder, check the index, open another library", items: libraryItems }} />
  <div class="home">
    <p class="page-lead">Your library is in <b>${lib.path}</b>. How its folders are laid out is in <a href="#/about" id="home-layout-link">About</a>.</p>
    ${s.error ? html`<p class="warn-note" role="alert">${s.error}</p>` : null}
    ${lib.read_only ? html`<p class="warn-note" role="alert">${lib.read_only}</p>` : null}
    ${s.home?.error ? html`<p class="warn-note" role="alert">${s.home.error}</p>` : null}
    <div class="home-grid">
      ${s.home && !s.home.error ? html`<${PlatformsCard} home=${s.home} /><${AttentionCard} home=${s.home} />` : null}
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
