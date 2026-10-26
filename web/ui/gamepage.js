// A game's page (docs/PLAN.md, "Phase 6 design"): `#/game/<system>/<DAT name>` (or
// `pc/<id>`, or `<system>/~<SHA1>` for a game no DAT knows). Its pictures as a gallery,
// its details each saying where it came from and editable by hand (kept over later
// scrapes, D44), its documents, its files, the collections it's in, and Scrape…,
// Identify… and Attach documents…; files dropped on the page are attached to it.
// Each detail lists what the other sources said, and each kind of picture the other
// sources' ones, with Use this ("Phase 6b design", D46, D47); Scrape again… and Delete
// metadata… are in its menu (D48). Previous and Next go through the Library list it was
// opened from (or a collection's, or the backlog's). Its saves from devices, with their
// versions (D191). Its place in the backlog is shown with
// In progress, Completed and the rest (D157–D159). A PC game's page is the same, its details and pictures kept as any game's,
// with a PC section for what only a PC game has: the program, Unpack, its folder and files
// ("PC game page design", D55); its title and year name its folder, so they're its own.
// Its menu has Remove from the library… (D129), and Identify… None of these (D128).
// Along its right edge, a ribbon that folds (]): every kind of picture it has, each
// source's, larger, with Use this (D69).
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
import { identifyDialog, PcParts, pcLine, removePc, unmatchPc } from "./pcgames.js";
import { convertDialog } from "./organise.js";
import { opened, gameHash, systemColour, libraryHash, addonLabel, discsLine } from "./browse.js";
import { Icon } from "./icons.js";
import { Inspector } from "./panels.js";
import { isVideo, Video } from "./video.js";
import { watch, videoFacts } from "./videos.js";
import { variantDialog, undoVariants } from "./variants.js";
import { addToBacklog, backlogDo, dayText } from "./backlog.js";
import { GameSaves } from "./saves.js";

/** The key a collection knows a game by (D175): a disc's game's, an update's or DLC's game's. */
const inKey = (g) => g.disc_of?.key || g.addon?.base?.key || g.key;

const err = (e) => e.message || String(e);
const PICTURES = { cover: "Box front", back: "Back", box3d: "3D box", screenshot: "Screenshot", titlescreen: "Title screen", marquee: "Logo", fanart: "Fan art", physical: "Cartridge or disc", video: "Video", banner: "Banner", hero: "Hero", logo: "Logo", icon: "Icon" };
const ORDER = ["cover", "box3d", "back", "screenshot", "titlescreen", "fanart", "physical", "marquee", "video", "hero", "banner", "logo", "icon"];
const FIELDS = [["title", "Title"], ["year", "Year"], ["released", "Release date"], ["developer", "Developer"], ["publisher", "Publisher"], ["genres", "Genres"], ["players", "Players"], ["rating", "Rating"], ["description", "Description"]];

const ADDON_KINDS = { update: "Update", dlc: "DLC", patch: "Patch" };

/** A game's updates, DLC and patches (D66): which you have and their versions, each one
 *  movable to another game, and the system's ones that belong to no game, to link here. */
function Addons({ g, dat, off, onChange }) {
  const a = g.addons;
  const [pick, setPick] = useState("");
  const link = async (addon, base, clear = false) => {
    try {
      await api("addon_link", { platform: g.platform, addon, base, clear });
      ui.set((x) => ({ changes: (x.changes || 0) + 1 }));
      await onChange();
    } catch (e) {
      toast(`Couldn't link it: ${err(e)}`, 8000);
    }
  };
  return html`<section class="ls-section" id="game-addons"><h2>Updates and DLC</h2>
    ${a.items.length ? html`<p class="muted" id="game-addons-have">${g.kind === "pc" ? "Its patches and DLC, in its folder." : `You have ${a.have.toLocaleString()} of ${a.items.length.toLocaleString()}.`}</p>
      <ul class="ls-list">${a.items.map((x) => html`<li key=${x.name} data-status=${x.status} data-kind=${x.kind}>
        <span class="op-paths"><span class="op-to">${x.name}</span>
          <span class="muted">${[ADDON_KINDS[x.kind] || x.kind, x.version, dat ? (x.status === "have" ? "you have it" : x.status === "partial" ? "some of its files" : "you don't have it") : null, x.by_hand ? "linked by hand" : null].filter(Boolean).join(" · ")}</span></span>
        ${dat ? html`<span class="seg">
          <button type="button" class="ghost compact" disabled=${off} onClick=${() => ui.set({ dialog: { type: "addon-link", platform: g.platform, addon: x.name, from: g.name, onDone: onChange } })}>Link to another game…</button>
          ${x.by_hand ? html`<button type="button" class="ghost compact" disabled=${off} title="Go back to the game its name says" onClick=${() => link(x.name, null, true)}>Undo the link</button>` : null}</span>` : null}</li>`)}</ul>`
      : html`<p class="muted">None of its system's updates or DLC belong to it.</p>`}
    ${dat && a.loose.length ? html`<p class="addon-loose"><label>Not linked to a game:
      <select id="game-addon-loose" class="compact" value=${pick} onChange=${(e) => setPick(e.target.value)}>
        <option value="">${a.loose.length.toLocaleString()} to choose from</option>
        ${a.loose.map((n) => html`<option key=${n} value=${n}>${n}</option>`)}</select></label>
      <button type="button" class="ghost compact" id="game-addon-link-here" disabled=${off || !pick} onClick=${() => link(pick, g.name).then(() => setPick(""))}>Link to this game</button></p>` : null}
  </section>`;
}

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
    <p class="muted">No pictures yet: ${g.kind === "pc" ? "Identify… brings them" : "Scrape… brings them"}.</p></div>`;
  return html`<div class="game-gallery" id="game-gallery">
    ${isVideo(rel) ? html`<${Video} cls="game-hero" rel=${rel} src=${url(kind)} />` : html`<img class="game-hero" src=${url(kind)} alt=${PICTURES[cur] || cur} />`}
    ${kinds.length > 1 ? html`<div class="game-strip" id="game-strip" role="tablist" aria-label="Pictures">${kinds.map((k) => html`<button type="button" role="tab" key=${k} data-kind=${k} aria-selected=${k === cur ? "true" : "false"} title=${PICTURES[k] || k} onClick=${() => setCur(k)}>
      ${isVideo(media[k]) ? html`<span class="game-strip-video">${Icon.video(16)} Video</span>` : html`<img src=${url(k, "~thumb/128/")} alt="" loading="lazy" />`}</button>`)}</div>` : null}
    <p class="muted game-caption">${PICTURES[cur] || cur}${g.scraped?.media?.[cur] ? ` · from ${sourceName(g.scraped.media[cur])}` : ""}</p>
    ${vs.length > 1 ? html`<p class="game-variants-label">${PICTURES[cur] || cur} from each source</p>
    <div class="game-variants" id="game-variants" data-kind=${cur} role="group" aria-label="From each source">${vs.map((x) => html`<button type="button" key=${x.name} data-name=${x.name} aria-pressed=${x.chosen ? "true" : "false"}
      disabled=${off} title=${x.chosen ? `Shown: ${x.source_name}'s` : `Use ${x.source_name}'s`} onClick=${() => { if (!x.chosen) onChoose(cur, x.name); }}>
      ${isVideo(x.path) ? html`<span class="game-strip-video">${Icon.video(16)} Video</span>` : html`<img src=${libraryUrl(`~thumb/128/${x.path}`)} alt="" loading="lazy" />`}
      <span class="muted">${x.source_name}${x.chosen ? " · shown" : ""}</span></button>`)}</div>` : null}
  </div>`;
}

/** The Media ribbon: each kind, the one shown and every source's, larger on a click. */
function MediaPane({ g, off, onChoose }) {
  const [big, setBig] = useState(null);
  useEffect(() => setBig(null), [g.key]);
  const media = g.media || {};
  const kinds = [...ORDER.filter((k) => media[k]), ...Object.keys(media).filter((k) => !ORDER.includes(k))];
  if (!kinds.length) return html`<p class="muted media-none" id="game-media">No pictures yet: ${g.kind === "pc" ? "Identify… brings them" : "Scrape… brings them"}.</p>`;
  const shownName = (k) => g.variants?.[k]?.find((x) => x.chosen)?.name || "";
  // a video plays only when seen larger: a small one in its button showed no controls of its
  // own but could start, and opening it larger then played it a second time (Jerred, batch 4)
  const pic = (rel, name, thumb) => isVideo(rel)
    ? thumb ? html`<span class="game-strip-video">${Icon.video(16)} Video</span>`
      : html`<${Video} rel=${rel} src=${`${libraryUrl(rel)}?v=${encodeURIComponent(name)}`} />`
    : html`<img src=${`${libraryUrl(`${thumb}${rel}`)}?v=${encodeURIComponent(name)}`} alt="" loading="lazy" />`;
  return html`<div class="media-pane" id="game-media">
    ${big ? html`<figure class="media-view" id="media-view">
      ${pic(big.path, big.name, "")}
      <figcaption><span>${big.label} · ${big.source}</span>
        <button type="button" class="ghost compact" id="media-view-close" onClick=${() => setBig(null)}>Close</button></figcaption>
    </figure>` : null}
    ${kinds.map((k) => {
      const vs = g.variants?.[k]?.length ? g.variants[k] : [{ name: shownName(k), path: media[k], chosen: true, source_name: g.scraped?.media?.[k] ? sourceName(g.scraped.media[k]) : "" }];
      const label = PICTURES[k] || k;
      return html`<section class="media-kind" key=${k} data-kind=${k}>
        <h3>${label}${vs.length > 1 ? html` <span class="muted">${plural(vs.length, "source", "sources")}</span>` : null}</h3>
        <div class="media-variants">${vs.map((x) => html`<div class=${`media-variant${x.chosen ? " chosen" : ""}`} key=${x.name || "shown"} data-name=${x.name}>
          <button type="button" class="media-thumb" title=${`See it larger: ${label}${x.source_name ? `, ${x.source_name}'s` : ""}`} onClick=${() => setBig({ path: x.path, name: x.name, label, source: x.source_name || "shown" })}>${pic(x.path, x.name, "~thumb/256/")}</button>
          <span class="media-source">${x.source_name || ""}${x.chosen ? html` <b>· shown</b>` : null}</span>
          ${!x.chosen && vs.length > 1 ? html`<button type="button" class="linkish" data-action="use" disabled=${off} onClick=${() => onChoose(k, x.name)}>Use this</button>` : null}
        </div>`)}</div>
      </section>`;
    })}
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
  const pc = g?.kind === "pc";
  const attach = (paths = null) => {
    // a PC game's go in its folder in the library (D55), as a DAT game's do
    if (pc) { attachDocs({ folder: g.folder, gameName: g.title, platform: "pc", paths }); return; }
    if (!dat) { toast("Documents are attached to the games a DAT knows, and PC games.", 8000); return; }
    attachDocs({ game: g.id, gameName: g.title, platform: g.platform, paths });
  };
  const reload = async () => setG(await api("browse_game", { key }));
  const identifyPc = () => identifyDialog({
    title: `Identify ${g.title}`, query: g.title, year: g.year, platform: "pc",
    onPick: async (x) => { await api("identify_pick", { source: x.source, id: x.id, game: g.pc_id, retitle: true }); await api("browse"); await reload(); },
    onNone: async () => { await unmatchPc({ id: g.pc_id, title: g.title }); await api("browse"); await reload(); }, noneNote: "its ids at the sources go, and its title stays.",
  });
  useDroppedFiles((paths) => {
    if (!g || off) return false;
    attach(paths);
    return true;
  });
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
  // a PC game's title and year are its own: another source's is taken as typed in
  const use = (field, source) => (pc && (field === "title" || field === "year")
    ? save(field, (g.sources || []).find((a) => a.source === source)?.[field] ?? null)
    : changed("meta_pin", { field, source }));
  const choose = (kind, name) => changed("meta_choose", { kind, name });
  const fromList = !!(opened.query || opened.keys) && opened.index >= 0 && opened.key === key;
  const step = async (by) => {
    const i = opened.index + by;
    if (!fromList || i < 0 || i >= opened.total) return;
    // a list of its own (the backlog's), else the Library's query a game at a time
    const next = opened.keys ? opened.keys[i] : (await api("browse_query", { query: { ...opened.query, offset: i, limit: 1 } })).items[0]?.key;
    if (!next) return;
    Object.assign(opened, { index: i, key: next });
    // in place of this game in the history, so Back goes to the list rather than through every game stepped past
    location.replace(gameHash(next));
  };
  const back = () => (history.length > 1 ? history.back() : (location.hash = libraryHash()));
  const tryIt = (p) => p.catch((e) => toast(`Couldn't change the backlog: ${err(e)}`, 8000));
  const removeVariant = async () => {
    try {
      await api("variants_remove", { platform: g.platform, names: [g.variant.name] });
      ui.set((x) => ({ changes: (x.changes || 0) + 1 }));
      toast(`Took ${g.variant.name} out of the custom DAT: its file is on Audit › Unknown again.`, { ms: 10000, actions: [{ label: "Undo", run: () => undoVariants(g.platform) }] });
      back();
    } catch (e) { toast(`Couldn't take it out: ${err(e)}`, 8000); }
  };
  usePageKeys((e) => {
    if (e.altKey && e.key === "ArrowLeft") { e.preventDefault(); back(); }
    else if (e.key === "ArrowRight" || e.key === "PageDown") step(1);
    else if (e.key === "ArrowLeft" || e.key === "PageUp") step(-1);
  });
  const more = () => [
    ...(dat ? [{ id: "game-identify", label: "Identify…", icon: "search", disabled: off, run: () => identifyDialog({ title: `Identify ${g.name}`, query: g.title, platform: g.platform, onPick: async (x) => { setG(await api("meta_pick", { game: g.id, source: x.source, id: x.id }).then(() => api("browse_game", { key }))); } }) }] : []),
    ...(pc ? [{ id: "game-pc", label: "Open in PC games", icon: "monitor", run: () => { location.hash = routeHash("pc"); } }] : []),
    ...(dat ? [{ id: "game-scrape-again", label: "Scrape again…", icon: "scrape", disabled: off, run: () => scrapeDialog({ platform: g.platform, platformName: g.platform_name, games: [g.id], gameName: g.title, again: true }) }] : []),
    // a variant's own (D123): its name, tags and original; taken out, its file is on Unknown again
    ...(g?.variant ? [
      { id: "game-variant-edit", label: "Edit the variant…", icon: "edit", disabled: off, run: () => variantDialog({ platform: g.platform, editing: g.variant.name, was: g.variant, original: g.variant.of?.name || null, onDone: (n) => { if (n !== g.variant.name) location.replace(gameHash(`${g.platform}/${n}`)); } }) },
      { id: "game-variant-remove", label: "Remove from the custom DAT", icon: "trash", disabled: off, run: removeVariant },
    ] : []),
    { id: "game-delete-metadata", label: "Delete metadata…", icon: "trash", disabled: off || !g?.folder, run: () => deleteMetadata({ folder: g.folder, gameName: g.title, platformName: g.platform_name, pc }) },
    // the game itself, its folder and its details set aside as a change with Undo (D129)
    ...(pc ? [{ id: "game-pc-remove", label: "Remove from the library…", icon: "trash", disabled: off, run: () => removePc(g.pc_id, () => { location.hash = routeHash("pc"); }) }] : []),
    { sep: true },
    { id: "game-attach", label: "Attach documents…", icon: "file", disabled: off || !isDesktop() || !(dat || pc), run: () => attach() },
    // a PC game's own folder (PC/<Title> (<Year>)), else where its metadata is kept
    { id: "game-reveal", label: "Show in folder", icon: "folder", disabled: !isDesktop() || !(g?.pc?.dir || g?.folder_path || g?.dir), run: () => reveal(g.pc?.dir || g.folder_path || g.dir) },
    { sep: true },
    { id: "game-add", label: "Add to a collection…", icon: "collection", disabled: off || (!dat && "Collections hold games from DATs, and a PC game has none"), run: () => ui.set({ dialog: { type: "lib-add", games: [{ key: inKey(g), kind: g.kind, platform: g.platform }] } }) },
    ...(g && !g.backlog ? [{ id: "game-backlog-add-menu", label: "Add to the backlog", icon: "list", disabled: off, run: () => tryIt(addToBacklog([{ key: g.key, title: g.title, platform: g.platform }])) }] : []),
  ];
  const head = html`<${CommandBar} id="game-head" back=${{ id: "game-back", run: back }} title=${g?.title || "A game"} count=${g ? [g.regions, g.platform_name].filter(Boolean).join(" · ") || null : null}
    tools=${fromList ? html`<span class="game-steps" id="game-steps"><button type="button" class="ghost compact" id="game-prev" title=${`The previous game in ${opened.name || "the list"} (←)`} disabled=${opened.index <= 0} onClick=${() => step(-1)}>‹ Previous</button>
      <span class="muted game-pos" title=${opened.name ? `Its place in ${opened.name}` : null}>${(opened.index + 1).toLocaleString()} of ${opened.total.toLocaleString()}${opened.name ? ` in ${opened.name}` : ""}</span>
      <button type="button" class="ghost compact" id="game-next" title=${`The next game in ${opened.name || "the list"} (→)`} disabled=${opened.index >= opened.total - 1} onClick=${() => step(1)}>Next ›</button></span>` : null}
    primary=${g && dat ? html`<button type="button" class="primary compact" id="game-scrape" disabled=${off} onClick=${() => scrapeDialog({ platform: g.platform, platformName: g.platform_name, games: [g.id], gameName: g.title })}>${Icon.scrape(15)} Scrape…</button>`
      : g && pc ? html`<button type="button" class="primary compact" id="game-identify-pc" disabled=${off} title="Search Steam, GOG and the other sources for it" onClick=${identifyPc}>${Icon.search(15)} Identify…</button>` : null}
    more=${g ? { id: "game-menu", title: "Identify, attach documents, show its folder, add to a collection", items: more } : null} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (error && !g) return html`${head}<div class="pages"><p class="warn-note" role="alert" id="game-error">${error}</p><p><a href=${libraryHash()}>Back to the Library</a></p></div>`;
  if (!g) return html`${head}<div class="pages"><p class="muted">Reading…</p></div>`;
  const removeFrom = async (c) => {
    try {
      // the collection takes it out of whichever group has it, and History can undo it (review M34)
      await api("collection_hand", { id: c.id, leave_out: [inKey(g)] });
      ui.set((x) => ({ changes: (x.changes || 0) + 1 }));
      toast(`Removed ${g.title} from ${c.name}.`, 6000);
    } catch (e) {
      toast(`Couldn't change ${c.name}: ${err(e)}`, 8000);
    }
  };
  const files = g.files || [];
  const bl = g.backlog;
  const it = { key: g.key, title: g.title, platform: g.platform };
  // its place in the backlog (D157): to play (with its place and In progress) or completed
  const backlog = html`<section class="ls-section" id="game-backlog" data-list=${bl?.list || "none"}><h2>Backlog</h2>
    <p id="game-backlog-line" class=${bl ? null : "muted"}>${!bl ? "Not in the backlog."
      : bl.list === "done" ? `Completed${bl.completed ? ` on ${dayText(bl.completed)}` : ""}.`
      : html`<b>${bl.place}</b> of ${bl.of} to play${bl.started ? html` · <span class="bl-badge">In progress</span> since ${dayText(bl.started)}` : null}`}
      ${bl ? html` <a href=${routeHash("backlog")} id="game-backlog-open">Open the backlog</a>` : null}</p>
    <div class="home-actions">${!bl ? html`<button type="button" class="ghost compact" id="game-backlog-add" disabled=${off} onClick=${() => tryIt(addToBacklog([it]))}>${Icon.list(14)} Add to the backlog</button>`
      : bl.list === "todo" ? html`<button type="button" class="ghost compact" id="game-backlog-playing" aria-pressed=${bl.started ? "true" : "false"} disabled=${off} onClick=${() => tryIt(backlogDo.playing(it, !bl.started))}>${Icon.gamepad(14)} ${bl.started ? "Not in progress" : "In progress"}</button>
        <button type="button" class="ghost compact" id="game-backlog-complete" disabled=${off} onClick=${() => tryIt(backlogDo.complete(it))}>${Icon.check(14)} Completed</button>
        <button type="button" class="ghost compact" id="game-backlog-remove" disabled=${off} onClick=${() => tryIt(backlogDo.remove(it))}>Remove from the backlog</button>`
      : html`<button type="button" class="ghost compact" id="game-backlog-again" disabled=${off} onClick=${() => tryIt(backlogDo.again(it))}>${Icon.refresh(14)} Back to the backlog</button>
        <button type="button" class="ghost compact" id="game-backlog-remove" disabled=${off} onClick=${() => tryIt(backlogDo.remove(it))}>Remove from the backlog</button>`}</div>
  </section>`;
  // the source most details came from: named once, above them, instead of under each
  const tally = {};
  for (const [k] of FIELDS) { const f = g.scraped?.[k]; if (f && f !== "you") tally[f] = (tally[f] || 0) + 1; }
  const main = Object.entries(tally).sort((a, b) => b[1] - a[1])[0]?.[0] || null;
  const dir = g.dir || "";
  return html`${head}<div class="page-with-inspector"><div class="pages game-page" id="game-page" data-key=${g.key} data-kind=${g.kind}>
    <div class="game-top">
      <${Gallery} g=${g} off=${off || busy} onChoose=${choose} />
      <section class="game-details" id="game-details" aria-busy=${busy ? "true" : "false"}>
        <p class="muted game-sub">${[g.platform_name, g.kind === "dat" ? (g.have ? "in the library" : "not in the library") : g.kind === "pc" ? "a PC game" : "a game no DAT knows", (g.set?.name || g.name) && (g.set?.name || g.name) !== g.title ? g.set?.name || g.name : null].filter(Boolean).join(" · ")}</p>
        ${g.variant ? html`<p class="game-variant-of" id="game-variant-of"><span>${g.variant.of ? html`A variant of <a href=${gameHash(g.variant.of.key)}>${g.variant.of.name}</a>` : "A game of its own, in the system's custom DAT"}</span>${g.variant.tags?.length ? html`<span class="variant-tags">${g.variant.tags.map((t) => html`<span class="variant-tag" key=${t}>${t}</span>`)}</span>` : null}</p>` : null}
        ${g.addon ? html`<p class="game-variant-of" id="game-addon-of"><span>${addonLabel(g.addon)}${g.addon.base ? html` of <a href=${gameHash(g.addon.base.key)}>${g.addon.base.title || g.addon.base.key}</a>` : ", linked to no game yet"}</span>
          ${off ? null : html`<button type="button" class="linkish" id="game-addon-relink" onClick=${() => ui.set({ dialog: { type: "addon-link", platform: g.platform, addon: g.name, from: g.addon.base?.key.slice(g.platform.length + 1) || null, onDone: reload } })}>Link to another game…</button>`}</p>` : null}
        ${g.disc_of ? html`<p class="game-variant-of" id="game-disc-of"><span>Disc ${g.disc_of.disc} of <a href=${gameHash(g.disc_of.key)}>${g.disc_of.title || g.disc_of.key}</a>${g.disc_of.discs ? `, a game on ${g.disc_of.discs} discs` : ""}</span></p>` : null}
        ${g.variant_games?.length ? html`<p class="muted" id="game-variant-list">${g.variant_games.length === 1 ? "Its variant" : "Its variants"}: ${g.variant_games.map((x, i) => html`${i ? ", " : ""}<a key=${x.key} href=${gameHash(x.key)}>${x.name}</a>`)}</p>` : null}
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
        ${main ? html`<p class="muted game-main-source" id="game-main-source">Details from ${sourceName(main)}, unless a detail says otherwise.</p>` : null}
        <div class="game-fields">${FIELDS.map(([k, l]) => html`<${Detail} key=${k} g=${g} field=${k} label=${l} off=${off || busy} main=${main} onSave=${save} onUnmark=${unmark} onUse=${use} />`)}</div>
        <p class="muted">${off ? "" : "Click a detail to change it. "}${pc ? "Its title and year name its folder, which is renamed with them; the rest is kept in the library and stays when it's identified again." : "What you change here is kept in its library folder and stays when it's scraped again."}</p>
        ${Object.keys(g.ids || {}).length ? html`<p class="muted" id="game-ids">Its ids at the sources: ${Object.entries(g.ids).map(([k, v]) => `${sourceName(k)} ${v}`).join(" · ")}</p>` : null}
      </section>
    </div>
    ${pc && g.pc ? html`<section class="ls-section game-pc" id="game-pc"><h2>PC game</h2>
      <p class="muted" id="game-pc-line">${pcLine(g.pc)}</p>
      <${PcParts} g=${g.pc} off=${off} onChange=${reload} />
    </section>` : null}
    ${g.review?.length ? html`<section class="ls-section" id="game-review"><h2>Which game is it?</h2>
      <p class="muted">A scrape wasn't sure of this one: <a href=${routeHash("metadata")}>the Metadata page</a> lists what it found, to pick from.</p></section>` : null}
    ${g.addons && (g.addons.items.length || (dat && g.addons.loose.length)) ? html`<${Addons} g=${g} dat=${dat} off=${off} onChange=${reload} />` : null}
    <section class="ls-section" id="game-docs"><h2>Documents</h2>
      ${g.docs?.length ? html`<ul class="ls-list">${g.docs.map((x) => html`<li key=${x.id} data-kind=${x.kind}><button type="button" class="linkish" onClick=${() => viewDoc(x.id)}>${x.title}</button><span class="muted">${x.kind_name}${x.source ? ` · ${sourceName(x.source)}` : ""}</span></li>`)}</ul>`
        : html`<p class="muted">None yet.${dat ? " Attach your own (or drop files on this page), or scrape its manual." : pc ? " Attach your own (or drop files on this page): a manual, a guide, a map." : ""}</p>`}
      ${(dat || pc) && isDesktop() ? html`<button type="button" class="ghost compact" id="game-attach" disabled=${off} onClick=${() => attach()}>${Icon.file(14)} Attach documents…</button>` : null}
    </section>
    ${g.magazines?.length ? html`<section class="ls-section" id="game-magazines"><h2>In magazines</h2>
      <ul class="ls-list">${g.magazines.map((x) => html`<li key=${x.path} data-path=${x.path}><span>${x.title}</span>
        <span class="mag-game-pages">${x.pages.length ? x.pages.map((p) => html`<button type="button" class="linkish" key=${p} data-page=${p} disabled=${!x.doc} title=${`Read page ${p}`} onClick=${() => viewDoc(x.doc, { page: p, find: g.title })}>p. ${p}</button>`)
          : html`<button type="button" class="linkish" disabled=${!x.doc} onClick=${() => viewDoc(x.doc, { find: g.title })}>Read it</button>`}</span></li>`)}</ul>
    </section>` : null}
    ${g.videos?.length ? html`<section class="ls-section" id="game-videos"><h2>Videos</h2>
      <ul class="ls-list">${g.videos.map((v) => html`<li key=${v.id} data-id=${v.id}><span class="op-paths">
        <button type="button" class="linkish" data-action="watch" title=${v.file ? "Watch it on the Videos page" : "Watch it in your browser"} disabled=${v.file && v.gone} onClick=${() => watch(v)}>${v.file ? Icon.video(14) : Icon.external(14)} ${v.title}</button>
        <span class="muted">${videoFacts(v)}</span></span></li>`)}</ul>
    </section>` : null}
    ${g.set ? html`<section class="ls-section game-discs" id="game-discs"><h2>Discs</h2>
      <p class="muted" id="game-discs-line">${discsLine(g.set.discs)}. They're one game: a collection, Convert and an export take every disc.</p>
      <ul class="ls-list">${g.set.discs.map((d) => html`<li key=${d.key} data-disc=${d.disc} data-have=${d.have ? (d.partial ? "part" : "yes") : "no"}>
        <span class="op-paths"><span>Disc ${d.disc}</span><span class="muted">${d.name}</span></span>
        <span class="muted disc-state">${[d.have ? (d.partial ? "there in part" : "in the library") : "missing", d.size ? size(d.size) : null].filter(Boolean).join(" · ")}</span></li>`)}</ul>
    </section>` : null}
    ${pc ? null : html`<section class="ls-section" id="game-files"><h2>Files</h2>
      ${files.length ? html`<ul class="ls-list">${files.map((f, i) => html`<li key=${i} data-missing=${f.missing ? "yes" : "no"}>
        <span class="op-paths"><span class="op-to" title=${f.path}>${dir ? f.path : shortPath(f.path, 3)}</span>
          <span class="muted">${[f.disc ? `disc ${f.disc}` : null, f.kind, f.size ? size(f.size) : null, f.roms?.length > 1 ? plural(f.roms.length, "file", "files") : null, f.missing ? "gone since the last scan" : null].filter(Boolean).join(" · ")}</span></span>
        ${isDesktop() && !f.missing && !dir ? html`<button type="button" class="ghost compact" onClick=${() => reveal(f.path)}>Show</button>` : null}</li>`)}</ul>`
        : html`<p class="muted">${g.kind === "dat" && !g.have ? "You don't have it: the audit lists what to look for." : "No files."}</p>`}
      ${dat && g.have && g.dat_id && ui.get().platforms?.find((p) => p.id === g.platform)?.formats?.length ? html`<button type="button" class="ghost compact" id="game-convert" disabled=${off} onClick=${() => convertDialog({ dat: g.dat_id, platform: g.platform, platformName: g.platform_name, games: [g.name] })}>${Icon.convert(14)} Convert…</button>` : null}
      ${dat ? html`<dl class="pc-facts" id="game-dat">
        <dt>In the DAT</dt><dd>${g.dat || ""}</dd>
        <dt>Name</dt><dd>${g.name}</dd>
        ${g.regions ? html`<dt>Regions</dt><dd>${g.regions}</dd>` : null}
        <dt>Files</dt><dd>${(g.roms || []).length}</dd></dl>` : null}
    </section>`}
    ${pc ? null : html`<${GameSaves} gameKey=${g.disc_of?.key || g.key} off=${off} />`}
    ${dat ? html`<section class="ls-section" id="game-collections"><h2>Collections</h2>
      ${g.collections?.length ? html`<ul class="ls-list">${g.collections.map((c) => html`<li key=${c.id} data-id=${c.id}><a href=${routeHash("collections")}>${c.name}</a>
        <button type="button" class="ghost compact" data-action="remove-from" disabled=${off} title=${`Remove it from ${c.name}`} onClick=${() => removeFrom(c)}>Remove from collection</button></li>`)}</ul>` : html`<p class="muted">In none.</p>`}
      <button type="button" class="ghost compact" id="game-add" disabled=${off} onClick=${() => ui.set({ dialog: { type: "lib-add", games: [{ key: inKey(g), kind: g.kind, platform: g.platform }] } })}>${Icon.collection(14)} Add to a collection…</button>
    </section>` : null}
    ${backlog}
  </div>
  <${Inspector} page="game" label="Pictures" open=${false} tabs=${[{ id: "media", label: "Pictures", icon: "image", body: html`<${MediaPane} g=${g} off=${off || busy} onChoose=${choose} />` }]} /></div>`;
}
