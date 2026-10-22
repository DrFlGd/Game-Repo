// Documents (docs/PLAN.md, "Phase 5 design", D30, D34): every guide, FAQ, manual, map,
// poster and scan in the library, searched by their words (the index reads the text of
// PDFs and text files), whatever game they're with. They're stored with their game
// (its folder's docs/, with docs.json for their kinds and titles), or in _library/docs/
// when they go with none. Import documents… reads files or a folder, pairs each with
// the game its name suggests, lets each pairing be changed, then brings them in as a
// change with a preview and Undo. The viewer shows PDFs, pictures, text and audio.
import { html, useState, useEffect, useRef } from "../lib/html.js";
import { useStore, createStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, SearchNote } from "./layout.js";
import { Dialog, More, close } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs } from "./organise.js";
import { Icon } from "./icons.js";
import { importFrom } from "./importpage.js";

const PAGE = 100;
const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const prefs = () => ctx.platform?.store?.prefs;

const SOURCE_NAMES = { screenscraper: "ScreenScraper", igdb: "IGDB", thegamesdb: "TheGamesDB", launchbox: "LaunchBox", steam: "Steam", gog: "GOG" };
/** A source's name, from its id. */
export const sourceName = (id) => SOURCE_NAMES[id] || id;

const PAIRING = {
  sure: ["Sure", "Its name is the game's title"],
  likely: ["Likely", "Its name is close to the game's title: check it"],
  none: ["No game", "Nothing matched: it's kept with the system's documents (or General)"],
  you: ["You chose", "Paired by you"],
};

/** A snippet with the words found marked ([ and ] around them), as nodes. */
function Snippet({ text }) {
  const parts = String(text || "").split(/(\[[^\]]*\])/g);
  return html`<span class="doc-snippet muted">${parts.map((p, i) => (p.startsWith("[") && p.endsWith("]") ? html`<mark key=${i}>${p.slice(1, -1)}</mark>` : p))}</span>`;
}

// ------------------------------------------------------------ the viewer

/** A PDF in the system's viewer (from the library's own address), with page and zoom
 *  (Model Library's document viewer, reading from the library only). The page (from 1) is
 *  the viewer's, so a magazine's text follows it. */
function PdfView({ url, title, page, setPage }) {
  const [zoom, setZoom] = useState("page-width");
  const move = (n) => setPage(Math.max(1, Math.min(99999, page + n)));
  const fragment = zoom === "fit-page" ? "view=Fit" : zoom === "page-width" ? "view=FitH" : `zoom=${zoom}`;
  const address = `${url}#toolbar=0&navpanes=0&page=${page}&${fragment}`;
  return html`<div class="pdf-viewer" aria-label="PDF document">
    <div class="pdf-controls" role="toolbar" aria-label="PDF controls">
      <button type="button" class="ghost" disabled=${page <= 1} aria-label="Previous page" onClick=${() => move(-1)}>‹</button>
      <label class="pdf-page-label">Page <input type="number" min="1" max="99999" aria-label="Page" value=${page} onChange=${(e) => setPage(Math.max(1, Math.min(99999, Number(e.target.value) || 1)))} /></label>
      <button type="button" class="ghost" aria-label="Next page" onClick=${() => move(1)}>›</button>
      <label class="pdf-zoom-label">Zoom
        <select aria-label="Zoom" value=${zoom} onChange=${(e) => setZoom(e.target.value)}>
          <option value="page-width">Fit width</option><option value="fit-page">Fit page</option>
          <option value="75">75%</option><option value="100">100%</option><option value="125">125%</option><option value="150">150%</option><option value="200">200%</option>
        </select></label>
    </div>
    <iframe class="doc-frame pdf-frame" key=${address} title=${title} src=${address}></iframe>
  </div>`;
}

/** A comic book archive (CBZ or CBR), a page at a time (D68); `page` from 1, as the PDF's. */
function ComicView({ path, title, page: shown, setPage: show }) {
  const [count, setCount] = useState(null);
  useEffect(() => {
    setCount(null);
    api("magazine_pages", { path }).then((r) => setCount(r.count)).catch(() => setCount(0));
  }, [path]);
  if (count === null) return html`<p class="muted">Opening…</p>`;
  if (!count) return html`<p class="muted" id="comic-empty">There are no pictures in this file.</p>`;
  const page = Math.max(0, Math.min(count - 1, shown - 1));
  const move = (n) => show(Math.max(0, Math.min(count - 1, page + n)) + 1);
  return html`<div class="pdf-viewer comic-viewer" id="comic-view" aria-label="Comic book" tabIndex="0"
      onKeyDown=${(e) => { if (e.key === "ArrowRight" || e.key === "PageDown") move(1); else if (e.key === "ArrowLeft" || e.key === "PageUp") move(-1); }}>
    <div class="pdf-controls" role="toolbar" aria-label="Page controls">
      <button type="button" class="ghost" id="comic-back" disabled=${page <= 0} aria-label="Previous page" onClick=${() => move(-1)}>‹</button>
      <span class="pdf-page-label" id="comic-page">Page ${page + 1} of ${count}</span>
      <button type="button" class="ghost" id="comic-next" disabled=${page >= count - 1} aria-label="Next page" onClick=${() => move(1)}>›</button>
    </div>
    <div class="comic-page"><img key=${page} src=${libraryUrl(`~page/${page}/${path}`)} alt=${`${title}, page ${page + 1}`} onClick=${() => move(1)} /></div>
  </div>`;
}

/** Words to look for, as the text is compared: lower case, without accents. */
const plain = (s) => String(s || "").normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
const wordsOf = (s) => plain(s).split(/[^\p{L}\p{N}]+/u).filter((w) => w.length > 1);

/** A magazine page's text (D145), with the words looked for marked. */
function PageText({ text, page, find }) {
  const t = text.pages?.[page - 1];
  if (t === undefined) return html`<div class="doc-page-text" id="doc-page-text"><p class="muted">There's no page ${page}.</p></div>`;
  if (!t.trim()) {
    const unread = (text.unread || []).includes(page);
    return html`<div class="doc-page-text" id="doc-page-text"><p class="muted">${unread ? `Page ${page}'s text couldn't be read.${text.note ? ` ${text.note}` : ""}` : `There's no text on page ${page}.`}</p></div>`;
  }
  const want = wordsOf(find);
  const parts = want.length ? t.split(/([\p{L}\p{N}]+)/u) : [t];
  return html`<div class="doc-page-text" id="doc-page-text" aria-label=${`Page ${page}'s text`}>${parts.map((p, i) => (want.length && want.some((w) => plain(p).startsWith(w)) ? html`<mark key=${i}>${p}</mark>` : p))}</div>`;
}

/** One document: its kind and title (changed here), and the document itself; `page` to
 *  open it at (from 1), and `find`: words a magazine's page text has marked, its first
 *  page with them all opened when no page is given. */
function DocView({ id, off, onChange, onClose, page: start, find }) {
  const [d, setD] = useState(null);
  const [error, setError] = useState("");
  const [page, setPage] = useState(start || 1);
  const [text, setText] = useState(null);
  const [showText, setShowText] = useState(!!find);
  const kinds = useStore(docsState, (s) => s.kinds);
  useEffect(() => {
    setD(null);
    setError("");
    setText(null);
    setPage(start || 1);
    api("docs_get", { id }).then((x) => {
      setD(x);
      // a magazine's issue: its text, as it was read (D145)
      if (x.path.startsWith("Magazines/")) {
        api("magazine_text", { path: x.path }).then((t) => {
          if (!t) return;
          setText(t);
          const want = wordsOf(find);
          if (!start && want.length) {
            const at = t.pages.findIndex((p) => { const w = plain(p); return want.every((x) => w.includes(x)); });
            if (at >= 0) setPage(at + 1);
          }
        }).catch(() => {});
      }
    }).catch((e) => setError(err(e)));
  }, [id]);
  const set = async (patch) => {
    try {
      setD({ ...(await api("docs_set", { id, ...patch })), html: d.html, full: d.full });
      onChange?.();
    } catch (e) {
      setError(err(e));
    }
  };
  if (error) return html`<aside class="pc-panel doc-panel" id="doc-view"><p class="warn-note" role="alert">${error}</p></aside>`;
  if (!d) return html`<aside class="pc-panel doc-panel" id="doc-view"><p class="muted">Opening…</p></aside>`;
  const url = libraryUrl(d.path);
  // a magazine's issue is named and changed on the Magazines page
  const magazine = d.path.startsWith("Magazines/");
  return html`<aside class="pc-panel doc-panel" id="doc-view" data-id=${d.id} data-view=${d.view}>
    <div class="doc-head">
      <div class="doc-head-main"><h2 id="doc-title">${d.title}</h2>
        <p class="muted">${[d.kind_name, d.game || null, d.platform_name || null, d.source ? `from ${sourceName(d.source)}` : "yours", size(d.size)].filter(Boolean).join(" · ")}</p></div>
      ${onClose ? html`<button type="button" class="ghost" aria-label="Close" title="Close" onClick=${onClose}>${Icon.close(15)}</button>` : null}
    </div>
    <div class="pick-row doc-edit">
      <input type="text" class="compact" id="doc-edit-title" aria-label="Title" value=${d.title} disabled=${off || magazine} title=${magazine ? "A magazine's issue is changed on the Magazines page" : null} onChange=${(e) => e.target.value.trim() && set({ title: e.target.value.trim() })} />
      <select class="compact" id="doc-edit-kind" aria-label="Kind" value=${d.kind} disabled=${off || magazine} onChange=${(e) => set({ kind: e.target.value })}>
        ${(kinds || []).map((k) => html`<option key=${k.id} value=${k.id}>${k.name}</option>`)}
      </select>
      ${isDesktop() ? html`<button type="button" class="ghost compact" id="doc-open" title="Open it in its own program" onClick=${() => ctx.platform.library.openPath(d.full)}>${Icon.external(14)} Open</button>
        <button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => reveal(d.full)}>${Icon.folder(14)}</button>` : null}
      ${text ? html`<button type="button" class="ghost compact" id="doc-text-toggle" aria-pressed=${showText ? "true" : "false"} title="The page's text, as it was read" onClick=${() => setShowText(!showText)}>${Icon.search(14)} Text</button>` : null}
    </div>
    ${text && showText ? html`<${PageText} text=${text} page=${page} find=${find} />` : null}
    <div class="doc-body">
      ${d.view === "pdf" ? html`<${PdfView} url=${url} title=${d.title} page=${page} setPage=${setPage} />`
        : d.view === "comic" ? html`<${ComicView} path=${d.path} title=${d.title} page=${page} setPage=${setPage} />`
        : d.view === "image" ? html`<img class="doc-image" src=${url} alt=${d.title} />`
        : d.view === "audio" ? html`<audio class="doc-audio" src=${url} controls preload="metadata"></audio>`
        : d.view === "text" || d.view === "markdown" ? html`<div class="doc-text-view" id="doc-text" dangerouslySetInnerHTML=${{ __html: d.html || "" }}></div>`
        : html`<p class="muted" id="doc-outside">This kind of file (${d.name.split(".").pop().toUpperCase()}) opens in its own program.${isDesktop() ? " Press Open." : ""}</p>`}
    </div>
  </aside>`;
}

/** The kinds of document, as the index names them (for the viewer's menu). */
const docsState = createStore({ kinds: [] });
const loadKinds = (kinds) => { if (kinds?.length) docsState.set({ kinds: kinds.map(({ id, name }) => ({ id, name })) }); };

function DocDialog({ id, page, find }) {
  useEffect(() => { if (!docsState.get().kinds.length) api("docs_list", { limit: 1, offset: 1 }).then((r) => loadKinds(r.kinds)).catch(() => {}); }, []);
  // Esc closes it, and so does going to another page (review M36); a field being edited
  // takes its own Esc first
  useEffect(() => {
    const key = (e) => { if (e.key === "Escape" && !e.defaultPrevented && !e.target.closest?.("input, select, textarea")) close(); };
    const page = location.hash.replace(/^#\/?/, "").split(/[/?]/)[0];
    const moved = () => { if (location.hash.replace(/^#\/?/, "").split(/[/?]/)[0] !== page) close(); };
    addEventListener("keydown", key);
    addEventListener("hashchange", moved);
    return () => { removeEventListener("keydown", key); removeEventListener("hashchange", moved); };
  }, []);
  const off = !!ui.get().library?.read_only;
  return html`<div class="dialog-backdrop" onMouseDown=${(e) => { if (e.target === e.currentTarget) close(); }}>
    <div class="dialog wide doc-dialog" id="doc-dialog" role="dialog" aria-modal="true" aria-label="Document">
      <${DocView} id=${id} off=${off} onChange=${again} onClose=${close} page=${page} find=${find} />
    </div></div>`;
}

/** Show a document over the page: at `page` (from 1), with `find`'s words marked in a
 *  magazine page's text. */
export const viewDoc = (id, { page = null, find = "" } = {}) => ui.set({ dialog: { type: "doc", id, page, find } });

// ------------------------------------------------------------ Import documents…

/** Pick the game a document goes with: a system, a search of its DAT's games, or no game. */
function GamePick({ it, platforms, onPick, onCancel }) {
  const [platform, setPlatform] = useState(it.platform || platforms[0]?.id || "");
  const [q, setQ] = useState(it.title);
  const [found, setFound] = useState(null);
  useEffect(() => {
    if (!platform) return;
    let gone = false;
    const t = setTimeout(() => api("meta_games", { platform, query: q }).then((r) => { if (!gone) setFound(r); }).catch(() => { if (!gone) setFound([]); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [platform, q]);
  const others = (it.others || []).filter((o) => o.id !== it.game);
  return html`<div class="game-pick" data-for=${it.path}>
    ${others.length ? html`<div class="game-pick-others"><span class="muted">It could also be:</span>
      ${others.map((o) => html`<button type="button" class="ghost compact" key=${o.id} data-game=${o.id} onClick=${() => onPick({ id: o.id, name: o.name, title: o.name.replace(/\s*[([].*$/, ""), platform: o.platform })}>${o.name}</button>`)}</div>` : null}
    <div class="pick-row">
      <select class="compact" aria-label="System" value=${platform} onChange=${(e) => setPlatform(e.target.value)}>
        ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      <input type="search" class="compact game-pick-search" aria-label="Search the system's games" placeholder="Search its games" value=${q} onInput=${(e) => setQ(e.target.value)} />
      <button type="button" class="ghost compact" data-action="no-game" onClick=${() => onPick(null, platform)}>No game</button>
      <button type="button" class="ghost compact" onClick=${onCancel}>Cancel</button>
    </div>
    ${found ? html`<ul class="ls-list game-pick-list">${found.slice(0, 12).map((g) => html`<li key=${g.id} data-game=${g.id}>
      <span><b>${g.name}</b>${g.have ? html` <span class="muted">in the library</span>` : null}</span>
      <button type="button" class="ghost compact" data-action="pick-game" onClick=${() => onPick(g)}>This one</button></li>`)}
      ${!found.length ? html`<li class="muted">No game of that name.</li>` : null}</ul>` : null}
  </div>`;
}

/** Import documents… (or Attach documents… on a game, with `game`, or `folder` for a PC
 *  game's, D55): read `paths` into items paired with games, adjusted here, then planned
 *  and run as a change. */
function DocsImportDialog({ paths, game = null, folder = null, gameName = "", platform = null }) {
  const platforms = (useStore(ui, (s) => s.platforms) || []).filter((p) => p.id !== "pc");
  const [items, setItems] = useState(null);
  const [kinds, setKinds] = useState([]);
  const [progress, setProgress] = useState("");
  const [picking, setPicking] = useState(null);
  const [how, setHow] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [only, setOnly] = useState("all");
  useEffect(() => {
    let gone = false;
    (async () => {
      try {
        const { job } = await api("docs_items", { paths, platform, game, folder, game_name: gameName });
        const done = await followJob(job, "Reading the documents", (j) => {
          const p = j.progress || {};
          if (!gone && p.items) setProgress(`${p.item} of ${p.items}: ${p.name || ""}`);
        });
        if (gone) return;
        if (done.error) throw new Error(done.error);
        setKinds(done.result.kinds);
        loadKinds(done.result.kinds);
        setItems(done.result.items);
      } catch (e) {
        if (!gone) setError(err(e));
      }
    })();
    return () => { gone = true; };
  }, []);
  const set = (path, patch) => setItems((all) => all.map((it) => (it.path === path ? { ...it, ...patch } : it)));
  const pickGame = (it, g, p) => {
    set(it.path, g ? { game: g.id, game_name: g.name, game_title: g.title || g.name, platform: g.platform, pairing: "you" } : { game: null, game_name: null, game_title: null, platform: p || it.platform, pairing: "none" });
    setPicking(null);
  };
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("docs_plan", { items, ...howArgs(how) }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  const one = game || folder;
  const title = one ? `Attach documents to ${gameName}` : "Import documents";
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="docs-import-dialog" wide=${true} sticky=${true} onSubmit=${() => { close(); runPlan(plan).then(again); }} busy=${plan.short || !plan.ops} submitLabel="Import"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Each goes into its game's library folder. ${how !== "move" ? "Your files stay where they are." : "The files are moved; Undo puts them back."}</p>
      <${More} id="docs-import-more"><p>Those with no game go into <code>_library/docs</code>. Each one's kind and title are kept in docs.json, and its text is read for searching afterwards.</p><//>
    <//>`;
  }
  const n = (k) => (items || []).filter((i) => i.pairing === k).length;
  const shown = (items || []).filter((it) => only === "all" || it.pairing === only);
  const included = (items || []).filter((i) => i.include).length;
  // a stray click or Esc doesn't lose the reading and the pairings (review M35)
  return html`<${Dialog} title=${title} id="docs-import-dialog" wide=${true} sticky=${!items || items.length > 0} onSubmit=${preview} busy=${busy || !items || !included} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${!items ? html`<p class="muted" id="docs-reading">Reading the files and pairing each with a game${progress ? html` (${progress})` : null}…</p>` : html`
      ${!one ? html`<div class="seg audit-tabs" role="tablist" id="docs-import-tabs">${[["all", "All", items.length], ["sure", "Sure", n("sure")], ["likely", "Likely", n("likely")], ["none", "No game", n("none")], ["you", "You chose", n("you")]].filter(([k, , c]) => k === "all" || c).map(([k, l, c]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${only === k ? "true" : "false"} onClick=${() => setOnly(k)}>${l} <span class="tab-count">${c}</span></button>`)}</div>` : null}
      <ul class="ls-list pc-items doc-items" id="doc-items">${shown.map((it) => {
        const [badge, why] = PAIRING[it.pairing] || [it.pairing, ""];
        return html`<li key=${it.path} class="pc-item" data-path=${it.path} data-name=${it.name} data-pairing=${it.pairing} data-include=${it.include ? "yes" : "no"}>
          <label class="check-row pc-item-include"><input type="checkbox" data-action="include" aria-label=${`Import ${it.name}`} checked=${it.include} onChange=${(e) => set(it.path, { include: e.target.checked })} /></label>
          <div class="pc-item-main">
            <span class="pc-item-name" title=${it.path}>${it.name} <span class="muted">${size(it.size)}</span></span>
            <span class="pc-item-edit">
              <input type="text" class="compact doc-item-title" aria-label="Title" value=${it.title} disabled=${!it.include} onInput=${(e) => set(it.path, { title: e.target.value })} />
              <select class="compact doc-item-kind" aria-label="Kind" value=${it.kind} disabled=${!it.include} onChange=${(e) => set(it.path, { kind: e.target.value })}>
                ${kinds.map((k) => html`<option key=${k.id} value=${k.id}>${k.name}</option>`)}</select>
            </span>
            <span class="muted pc-item-meta"><span class=${`pc-badge doc-pairing-${it.pairing}`} title=${why}>${badge}</span>
              <span class="doc-item-game">${it.game || it.folder ? it.game_name : it.platform ? `with ${platforms.find((p) => p.id === it.platform)?.name || it.platform}'s documents` : "with the general documents"}</span></span>
            ${picking === it.path ? html`<${GamePick} it=${it} platforms=${platforms} onPick=${(g, p) => pickGame(it, g, p)} onCancel=${() => setPicking(null)} />` : null}
          </div>
          ${picking !== it.path && !one ? html`<button type="button" class="ghost compact" data-action="change-game" disabled=${!it.include} onClick=${() => setPicking(it.path)}>Change game…</button>` : null}
        </li>`;
      })}</ul>
      <${HowControl} id="docs-import-mode" value=${how} onChange=${setHow} />
      <p class="muted"><b>${plural(included, "document", "documents")}</b> to import. Nothing changes until you've seen the preview.</p>`}
  <//>`;
}

/** Import documents… (files or a folder), or Attach documents… to `game` (a DAT game's id)
 *  or `folder` (a PC game's folder in the library). */
export async function attachDocs({ game = null, folder = null, gameName = "", platform = null, how = null, paths: given = null } = {}) {
  let paths = given || [];
  const one = game || folder;
  if (given) {
    // dropped on the page
  } else if ((how || (one ? "files" : "folder")) === "files") {
    paths = (await ctx.platform.library.pickFiles(one ? `Documents for ${gameName}` : "The documents to import")) || [];
  } else {
    const p = await ctx.platform.library.pickFolder("A folder of documents (guides, manuals, maps, scans)");
    if (p) paths = [p];
  }
  if (!paths.length) return;
  ui.set({ dialog: { type: "docs-import", paths, game, folder, gameName, platform } });
}

let dropping = null;
let listening = false;

/** Files dropped on the window (D78): to the shown page's own handler when it takes them
 *  (`useDroppedFiles`), else to `otherwise(paths, hint)`, the Import page. Started once. */
export function listenForDrops(otherwise) {
  if (listening || !ctx.platform?.onDrop) return;
  listening = true;
  ctx.platform.onDrop((paths) => {
    if (!paths?.length || ui.get().dialog) return;
    const took = dropping ? dropping(paths) : false;
    if (took !== true) otherwise(paths, took || null);
  }, (on) => {
    const el = document.documentElement;
    el.classList.toggle("drop-over", !!(on && dropping));
    el.classList.toggle("drop-over-import", !!(on && !dropping && !ui.get().dialog));
  });
}

/** Files dropped on the window go to `fn(paths)` while the page using this is shown (a
 *  game's page, the Metadata page's panel: they're attached to its game). It returns true
 *  when it took them; otherwise they go to the Import page, with the kind it returns as a
 *  hint ("mags"). */
export function useDroppedFiles(fn) {
  const ref = useRef(fn);
  ref.current = fn;
  useEffect(() => {
    const h = (paths) => ref.current?.(paths);
    dropping = h;
    return () => { if (dropping === h) dropping = null; };
  }, []);
}

export function DocDialogs({ dialog }) {
  if (dialog.type === "docs-import") return html`<${DocsImportDialog} key=${dialog.paths.join("|")} ...${dialog} />`;
  if (dialog.type === "doc") return html`<${DocDialog} key=${`${dialog.id}:${dialog.page || ""}`} id=${dialog.id} page=${dialog.page} find=${dialog.find} />`;
  return null;
}

// ------------------------------------------------------------ the page

export function Documents() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const saved = prefs()?.get("gr-docs", {}) || {};
  const [q, setQ] = useState("");
  const [kind, setKind] = useState(saved.kind || "");
  const [platform, setPlatform] = useState(saved.platform || "");
  const [source, setSource] = useState(saved.source || "");
  const [list, setList] = useState({ total: 0, items: [], loading: true });
  const [sel, setSel] = useState(null);
  const [tick, setTick] = useState(0);
  const [reading, setReading] = useState(false);
  const off = !!s.library?.read_only;
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    setList((l) => ({ ...l, loading: true }));
    const t = setTimeout(() => {
      api("docs_list", { query: q, kind, platform, source, offset: 0, limit: PAGE })
        .then((r) => { if (gone) return; setList({ ...r, loading: false }); loadKinds(r.kinds); })
        .catch((e) => { if (!gone) setList({ total: 0, items: [], loading: false, error: err(e) }); });
    }, q ? 250 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [s.library?.path, q, kind, platform, source, s.changes, tick]);
  const more = async () => {
    const r = await api("docs_list", { query: q, kind, platform, source, offset: list.items.length, limit: PAGE });
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  const filter = (patch) => {
    const next = { kind, platform, source, ...patch };
    if ("kind" in patch) setKind(patch.kind);
    if ("platform" in patch) setPlatform(patch.platform);
    if ("source" in patch) setSource(patch.source);
    prefs()?.set("gr-docs", next);
  };
  const readTexts = async () => {
    setReading(true);
    try {
      const { job } = await api("docs_read");
      const done = await followJob(job, "Reading documents");
      if (done.error) throw new Error(done.error);
      toast(`Read ${plural(done.result.read, "document", "documents")} for searching.`);
      setTick((t) => t + 1);
    } catch (e) {
      toast(`Couldn't read them: ${err(e)}`, 8000);
    } finally {
      setReading(false);
    }
  };
  const menu = () => [
    { id: "docs-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importFrom("import-docs", () => attachDocs({ how: "files" })) },
    { id: "docs-import-folder", label: "Import a folder…", icon: "folder", disabled: off || !isDesktop(), run: () => importFrom("import-docs", () => attachDocs({ how: "folder" })) },
    { sep: true },
    { id: "docs-read", label: "Make them searchable", icon: "search", disabled: reading || !list.unread, run: readTexts },
  ];
  const head = html`<${CommandBar} id="docs-head" title="Documents" count=${list.loading ? null : plural(list.total, "document", "documents")}
    search=${html`<input type="search" id="docs-search" class="compact" placeholder="Search the documents' words" aria-label="Search the documents" value=${q} onInput=${(e) => setQ(e.target.value)} />`}
    primary=${html`<button type="button" class="primary compact" id="docs-import" disabled=${off || !isDesktop()} title=${isDesktop() ? "Import a folder of guides, manuals or scans, each paired with its game" : "In the app"} onClick=${() => importFrom("import-docs", () => attachDocs({ how: "folder" }))}>${Icon.inbox(15)} Import documents…</button>`}
    more=${{ id: "docs-menu", title: "Import files, read their text", items: menu }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const filtered = kind || platform || source;
  if (!list.loading && !list.total && !q && !filtered) {
    return html`${head}<div class="pages"><div class="library-settings" id="docs-empty">
      <p class="page-lead">${Icon.book(18)} Guides, FAQs, manuals, maps, posters and scans, searchable by their words and kept with their games.</p>
      <section class="ls-section"><h2>To start</h2>
        <p>Import a folder of documents on the <a href=${routeHash("import")}>Import</a> page: each is paired with the game its name suggests, and you can change any pairing before they come in. Scraped manuals (the <a href=${routeHash("metadata")}>Metadata</a> page) are listed here too.</p>
        <div class="home-actions"><button type="button" class="primary" id="docs-first" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-docs", () => attachDocs({ how: "folder" }))}>Import documents…</button></div></section>
    </div></div>`;
  }
  const current = sel && list.items.some((d) => d.id === sel) ? sel : null;
  return html`${head}<div class="pages pc-page docs-page" id="docs-page">
    <div class="audit-pick">
      <label class="field-inline"><span>Kind</span>
        <select id="docs-kind" class="compact" value=${kind} onChange=${(e) => filter({ kind: e.target.value })}>
          <option value="">Every kind</option>
          ${(list.kinds || []).filter((k) => k.count || k.id === kind).map((k) => html`<option key=${k.id} value=${k.id}>${k.name} (${k.count})</option>`)}
        </select></label>
      <label class="field-inline"><span>System</span>
        <select id="docs-platform" class="compact" value=${platform} onChange=${(e) => filter({ platform: e.target.value })}>
          <option value="">Every system</option>
          ${(list.platforms || []).map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
        </select></label>
      <label class="field-inline"><span>From</span>
        <select id="docs-source" class="compact" value=${source} onChange=${(e) => filter({ source: e.target.value })}>
          <option value="">Anywhere</option><option value="yours">Yours</option>
          ${(list.sources || []).map((x) => html`<option key=${x} value=${x}>${sourceName(x)}</option>`)}
        </select></label>
      ${list.unread ? html`<span class="muted" id="docs-unread">${plural(list.unread, "document's", "documents'")} words aren't read yet. <button type="button" class="linkish" id="docs-read-now" disabled=${reading} onClick=${readTexts}>Read them</button></span>` : null}
    </div>
    <div class="pc-body docs-body">
      <div class="pc-list-wrap">
        <${SearchNote} q=${q} found=${list.loading ? null : plural(list.total, "document", "documents")} onClear=${() => setQ("")} />
        ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
        ${!list.loading && !list.items.length ? html`<p class="muted" id="docs-none">${q ? "No document has those words." : "None of that kind."}</p>` : null}
        <ul class="pc-list" id="docs-list" data-total=${list.total} aria-busy=${list.loading ? "true" : "false"}>${list.items.map((d) => html`<li key=${d.id}>
          <button type="button" class=${`pc-row${current === d.id ? " active" : ""}`} data-id=${d.id} data-kind=${d.kind} aria-pressed=${current === d.id ? "true" : "false"} onClick=${() => setSel(d.id)}>
            <span class="doc-kind-icon" aria-hidden="true">${Icon[d.view === "image" ? "image" : d.kind === "manual" || d.kind === "guide" || d.kind === "faq" ? "book" : "file"](16)}</span>
            <span class="pc-row-main"><span class="pc-row-title">${d.title}</span>
              <span class="muted pc-row-meta">${[d.kind_name, d.game, d.platform_name, d.source && sourceName(d.source)].filter(Boolean).join(" · ")}</span>
              ${d.snippet ? html`<${Snippet} text=${d.snippet} />` : null}</span>
          </button></li>`)}</ul>
        ${list.items.length < list.total ? html`<div class="home-actions"><button type="button" class="ghost" id="docs-more" onClick=${more}>Show ${Math.min(PAGE, list.total - list.items.length)} more (${list.total - list.items.length} left)</button></div>` : null}
      </div>
      ${current ? html`<${DocView} key=${current} id=${current} off=${off} find=${q} onChange=${() => setTick((t) => t + 1)} />` : html`<aside class="pc-panel pc-panel-empty"><p class="muted">Pick a document to read it here.</p></aside>`}
    </div>
  </div>`;
}
