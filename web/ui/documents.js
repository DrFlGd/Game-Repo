// Documents (docs/PLAN.md, "Phase 5 design", D30, D34): every guide, FAQ, manual, map,
// poster and scan in the library, searched by their words (the index reads the text of
// PDFs and text files), whatever game they're with. They're stored with their game
// (its folder's docs/, with docs.json for their kinds and titles), or in _library/docs/
// when they go with none. Import documents… reads files or a folder, pairs each with
// the game its name suggests, lets each pairing be changed, then brings them in as a
// change with a preview and Undo. The viewer shows PDFs, pictures, text and audio. The
// page is laid out as the Library is, more simply (D155): the filters (kind, system, game,
// where from, whether their words are read) with their counts down the left, grouped by
// game, kind or system, as lines or tiles (a document's first page, else its game's
// picture), and the one picked read in the panel; a page of 100 at a time, asked of the
// core.
import { html, useState, useEffect, useRef } from "../lib/html.js";
import { useStore, createStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, ViewSwitch, SortMenu, MenuButton } from "./layout.js";
import { Dialog, More, close } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs } from "./organise.js";
import { Icon } from "./icons.js";
import { importFrom } from "./importpage.js";
import { usePageKeys, openMenu } from "./actions.js";
import { Values, SIZES, gameHash, systemColour } from "./browse.js";
import { useShelfState, Pic } from "./shelf.js";

const PAGE = 100;
const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));

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

const DOC_SORTS = [["title", "Title"], ["game", "Game"], ["kind", "Kind"], ["size", "Size"]];
const DOC_GROUPS = [["", "None"], ["game", "Game"], ["kind", "Kind"], ["system", "System"]];
const DOC_VIEWS = [["list", "List", "list"], ["grid", "Grid", "grid"]];
/** The filters (D155), as the page's state and the query name them. */
const DOC_FIELDS = [["kind", "Kind", "kinds"], ["platform", "System", "platforms"], ["game", "Game", "games", true], ["source", "From", "sources"], ["text", "Text", "text"]];
const kindIcon = (d) => (d.view === "image" ? "image" : d.kind === "manual" || d.kind === "guide" || d.kind === "faq" ? "book" : "file");
/** A document's group, as the core names it: its game's folder, kind or system ("" for none). */
const groupOf = (d, group) => (group === "game" ? d.folder : group === "kind" ? d.kind : group === "system" ? d.platform : null) || "";
/** A document's pictures for a tile: its first page (a PDF's, a comic's) or itself, then its game's. */
const docPics = (d, grid) => [
  grid && (d.view === "pdf" || d.view === "comic") ? libraryUrl(`~first/${d.path}`) : null,
  grid && d.view === "image" ? libraryUrl(`~thumb/256/${d.path}`) : null,
  d.game_picture ? libraryUrl(`~thumb/${grid ? 256 : 128}/${d.game_picture}`) : null,
];

export function Documents() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const [st, set] = useShelfState("docs", { view: "list", sort: "title", group: "game" });
  const [list, setList] = useState({ total: 0, items: [], loading: true });
  const [sel, setSel] = useState(null);
  const [tick, setTick] = useState(0);
  const [reading, setReading] = useState(false);
  const scrollRef = useRef(null);
  const off = !!s.library?.read_only;
  const f = st.f || {};
  const sort = DOC_SORTS.some(([k]) => k === st.sort) ? st.sort : "title";
  const group = DOC_GROUPS.some(([k]) => k === st.group) ? st.group : "game";
  const view = DOC_VIEWS.some(([k]) => k === st.view) ? st.view : "list";
  const query = (offset) => ({ query: st.search, ...Object.fromEntries(DOC_FIELDS.map(([k, , arg]) => [arg, f[k] || []])), sort, desc: !!st.desc, group, offset, limit: PAGE, facets: offset === 0 });
  const key = JSON.stringify(query(0));
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    setList((l) => ({ ...l, loading: true }));
    const t = setTimeout(() => {
      api("docs_list", query(0))
        .then((r) => { if (gone) return; setList({ ...r, loading: false }); loadKinds(r.kinds); })
        .catch((e) => { if (!gone) setList({ total: 0, items: [], loading: false, error: err(e) }); });
    }, st.search ? 250 : 0);
    return () => { gone = true; clearTimeout(t); };
  }, [s.library?.path, key, s.changes, tick]);
  const more = async () => {
    const r = await api("docs_list", query(list.items.length));
    setList((l) => ({ ...l, items: [...l.items, ...r.items] }));
  };
  const change = (patch) => {
    set(patch);
    setSel(null);
    scrollRef.current?.scrollTo({ top: 0 });
  };
  const toggle = (k) => (v) => change({ f: { ...f, [k]: (f[k] || []).includes(v) ? f[k].filter((x) => x !== v) : [...(f[k] || []), v] } });
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
  const items = list.items || [];
  const current = sel != null ? items.find((d) => d.id === sel) || null : null;
  const menuFor = (d) => [
    { id: "doc-open-big", label: "Open", icon: "book", run: () => viewDoc(d.id, { find: st.search }) },
    { id: "doc-open-own", label: "Open in its own program", icon: "external", disabled: !isDesktop(), run: async () => { const x = await api("docs_get", { id: d.id }); ctx.platform.library.openPath(x.full); } },
    { id: "doc-show", label: "Show in folder", icon: "folder", disabled: !isDesktop(), run: async () => { const x = await api("docs_get", { id: d.id }); reveal(x.full); } },
    ...(d.game_key ? [{ id: "doc-game", label: "Go to its game", icon: "gamepad", run: () => { location.hash = gameHash(d.game_key); } }] : []),
  ];
  usePageKeys((e) => {
    if (e.key === "Escape" && current) { setSel(null); return; }
    if (e.key === "Enter" && current) { viewDoc(current.id, { find: st.search }); return; }
    if ((e.key === "ArrowDown" || e.key === "ArrowUp") && items.length) {
      e.preventDefault();
      const i = current ? items.indexOf(current) : -1;
      const next = items[Math.max(0, Math.min(items.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      setSel(next.id);
      scrollRef.current?.querySelector(`[data-id="${next.id}"]`)?.scrollIntoView({ block: "nearest" });
      return;
    }
    if (e.key === "ContextMenu" && current) {
      const r = scrollRef.current?.querySelector(".selected")?.getBoundingClientRect();
      if (r) openMenu({ clientX: r.left + 20, clientY: r.top + 20 }, menuFor(current));
    }
  });
  const menu = () => [
    ...(list.all ? [{ head: "Tile size" }, ...Object.keys(SIZES).map((k) => ({ id: `docs-size-${k}`, label: k[0].toUpperCase() + k.slice(1), icon: (st.size || "medium") === k ? "check" : null, run: () => set({ size: k }), disabled: view !== "grid" ? "Only in the grid" : false })), { sep: true }] : []),
    ...(list.all ? [{ id: "docs-import-more", label: "Import documents…", icon: "inbox", disabled: off || !isDesktop(), run: () => importFrom("import-docs", () => attachDocs({ how: "folder" })) }] : []),
    { id: "docs-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importFrom("import-docs", () => attachDocs({ how: "files" })) },
    { id: "docs-import-folder", label: "Import a folder…", icon: "folder", disabled: off || !isDesktop(), run: () => importFrom("import-docs", () => attachDocs({ how: "folder" })) },
    { sep: true },
    { id: "docs-read", label: "Make them searchable", icon: "search", disabled: reading || !list.unread, run: readTexts },
  ];
  const search = html`<input type="search" id="docs-search" data-search="1" class="compact" placeholder="Search the documents' words" aria-label="Search the documents" value=${st.search} onInput=${(e) => change({ search: e.target.value })} />`;
  const tools = html`<${ViewSwitch} id="docs-view" views=${DOC_VIEWS} value=${view} onChange=${(v) => set({ view: v })} />
    <${SortMenu} id="docs-sort" options=${DOC_SORTS} value=${sort} onChange=${(k) => change({ sort: k, desc: false })} />
    <button type="button" class="ghost compact" id="docs-desc" title=${st.desc ? "Reversed: back to the usual order" : "Reverse the order"} aria-pressed=${st.desc ? "true" : "false"} onClick=${() => change({ desc: !st.desc })}>${st.desc ? "↑" : "↓"}</button>
    <${MenuButton} id="docs-group" label=${`Group: ${DOC_GROUPS.find(([k]) => k === group)?.[1]}`} icon="grouped" items=${() => DOC_GROUPS.map(([k, l]) => ({ id: `docs-group-${k || "none"}`, label: l, icon: group === k ? "check" : null, run: () => change({ group: k }) }))} />`;
  const head = html`<${CommandBar} id="docs-head" title="Documents" count=${list.loading ? null : plural(list.total, "document", "documents")}
    search=${list.all ? search : null} tools=${list.all ? tools : null}
    primary=${list.all ? null : html`<button type="button" class="primary compact" id="docs-import" disabled=${off || !isDesktop()} title=${isDesktop() ? "Import a folder of guides, manuals or scans, each paired with its game" : "In the app"} onClick=${() => importFrom("import-docs", () => attachDocs({ how: "folder" }))}>${Icon.inbox(15)} Import documents…</button>`}
    more=${{ id: "docs-menu", title: "Import files, read their text, tile size", items: menu }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!list.loading && !list.all && !list.error) {
    return html`${head}<div class="pages"><div class="library-settings" id="docs-empty">
      <p class="page-lead">${Icon.book(18)} Guides, FAQs, manuals, maps, posters and scans, searchable by their words and kept with their games.</p>
      <section class="ls-section"><h2>To start</h2>
        <p>Import a folder of documents on the <a href=${routeHash("import")}>Import</a> page: each is paired with the game its name suggests, and you can change any pairing before they come in. Scraped manuals (the <a href=${routeHash("metadata")}>Metadata</a> page) are listed here too.</p>
        <div class="home-actions"><button type="button" class="primary" id="docs-first" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-docs", () => attachDocs({ how: "folder" }))}>Import documents…</button></div></section>
    </div></div>`;
  }
  const facets = list.facets || {};
  const nameOf = (k, v) => facets[k]?.find((x) => x.value === v)?.name || (k === "game" && v === "" ? "No game" : v);
  const chips = [];
  for (const [k, name] of DOC_FIELDS) for (const v of f[k] || []) chips.push([`${k}-${v}`, `${name}: ${nameOf(k, v)}`, () => toggle(k)(v)]);
  if (st.search) chips.push(["search", `Search: ${st.search}`, () => change({ search: "" })]);
  // the documents in their groups, each heading with how many the whole group has
  const sections = [];
  for (const d of items) {
    const g = group ? groupOf(d, group) : null;
    if (!sections.length || sections[sections.length - 1].value !== g) {
      const info = list.groups?.find((x) => (x.value || "") === g);
      sections.push({ value: g, name: info?.name || (g || "None"), count: info?.count, items: [] });
    }
    sections[sections.length - 1].items.push(d);
  }
  const draw = (d) => {
    const on = current?.id === d.id;
    const ev = { onClick: () => setSel(d.id), onDblClick: () => viewDoc(d.id, { find: st.search }), onContextMenu: (e) => { e.preventDefault(); setSel(d.id); openMenu(e, menuFor(d)); } };
    const none = { icon: kindIcon(d), colour: systemColour(d.kind) };
    const meta = [d.kind_name, group === "game" ? null : d.game, group === "system" ? null : d.platform_name, d.source && sourceName(d.source)].filter(Boolean).join(" · ");
    if (view === "grid") {
      return html`<button type="button" key=${d.id} class=${`lib-tile shelf-tile doc-tile${on ? " selected" : ""}`} data-id=${d.id} data-kind=${d.kind} aria-pressed=${on ? "true" : "false"} title=${d.title} ...${ev}>
        <span class="lib-tile-art"><${Pic} srcs=${docPics(d, true)} none=${none} /><span class="lib-badge" style=${`border-color:${systemColour(d.kind)}`}>${d.kind_name}</span></span>
        <span class="lib-tile-title">${d.title}</span>
        <span class="lib-tile-meta muted">${(group === "game" ? null : d.game) || d.platform_name || (d.source ? sourceName(d.source) : "")}</span></button>`;
    }
    return html`<button type="button" key=${d.id} class=${`lib-line shelf-line doc-line pc-row${on ? " selected active" : ""}`} data-id=${d.id} data-kind=${d.kind} aria-pressed=${on ? "true" : "false"} ...${ev}>
      <${Pic} srcs=${docPics(d, false)} none=${none} cls="lib-cover small" px=${18} />
      <span class="lib-line-main"><span class="lib-line-title">${d.title}</span><span class="muted">${meta}</span>${d.snippet ? html`<${Snippet} text=${d.snippet} />` : null}</span></button>`;
  };
  return html`${head}<div class=${`lib-page shelf-page docs-shelf${current ? " docs-open" : ""}`} id="docs-page" data-view=${view}>
    <aside class="lib-filters" id="docs-filters" aria-label="Filters">
      ${list.unread ? html`<div class="lib-switches"><p class="muted shelf-note" id="docs-unread">${plural(list.unread, "document's", "documents'")} words aren't read yet. <button type="button" class="linkish" id="docs-read-now" disabled=${reading} onClick=${readTexts}>Read them</button></p></div>` : null}
      ${DOC_FIELDS.map(([k, name, , long]) => html`<${Values} key=${k} id=${`docs-${k}`} name=${name} values=${facets[k]} chosen=${f[k] || []} onToggle=${toggle(k)} long=${!!long} />`)}
    </aside>
    <div class="lib-main">
      ${chips.length ? html`<div class="lib-chips" id="docs-chips">${chips.map(([k, label, x]) => html`<span class="lib-chip" key=${k} data-chip=${k}>${label}<button type="button" class="ghost" aria-label=${`Remove ${label}`} title="Remove" onClick=${x}>${Icon.close(11)}</button></span>`)}
        <button type="button" class="linkish" id="docs-clear" onClick=${() => change({ f: {}, search: "" })}>Clear</button></div>` : null}
      <div class="lib-scroll shelf-scroll" ref=${scrollRef} aria-busy=${list.loading ? "true" : "false"}>
        ${st.search && !list.loading ? html`<p class="muted shelf-note" id="docs-best">${list.total ? `The best matches first${group ? ", in each group" : ""}.` : ""}</p>` : null}
        ${list.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
        ${!list.loading && !items.length ? html`<p class="muted lib-none" id="docs-none">${st.search ? "No document has those words." : "No document matches these filters."}</p>` : null}
        <div class=${`shelf-items shelf-${view}s`} id="docs-list" data-total=${list.total} role="list" style=${`--tile:${SIZES[st.size] || SIZES.medium}px;--ratio:3 / 4`}>
          ${sections.map((sec) => html`<section class="shelf-section" key=${sec.value ?? "all"} data-group=${sec.value ?? ""} style=${`--n:${sec.items.length}`}>
            ${sec.value != null ? html`<div class="lib-group shelf-group-head" data-group=${sec.value}><h2 title=${sec.name}>${sec.name}</h2>${sec.count != null ? html`<span class="muted">${plural(sec.count, "document", "documents")}</span>` : null}
              ${group === "game" && sec.items[0]?.game_key ? html`<a class="shelf-group-link" href=${gameHash(sec.items[0].game_key)} title="Its page">${Icon.gamepad(14)} Its page</a>` : null}</div>` : null}
            <div class=${`shelf-${view}`}>${sec.items.map(draw)}</div></section>`)}
        </div>
        ${items.length < list.total ? html`<div class="home-actions"><button type="button" class="ghost" id="docs-more" onClick=${more}>Show ${Math.min(PAGE, list.total - items.length)} more (${list.total - items.length} left)</button></div>` : null}
      </div>
    </div>
    ${current ? html`<${DocView} key=${current.id} id=${current.id} off=${off} find=${st.search} onClose=${() => setSel(null)} onChange=${() => setTick((t) => t + 1)} />` : null}
  </div>`;
}
