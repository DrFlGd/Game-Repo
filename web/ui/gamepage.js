// A game's page (docs/PLAN.md, "Phase 6 design"): `#/game/<system>/<DAT name>` (or
// `pc/<id>`, or `<system>/~<SHA1>` for a game no DAT knows). Its pictures as a gallery,
// its details each saying where it came from and editable by hand (kept over later
// scrapes, D44), its documents, its files, the collections it's in, and Scrape…,
// Identify… and Attach documents…; files dropped on the page are attached to it.
// Each detail lists what the other sources said, and each kind of picture the other
// sources' ones, with Use this ("Phase 6b design", D46, D47); Scrape again… and Delete
// metadata… are in its menu (D48). Previous and Next go through the Library list it was
// opened from.
import { html, useState, useEffect, useLayoutEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { routeHash } from "./context.js";
import { api, toast, isDesktop, plural, size, libraryUrl, shortPath } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar } from "./layout.js";
import { usePageKeys } from "./actions.js";
import { scrapeDialog, deleteMetadata } from "./metadata.js";
import { attachDocs, viewDoc, sourceName, useDroppedFiles } from "./documents.js";
import { identifyDialog } from "./pcgames.js";
import { convertDialog } from "./organise.js";
import { opened, gameHash, systemColour } from "./browse.js";
import { Icon } from "./icons.js";

const err = (e) => e.message || String(e);
const PICTURES = { cover: "Box", back: "Back", box3d: "3D box", screenshot: "Screenshot", titlescreen: "Title screen", marquee: "Logo", fanart: "Fan art", physical: "Cartridge or disc", video: "Video", banner: "Banner", hero: "Hero", logo: "Logo", icon: "Icon" };
const ORDER = ["cover", "box3d", "back", "screenshot", "titlescreen", "fanart", "physical", "marquee", "video", "hero", "banner", "logo", "icon"];
const FIELDS = [["title", "Title"], ["year", "Year"], ["released", "Release date"], ["developer", "Developer"], ["publisher", "Publisher"], ["genres", "Genres"], ["players", "Players"], ["rating", "Rating"], ["description", "Description"]];
const isVideo = (rel) => /\.(mp4|webm|mkv|avi)$/i.test(rel || "");

/** The key in the address. */
export const keyFromHash = (hash = location.hash) => hash.replace(/^#\/game\//, "").split("?")[0].split("/").map(decodeURIComponent).join("/");

/** A value as it's shown in its box (genres joined, a rating as a percentage). */
function shown(field, v) {
  if (v == null) return "";
  if (field === "genres") return Array.isArray(v) ? v.join(", ") : String(v);
  if (field === "rating") return typeof v === "number" ? `${Math.round(v * 100)}%` : String(v);
  return String(v);
}

/** What's typed, as it's kept: "80%" is 0.8. */
function parsed(field, text) {
  const t = text.trim();
  if (!t) return null;
  if (field === "rating") {
    const n = Number(t.replace("%", ""));
    return Number.isNaN(n) ? t : t.includes("%") || n > 1 ? n / 100 : n;
  }
  return t;
}

/** What a source said of a detail. */
const said = (a, field) => (field === "title" || field === "year" ? a[field] : a.details?.[field]);
const short = (t) => (t.length > 90 ? `${t.slice(0, 88)}…` : t);

/** One detail: its box, where it came from (a source, or you), and what the other
 *  sources said, to use instead. */
function Detail({ g, field, label, off, main, onSave, onUnmark, onUse }) {
  const value = field === "title" || field === "year" ? g[field] : g.details?.[field];
  const [v, setV] = useState(shown(field, value));
  useLayoutEffect(() => setV(shown(field, value)), [JSON.stringify(value)]);
  const from = g.scraped?.[field];
  const mine = from === "you" || g.edited?.includes?.(field);
  const save = () => { if (v.trim() !== shown(field, value)) onSave(field, parsed(field, v)); };
  const others = (g.sources || []).filter((a) => a.source !== from && shown(field, said(a, field)) && shown(field, said(a, field)) !== shown(field, value));
  const pinned = g.pinned?.[field];
  const key = (e) => {
    if (e.key === "Escape") { e.stopPropagation(); setV(shown(field, value)); e.currentTarget.blur(); }
    else if (e.key === "Enter" && field !== "description") e.currentTarget.blur();
  };
  const props = { id: `game-${field}`, "aria-label": label, value: v, disabled: off, onInput: (e) => setV(e.target.value), onKeyDown: key, onBlur: save };
  return html`<div class=${`game-detail${field === "description" ? " wide" : ""}`} data-field=${field} data-edited=${mine ? "yes" : "no"}>
    <label class="game-detail-label" for=${`game-${field}`}>${label}</label>
    ${field === "description" ? html`<textarea rows="5" ...${props}></textarea>` : html`<input type="text" ...${props} placeholder=${off ? "" : field === "rating" ? "Not known (e.g. 80%)" : field === "genres" ? "Not known (e.g. Puzzle, Action)" : "Not known"} />`}
    <span class="game-detail-from muted">${mine ? html`Yours, kept over scrapes <button type="button" class="linkish" data-action="unmark" title="The sources' value comes back, and later scrapes may change it" onClick=${() => onUnmark(field)}>Let scrapes change it</button>`
      : from && (from !== main || pinned === from) ? html`From ${sourceName(from)}${pinned === from ? html`, as you chose <button type="button" class="linkish" data-action="unpin" title="Take it from the sources in their usual order" onClick=${() => onUse(field, null)}>Usual order</button>` : ""}` : ""}</span>
    ${others.length && !off ? html`<details class="game-detail-others" data-count=${others.length}><summary class="muted">${others.length === 1 ? `${others[0].name} says otherwise` : `${others.length} other sources say otherwise`}</summary>
      <ul>${others.map((a) => html`<li key=${a.source} data-source=${a.source}><span><b>${a.name}</b>: ${short(shown(field, said(a, field)))}</span>
        <button type="button" class="linkish" data-action="use" onClick=${() => onUse(field, a.source)}>Use this</button></li>`)}</ul></details>` : null}
  </div>`;
}

function Gallery({ g, off, onChoose }) {
  const media = g.media || {};
  const kinds = [...ORDER.filter((k) => media[k]), ...Object.keys(media).filter((k) => !ORDER.includes(k))];
  const [cur, setCur] = useState(kinds[0]);
  useEffect(() => setCur(kinds[0]), [g.key]);
  const kind = media[cur] ? cur : kinds[0];
  const vs = g.variants?.[kind] || [];
  // the chosen source's name after it, so another source's picture under the same name is shown (D47)
  const url = (k, thumb = "") => `${libraryUrl(`${thumb}${media[k]}`)}?v=${encodeURIComponent(g.variants?.[k]?.find((x) => x.chosen)?.name || "")}`;
  const rel = media[kind];
  if (!kinds.length) return html`<div class="game-gallery" id="game-gallery"><span class="game-hero lib-cover-none" style=${`background:${systemColour(g.platform)}`} aria-hidden="true">${(g.title || "?").slice(0, 1)}</span>
    <p class="muted">No pictures yet: Scrape… brings them${g.kind === "pc" ? " (or Identify on the PC games page)" : ""}.</p></div>`;
  return html`<div class="game-gallery" id="game-gallery">
    ${isVideo(rel) ? html`<video class="game-hero" src=${url(kind)} controls preload="metadata"></video>` : html`<img class="game-hero" src=${url(kind)} alt=${PICTURES[cur] || cur} />`}
    ${kinds.length > 1 ? html`<div class="game-strip" id="game-strip" role="tablist" aria-label="Pictures">${kinds.map((k) => html`<button type="button" role="tab" key=${k} data-kind=${k} aria-selected=${k === cur ? "true" : "false"} title=${PICTURES[k] || k} onClick=${() => setCur(k)}>
      ${isVideo(media[k]) ? html`<span class="game-strip-video">${Icon.image(16)} Video</span>` : html`<img src=${url(k, "~thumb/128/")} alt="" loading="lazy" />`}</button>`)}</div>` : null}
    <p class="muted game-caption">${PICTURES[cur] || cur}${g.scraped?.media?.[cur] ? ` · from ${sourceName(g.scraped.media[cur])}` : ""}</p>
    ${vs.length > 1 ? html`<p class="game-variants-label">${PICTURES[cur] || cur} from each source</p>
    <div class="game-variants" id="game-variants" data-kind=${cur} role="group" aria-label="From each source">${vs.map((x) => html`<button type="button" key=${x.name} data-name=${x.name} aria-pressed=${x.chosen ? "true" : "false"}
      disabled=${off} title=${x.chosen ? `Shown: ${x.source_name}'s` : `Use ${x.source_name}'s`} onClick=${() => { if (!x.chosen) onChoose(cur, x.name); }}>
      ${isVideo(x.path) ? html`<span class="game-strip-video">${Icon.image(16)} Video</span>` : html`<img src=${libraryUrl(`~thumb/128/${x.path}`)} alt="" loading="lazy" />`}
      <span class="muted">${x.source_name}${x.chosen ? " · shown" : ""}</span></button>`)}</div>` : null}
  </div>`;
}

export function GamePage() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes, route: st.route }));
  const off = !!s.library?.read_only;
  const [key, setKey] = useState(keyFromHash);
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    const on = () => { if (location.hash.startsWith("#/game/")) setKey(keyFromHash()); };
    addEventListener("hashchange", on);
    return () => removeEventListener("hashchange", on);
  }, []);
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    setError("");
    api("browse").then(() => api("browse_game", { key })).then((x) => { if (!gone) setG(x); }).catch((e) => { if (!gone) { setG(null); setError(err(e)); } });
    return () => { gone = true; };
  }, [key, s.changes, s.library?.path]);
  const dat = g?.kind === "dat";
  const attach = (paths = null) => {
    if (!dat) { toast("Documents are attached to a system's games here; a PC game's go in its folder.", 8000); return; }
    attachDocs({ game: g.id, gameName: g.title, platform: g.platform, paths });
  };
  useDroppedFiles((paths) => { if (g && !off) attach(paths); });
  const save = async (field, value) => {
    setBusy(true);
    setError("");
    try {
      setG(await api("browse_edit", { key, field, value }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  const unmark = async (field) => {
    try { setG(await api("browse_edit", { key, field })); } catch (e) { setError(err(e)); }
  };
  // another source's value, or picture, used instead (D46, D47)
  const changed = async (cmd, args) => {
    setBusy(true);
    setError("");
    try {
      await api(cmd, { folder: g.folder, ...args });
      setG(await api("browse_game", { key }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  const use = (field, source) => changed("meta_pin", { field, source });
  const choose = (kind, name) => changed("meta_choose", { kind, name });
  const step = async (by) => {
    const i = opened.index + by;
    if (!opened.query || i < 0 || i >= opened.total) return;
    const r = await api("browse_query", { query: { ...opened.query, offset: i, limit: 1 } });
    if (!r.items[0]) return;
    opened.index = i;
    // in place of this game in the history, so Back goes to the list rather than through every game stepped past
    location.replace(gameHash(r.items[0].key));
  };
  const back = () => (history.length > 1 ? history.back() : (location.hash = routeHash("library")));
  usePageKeys((e) => {
    if (e.altKey && e.key === "ArrowLeft") { e.preventDefault(); back(); }
    else if (e.key === "ArrowRight" || e.key === "PageDown") step(1);
    else if (e.key === "ArrowLeft" || e.key === "PageUp") step(-1);
  });
  const fromList = opened.query && opened.index >= 0;
  const more = () => [
    ...(dat ? [{ id: "game-identify", label: "Identify…", icon: "search", disabled: off, run: () => identifyDialog({ title: `Identify ${g.name}`, query: g.title, platform: g.platform, onPick: async (x) => { setG(await api("meta_pick", { game: g.id, source: x.source, id: x.id }).then(() => api("browse_game", { key }))); } }) }] : []),
    ...(g?.kind === "pc" ? [{ id: "game-pc", label: "Open in PC games", icon: "monitor", run: () => { location.hash = routeHash("pc"); } }] : []),
    ...(dat ? [{ id: "game-scrape-again", label: "Scrape again…", icon: "scrape", disabled: off, run: () => scrapeDialog({ platform: g.platform, platformName: g.platform_name, games: [g.id], gameName: g.title, again: true }) }] : []),
    ...(g?.kind !== "pc" ? [{ id: "game-delete-metadata", label: "Delete metadata…", icon: "trash", disabled: off || !g?.folder, run: () => deleteMetadata({ folder: g.folder, gameName: g.title, platformName: g.platform_name }) }] : []),
    { sep: true },
    { id: "game-attach", label: "Attach documents…", icon: "file", disabled: off || !isDesktop() || !dat, run: () => attach() },
    { id: "game-reveal", label: "Show its folder", icon: "folder", disabled: !isDesktop() || !(g?.folder_path || g?.dir), run: () => reveal(g.folder_path || g.dir) },
    { sep: true },
    { id: "game-add", label: "Add to a collection…", icon: "collection", disabled: off || !dat, run: () => ui.set({ dialog: { type: "lib-add", games: [{ key: g.key, kind: g.kind, platform: g.platform }] } }) },
  ];
  const head = html`<${CommandBar} id="game-head" back=${{ id: "game-back", run: back }} title=${g?.title || "A game"} count=${g ? [g.regions, g.platform_name].filter(Boolean).join(" · ") || null : null}
    tools=${fromList ? html`<span class="seg"><button type="button" class="ghost compact" id="game-prev" title="The previous game in the list (←)" disabled=${opened.index <= 0} onClick=${() => step(-1)}>‹</button>
      <span class="muted game-pos">${(opened.index + 1).toLocaleString()} of ${opened.total.toLocaleString()}</span>
      <button type="button" class="ghost compact" id="game-next" title="The next game in the list (→)" disabled=${opened.index >= opened.total - 1} onClick=${() => step(1)}>›</button></span>` : null}
    primary=${g && dat ? html`<button type="button" class="primary compact" id="game-scrape" disabled=${off} onClick=${() => scrapeDialog({ platform: g.platform, platformName: g.platform_name, games: [g.id], gameName: g.title })}>${Icon.scrape(15)} Scrape…</button>` : null}
    more=${g ? { id: "game-menu", title: "Identify, attach documents, show its folder, add to a collection", items: more } : null} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (error && !g) return html`${head}<div class="pages"><p class="warn-note" role="alert" id="game-error">${error}</p><p><a href=${routeHash("library")}>Back to the Library</a></p></div>`;
  if (!g) return html`${head}<div class="pages"><p class="muted">Reading…</p></div>`;
  const removeFrom = async (c) => {
    try {
      const cur = (await api("collection_get", { id: c.id })).collection;
      cur.added = (cur.added || []).filter((k) => k !== g.key);
      if (!cur.removed.includes(g.key)) cur.removed = [...cur.removed, g.key];
      await api("collection_save", { collection: cur });
      ui.set((x) => ({ changes: (x.changes || 0) + 1 }));
      toast(`Took ${g.title} out of ${c.name}.`, 6000);
    } catch (e) {
      toast(`Couldn't change ${c.name}: ${err(e)}`, 8000);
    }
  };
  const files = g.files || [];
  // the source most details came from: named once, above them, instead of under each
  const tally = {};
  for (const [k] of FIELDS) { const f = g.scraped?.[k]; if (f && f !== "you") tally[f] = (tally[f] || 0) + 1; }
  const main = Object.entries(tally).sort((a, b) => b[1] - a[1])[0]?.[0] || null;
  const dir = g.dir || "";
  return html`${head}<div class="pages game-page" id="game-page" data-key=${g.key} data-kind=${g.kind}>
    <div class="game-top">
      <${Gallery} g=${g} off=${off || busy || g.kind === "pc"} onChoose=${choose} />
      <section class="game-details" id="game-details" aria-busy=${busy ? "true" : "false"}>
        <p class="muted game-sub">${[g.platform_name, g.kind === "dat" ? (g.have ? "in the library" : "not in the library") : g.kind === "pc" ? "a PC game" : "a game no DAT knows", g.name && g.name !== g.title ? g.name : null].filter(Boolean).join(" · ")}</p>
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
        ${main ? html`<p class="muted game-main-source" id="game-main-source">Details from ${sourceName(main)}, unless a detail says otherwise.</p>` : null}
        <div class="game-fields">${FIELDS.map(([k, l]) => html`<${Detail} key=${k} g=${g} field=${k} label=${l} off=${off || busy} main=${main} onSave=${save} onUnmark=${unmark} onUse=${use} />`)}</div>
        <p class="muted">${off ? "" : "Click a detail to change it. "}What you change here is kept in the game's folder and stays when it's scraped again.</p>
        ${Object.keys(g.ids || {}).length ? html`<p class="muted" id="game-ids">Its ids at the sources: ${Object.entries(g.ids).map(([k, v]) => `${sourceName(k)} ${v}`).join(" · ")}</p>` : null}
      </section>
    </div>
    ${g.review?.length ? html`<section class="ls-section" id="game-review"><h2>Which game is it?</h2>
      <p class="muted">A scrape wasn't sure of this one: <a href=${routeHash("metadata")}>the Metadata page</a> lists what it found, to pick from.</p></section>` : null}
    <section class="ls-section" id="game-docs"><h2>Documents</h2>
      ${g.docs?.length ? html`<ul class="ls-list">${g.docs.map((x) => html`<li key=${x.id} data-kind=${x.kind}><button type="button" class="linkish" onClick=${() => viewDoc(x.id)}>${x.title}</button><span class="muted">${x.kind_name}${x.source ? ` · ${sourceName(x.source)}` : ""}</span></li>`)}</ul>`
        : html`<p class="muted">None yet.${dat ? " Attach your own (or drop files on this page), or scrape its manual." : ""}</p>`}
      ${dat && isDesktop() ? html`<button type="button" class="ghost compact" id="game-attach" disabled=${off} onClick=${() => attach()}>${Icon.file(14)} Attach documents…</button>` : null}
    </section>
    <section class="ls-section" id="game-files"><h2>Files</h2>
      ${files.length ? html`<ul class="ls-list">${files.map((f, i) => html`<li key=${i} data-missing=${f.missing ? "yes" : "no"}>
        <span class="op-paths"><span class="op-to" title=${f.path}>${dir ? f.path : shortPath(f.path, 3)}</span>
          <span class="muted">${[f.kind, f.size ? size(f.size) : null, f.roms?.length > 1 ? plural(f.roms.length, "ROM", "ROMs") : null, f.missing ? "gone since the last scan" : null].filter(Boolean).join(" · ")}</span></span>
        ${isDesktop() && !f.missing && !dir ? html`<button type="button" class="ghost compact" onClick=${() => reveal(f.path)}>Show</button>` : null}</li>`)}</ul>`
        : html`<p class="muted">${g.kind === "dat" && !g.have ? "You don't have it: the audit lists what to look for." : "No files."}</p>`}
      ${dat && g.have && g.dat_id && ui.get().platforms?.find((p) => p.id === g.platform)?.formats?.length ? html`<button type="button" class="ghost compact" id="game-convert" disabled=${off} onClick=${() => convertDialog({ dat: g.dat_id, platform: g.platform, platformName: g.platform_name, games: [g.name] })}>${Icon.convert(14)} Convert…</button>` : null}
      ${dat ? html`<dl class="pc-facts" id="game-dat">
        <dt>In the DAT</dt><dd>${g.dat || ""}</dd>
        <dt>Name</dt><dd>${g.name}</dd>
        ${g.regions ? html`<dt>Regions</dt><dd>${g.regions}</dd>` : null}
        <dt>ROMs</dt><dd>${(g.roms || []).length}</dd></dl>` : null}
    </section>
    ${dat ? html`<section class="ls-section" id="game-collections"><h2>Collections</h2>
      ${g.collections?.length ? html`<ul class="ls-list">${g.collections.map((c) => html`<li key=${c.id} data-id=${c.id}><a href=${routeHash("collections")}>${c.name}</a>
        <button type="button" class="ghost compact" disabled=${off} title=${`Take it out of ${c.name}`} onClick=${() => removeFrom(c)}>Take out</button></li>`)}</ul>` : html`<p class="muted">In none.</p>`}
      <button type="button" class="ghost compact" id="game-add" disabled=${off} onClick=${() => ui.set({ dialog: { type: "lib-add", games: [{ key: g.key, kind: g.kind, platform: g.platform }] } })}>${Icon.collection(14)} Add to a collection…</button>
    </section>` : null}
  </div>`;
}
