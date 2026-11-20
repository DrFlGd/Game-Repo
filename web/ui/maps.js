// Maps and Guides (docs/PLAN.md, D214–D221): the packs Atlas Importer writes (`.atlaspack`
// files of a game's maps, guides and wiki pages, read by Atlas), kept in the library's
// Maps and Guides folder, each linked to its games. The page is laid out as Videos is
// (shelf.js), grouped by game: a card shows the pack's cover (or the picture chosen for
// it, or a plain card with its mark), its game, its site, its counts and how old its pages
// are. A pack with no game is on To review, with the closest games offered. A pack is
// opened in Atlas (D221), linked to a game or taken off one (its file moves to match, as a
// change with Undo), or set aside. Import… takes `.atlaspack` files (D220). Game Repo never
// draws a map: the small pictures are the pack's own.
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl, undoable } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar } from "./layout.js";
import { Dialog, More, close } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs, undoChange } from "./organise.js";
import { importFrom } from "./importpage.js";
import { gameHash, systemColour } from "./browse.js";
import { useDroppedFiles } from "./documents.js";
import { Icon } from "./icons.js";
import { Shelf, useShelfState, Pic, PanelActions, askGame, byName } from "./shelf.js";
import { guidebookItem, guidebookTile, guidebookFacts, GuidebookPanel, guidebookActions, importGuidebooks } from "./guidebooks.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const fail = (e) => toast(err(e), 8000);

/** What a pack is to the page: linked to a game, on To review, or left with no game. */
const STATUS = { linked: "Linked to a game", review: "To review", nogame: "Left with no game", unreadable: "Can't be read" };
const status = (p) => (p.unreadable ? "unreadable" : p.games?.length ? "linked" : p.left || p.guidebook ? "nogame" : "review");
const KIND = { map: "Maps", guide: "Guides", wiki: "Wiki pages", guidebook: "Guidebooks" };
const DETAIL = { overview: "Overview", standard: "Standard", full: "Full" };

/** How long ago an instant was, in plain words: "today", "3 days ago", "2 months ago". */
export function howLong(iso) {
  const t = Date.parse(iso || "");
  if (!Number.isFinite(t)) return null;
  const days = Math.floor((Date.now() - t) / 86400000);
  if (days < 1) return "today";
  if (days === 1) return "yesterday";
  if (days < 60) return `${days} days ago`;
  if (days < 730) return `${Math.round(days / 30.4)} months ago`;
  return `${Math.round(days / 365.25)} years ago`;
}

const day = (iso) => (iso ? String(iso).slice(0, 10) : null);
/** "1 location", "1,840 locations". */
const many = (n, one, more) => `${Number(n).toLocaleString()} ${n === 1 ? one : more}`;
const pagesOf = (list) => list.reduce((n, g) => n + (g.pages || 0), 0);

/** A pack's counts: its maps, locations, guides' and wiki pages (from its description). */
export function packCounts(p) {
  const c = p.contents || {};
  const guides = (p.guides || []).filter((g) => g.kind !== "wiki");
  const wikis = (p.guides || []).filter((g) => g.kind === "wiki");
  const maps = p.maps?.length || c.maps || 0;
  return [
    maps ? plural(maps, "map", "maps") : null,
    c.locations ? many(c.locations, "location", "locations") : null,
    guides.length ? plural(pagesOf(guides) || guides.length, "guide page", "guide pages") : !p.guides?.length && c.guides ? plural(c.guides, "guide page", "guide pages") : null,
    wikis.length ? plural(pagesOf(wikis) || wikis.length, "wiki page", "wiki pages") : null,
  ].filter(Boolean);
}

/** Where a pack is from, by the contract's rule (`Description::from_words`): its site, "Made in Atlas", "Made in Atlas Map Maker" or "Came with an app". */
export const packFrom = (p) => p.from || null;

/** What's said under a pack's title: where it's from, its counts, detail and size. */
export const packFacts = (p) => [packFrom(p), ...packCounts(p), DETAIL[p.detail] ? `${DETAIL[p.detail]} detail` : null, size(p.size)].filter(Boolean).join(" · ");

/** How old its pages are (`source.newestPageAt`), and when it last changed (`pack.updated`). */
const ageLine = (p) => [p.source?.newest_page_at ? `Read ${howLong(p.source.newest_page_at)}` : null, p.updated ? `changed ${day(p.updated)}` : null].filter(Boolean).join(", ");

/** Open in Atlas (D221): the program set in Settings › Tools, started with the pack's path alone; else, on Windows, the one
 *  Windows opens packs with (elsewhere the backend says to set Atlas in Settings › Tools). */
export async function openInAtlas(p) {
  try {
    const r = await api("maps_open", { path: p.path });
    if (r.open_with) await ctx.platform.library.openPath(r.open_with);
    toast(`Opening ${p.title || p.name} in Atlas.`, 4000);
  } catch (e) {
    fail(e);
  }
}

/** A planned change ({ plan } or { nothing }) run with the preview's Undo. */
async function run(r) {
  if (r?.plan) return runPlan(r.plan).then(again);
  again();
  return null;
}

const linkTo = (p, also = false) => askGame({
  title: also ? "Link it to a second game" : "Link it to a game",
  button: "Link it",
  note: also ? "It stays in its first game's folder, and shows on both games' pages." : "Its file moves into the game's folder in Maps and Guides; Undo puts it back.",
  run: async (g) => { if (g) await run(await api("maps_link", { id: p.id, key: g.key, also })); },
});

const takeOff = (p, key = null) => api("maps_unlink", { id: p.id, key }).then(run, fail);
const leave = (p, left) => api("maps_leave", { id: p.id, left }).then(again, fail);

async function removePacks(paths) {
  try {
    const r = await api("maps_remove", { paths });
    if (r.plan) ui.set({ dialog: { type: "organise", plan: r.plan } });
  } catch (e) {
    toast(`Couldn't plan it: ${err(e)}`, 8000);
  }
}

async function choosePicture(p) {
  const at = await ctx.platform.library.pickFile(`A picture for ${p.title}`, ["jpg", "jpeg", "png", "webp"]);
  if (!at) return;
  try {
    await api("maps_picture", { id: p.id, picture: at });
    again();
  } catch (e) {
    fail(e);
  }
}

// ------------------------------------------------------------ Import (D220)

const STATE = {
  new: null,
  newer: "Newer than the copy kept, which it replaces (set aside, with Undo).",
  same: "The library has this pack already, just as new: it isn't brought in.",
  older: "The library has a newer copy of this pack: it isn't brought in.",
};

/** Import packs…: the files picked read into a review list, then planned and run. */
function ImportDialog({ paths }) {
  const [items, setItems] = useState(null);
  const [progress, setProgress] = useState("");
  const [how, setHow] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let gone = false;
    (async () => {
      try {
        const { job } = await api("maps_items", { paths });
        const done = await followJob(job, "Reading the packs", (j) => { if (!gone) setProgress(j.progress?.text || ""); });
        if (gone) return;
        if (done.error) throw new Error(done.error);
        setItems(done.result.items);
      } catch (e) {
        if (!gone) setError(err(e));
      }
    })();
    return () => { gone = true; };
  }, []);
  const set = (path, patch) => setItems((all) => all.map((it) => (it.path === path ? { ...it, ...patch } : it)));
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("maps_plan", { items, ...howArgs(how) }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="maps-import-dialog" wide=${true} sticky=${true} onSubmit=${() => { close(); runPlan(plan).then(again); }} busy=${plan.short || !plan.ops} submitLabel="Import"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">They go into Maps and Guides' <b>No game yet</b> folder, and those whose game is certain are linked to it and moved into its folder straight after (a change of its own, with Undo). ${how !== "move" ? "Your files stay where they are." : "The files are moved; Undo puts them back."}</p>
    <//>`;
  }
  const included = (items || []).filter((i) => i.include).length;
  return html`<${Dialog} title="Import maps and guides" id="maps-import-dialog" wide=${true} sticky=${!items || items.length > 0} onSubmit=${preview} busy=${busy || !items || !included} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${!items ? html`<p class="muted" id="maps-reading">Reading the packs${progress ? html` (${progress})` : null}…</p>` : !items.length ? html`<p class="muted" id="maps-none-found">No .atlaspack files there.</p>` : html`
      <ul class="ls-list pc-items" id="maps-items">${items.map((it) => html`<li key=${it.path} class="pc-item" data-name=${it.name} data-state=${it.state} data-include=${it.include ? "yes" : "no"}>
        <label class="check-row pc-item-include"><input type="checkbox" data-action="include" aria-label=${`Import ${it.name}`} checked=${it.include} disabled=${it.state === "same" || it.state === "older"} onChange=${(e) => set(it.path, { include: e.target.checked })} /></label>
        <div class="pc-item-main">
          <span class="pc-item-name" title=${it.path}>${it.title || it.name} <span class="muted">${[it.from, it.own_things, size(it.size)].filter(Boolean).join(" · ")}</span></span>
          ${it.to_name && it.to_name !== it.name ? html`<span class="muted maps-item-to">Named ${it.to_name}</span>` : null}
          ${it.error ? html`<span class="warn-note maps-item-error">${it.error} It's brought in all the same, and read once Game Repo can.</span>` : null}
          ${STATE[it.state] ? html`<span class="muted maps-item-state">${STATE[it.state]}</span>` : null}
        </div></li>`)}</ul>
      <${HowControl} id="maps-import-mode" value=${how} onChange=${setHow} />
      <p class="muted"><b>${plural(included, "pack", "packs")}</b> to import. Each is matched to its game by the title in it; those that aren't certain wait on To review.</p>`}
  <//>`;
}

export function MapsDialogs({ dialog }) {
  if (dialog.type === "maps-import") return html`<${ImportDialog} key=${dialog.paths.join("|")} ...${dialog} />`;
  return null;
}

/** Import maps and guides… (`.atlaspack` files picked, a folder of them, or `given`, dropped). */
export async function importMaps(folder = false, given = null) {
  const lib = ctx.platform.library;
  const paths = given || (folder ? [await lib.pickFolder("A folder of maps and guides (.atlaspack)")].filter(Boolean) : (await lib.pickFiles("Maps and guides (.atlaspack)")) || []);
  if (paths.length) ui.set({ dialog: { type: "maps-import", paths } });
}

// ------------------------------------------------------------ the page (D219)

const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
// by its packId first, so a card drawn before its pack moved still finds it
const atlasPic = (p, n) => (n != null && p.path ? libraryUrl(`~atlas/${n}/${encodeURIComponent(p.id)}/${p.path}`) : null);
/** A pack's pictures: the one chosen for it, else its own cover; else a plain card with its mark. */
const pics = (p, px) => [thumb(p.picture_path, px), atlasPic(p, p.cover_n)];
const noPic = (p) => ({ icon: p.unreadable ? "alert" : p.kinds?.[0] === "guide" ? "book" : p.kinds?.[0] === "wiki" ? "wiki" : "map", colour: systemColour(p.games?.[0]?.platform || "maps") });
const gameTitle = (p) => p.games?.[0]?.title || p.game_title || p.title || p.name;

const SORTS = [
  ["title", "Title", (a, b) => byName(a.title || a.name, b.title || b.name)],
  ["game", "Game", (a, b) => byName(gameTitle(a), gameTitle(b))],
  ["updated", "Changed", (a, b) => byName(b.updated || "", a.updated || "")],
  ["read", "Read", (a, b) => byName(b.source?.newest_page_at || "", a.source?.newest_page_at || "")],
  ["size", "Size", (a, b) => (b.size || 0) - (a.size || 0)],
  ["added", "Added", (a, b) => byName(b.added || "", a.added || "")],
];
const GROUPS = [["game", "Game"], ["", "None"], ["is", "Is"], ["source", "Site"], ["system", "System"]];
const COLUMNS = [
  ["title", "Title", (p) => p.title || p.name, "title"],
  ["game", "Game", (p) => (p.games || []).map((g) => g.title).join(", ") || STATUS[status(p)], "game"],
  ["source", "Site", (p) => packFrom(p) || "", null],
  ["counts", "Has", (p) => (p.unreadable ? p.error : packCounts(p).join(", ")), null],
  ["read", "Read", (p) => howLong(p.source?.newest_page_at) || "", "read"],
  ["size", "Size", (p) => size(p.size), "size"],
];
const COLS = "minmax(160px, 3fr) minmax(120px, 2fr) minmax(90px, 1.3fr) minmax(120px, 2fr) 104px 72px";

/** A To review pack's closest games, each a click from linking it. */
function Suggestions({ p, off }) {
  const list = p.suggestions || [];
  return html`<div class="maps-review" id="maps-review" data-id=${p.id}>
    <p class="muted">${p.game_title ? html`It's for <b>${p.game_title}</b>${p.game?.system ? html` (${p.game.system}, as its site says)` : null}, and no game in the Library is certainly that one.` : "It doesn't say which game it's for."} Pick the game, or leave it in No game yet.</p>
    ${list.length ? html`<ul class="ls-list maps-suggestions" id="maps-suggestions">${list.map((g) => html`<li key=${g.key} data-key=${g.key}>
      <span>${g.title} <span class="muted">${g.platform_name}${g.have ? "" : " · not in your library"}</span></span>
      <button type="button" class="ghost compact" data-action="link" disabled=${off} onClick=${() => api("maps_link", { id: p.id, key: g.key }).then(run, fail)}>Link</button></li>`)}</ul>` : html`<p class="muted">No game's title is like it.</p>`}
    <div class="home-actions">
      <button type="button" class="ghost compact" id="maps-review-other" disabled=${off} onClick=${() => linkTo(p)}>${Icon.gamepad(14)} Another game…</button>
      <button type="button" class="ghost compact" id="maps-review-leave" disabled=${off} onClick=${() => leave(p, true)}>Leave it in No game yet</button>
    </div>
  </div>`;
}

/** One pack in the panel: its picture, what it has (its maps with their small pictures, its
 *  guides and wikis with their page counts), its games, and its other copies. */
function PackPanel({ p, off, actions }) {
  if (p.unreadable) {
    return html`<div class="shelf-panel" id="maps-edit" data-id=${p.id}>
      <${Pic} srcs=${[]} none=${noPic(p)} cls="lib-panel-cover" px=${64} />
      <h2 id="maps-panel-title">${p.name}</h2>
      <p class="warn-note" id="maps-panel-error">${p.error}</p>
      <p class="muted">${[p.file, size(p.size)].join(" · ")}</p>
      <${PanelActions} page="maps" actions=${actions} />
    </div>`;
  }
  const guides = (p.guides || []).filter((g) => g.kind !== "wiki");
  const wikis = (p.guides || []).filter((g) => g.kind === "wiki");
  const c = p.contents || {};
  return html`<div class="shelf-panel" id="maps-edit" data-id=${p.id}>
    <${Pic} srcs=${pics(p, 512)} none=${noPic(p)} cls="lib-panel-cover" px=${64} />
    <h2 id="maps-panel-title">${p.title || p.name}</h2>
    <p class="muted" id="maps-panel-facts">${packFacts(p)}</p>
    ${ageLine(p) ? html`<p class="muted" id="maps-panel-age">${ageLine(p)}</p>` : null}
    ${p.own_things ? html`<p class="muted" id="maps-panel-mine">${p.own_things}.</p>` : null}
    ${p.personal_use ? html`<p class="muted" id="maps-panel-own-use">For your own use.</p>` : null}
    <${PanelActions} page="maps" actions=${actions} />
    ${status(p) === "review" ? html`<${Suggestions} p=${p} off=${off} />` : null}
    <h3>Games</h3>
    ${p.games?.length ? html`<ul class="ls-list shelf-games" id="maps-games">${p.games.map((g) => html`<li key=${g.key} data-key=${g.key}><a href=${gameHash(g.key)}>${g.title}</a>${" "}<span class="muted">${g.platform_name}</span>
      <button type="button" class="ghost compact" data-action="take-off" disabled=${off} title="Take it off this game" onClick=${() => takeOff(p, g.key)}>Take off</button></li>`)}</ul>`
      : html`<p class="muted">${p.left ? "None: it's left in No game yet." : "None yet."}${p.left ? html` <button type="button" class="linkish" id="maps-back-to-review" disabled=${off} onClick=${() => leave(p, false)}>Put it on To review</button>` : null}</p>`}
    ${p.maps?.length ? html`<h3>Maps</h3><ul class="ls-list maps-maps" id="maps-maps">${p.maps.map((m, i) => html`<li key=${m.id || i} data-map=${m.id || i}>
      <${Pic} srcs=${[atlasPic(p, m.n)]} none=${{ icon: "map", colour: "var(--graphite)" }} cls="maps-map-pic" px=${16} />
      <span class="maps-map-text"><span class="maps-map-name">${m.name || `Map ${i + 1}`}</span>
        <span class="muted">${[m.width && m.height ? `${m.width} × ${m.height}` : null, m.locations ? many(m.locations, "location", "locations") : null].filter(Boolean).join(" · ")}</span></span></li>`)}</ul>`
      : c.maps ? html`<h3>Maps</h3><p class="muted" id="maps-maps-count">${plural(c.maps, "map", "maps")}${c.locations ? `, ${many(c.locations, "location", "locations")}` : ""}.</p>` : null}
    ${guides.length || wikis.length ? html`<h3>Guides and wikis</h3><ul class="ls-list maps-guides" id="maps-guides">${[...guides, ...wikis].map((g, i) => html`<li key=${i} data-kind=${g.kind || "guide"}>
      <span>${g.kind === "wiki" ? Icon.wiki(14) : Icon.book(14)} ${g.title || (g.kind === "wiki" ? "A wiki" : "A guide")}</span>
      <span class="muted">${g.pages ? plural(g.pages, "page", "pages") : ""}</span></li>`)}</ul>`
      : c.guides ? html`<h3>Guides</h3><p class="muted" id="maps-guides-count">${plural(c.guides, "guide page", "guide pages")}.</p>` : null}
    ${p.copies?.length ? html`<h3>Other copies</h3><ul class="ls-list maps-copies" id="maps-copies">${p.copies.map((x) => html`<li key=${x.path} data-path=${x.path}>
      <span><span class="pc-badge">Another copy</span> ${x.file} <span class="muted">${[x.updated ? `changed ${day(x.updated)}` : null, size(x.size)].filter(Boolean).join(" · ")}</span></span>
      <button type="button" class="ghost compact" data-action="remove-copy" disabled=${off} onClick=${() => removePacks([x.file])}>Remove it…</button></li>`)}</ul>
      <p class="muted">An older file of the same pack. The newer one is the one shown here.</p>` : null}
    <p class="muted maps-file" title=${p.path}>${p.file}${p.version ? ` · format ${p.version}` : ""}${p.written_by ? ` · written by ${p.written_by}` : ""}</p>
  </div>`;
}

export function Maps() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [reading, setReading] = useState(false);
  const [st, set] = useShelfState("maps", { sort: "title", group: "game" });
  // files dropped here are imported, as packs (D78)
  useDroppedFiles(() => "maps");
  // with the guidebooks (D233), each a card of its own among the packs
  const reload = () => Promise.all([api("maps"), api("guidebooks").catch(() => ({ guidebooks: [] }))])
    .then(([d, gb]) => { setData({ ...d, guidebooks: gb.guidebooks || [] }); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => {
    if (!s.library) return;
    reload();
  }, [s.changes, s.library?.path]);
  // the folder read again when the page opens (D217), and the sure matches linked (D218)
  const look = async (said = false) => {
    setReading(true);
    try {
      const { job } = await api("maps_sync", {});
      const done = await followJob(job, "Reading Maps and Guides");
      if (done.error) throw new Error(done.error);
      const l = done.result?.maps?.linked;
      if (l?.id) undoable(`${l.label}.`, () => undoChange(l.id), 10000);
      else if (said) toast(`Looked again: ${plural(done.result?.maps?.packs || 0, "pack", "packs")}.`, 4000);
      again();
    } catch (e) {
      fail(e);
    } finally {
      setReading(false);
    }
  };
  useEffect(() => { if (s.library && !off) look(); }, [s.library?.path]);
  const packs = useMemo(() => [
    ...(data?.packs || []),
    ...(data?.unreadable || []).map((f) => ({ ...f, unreadable: true, title: f.name, games: [], kinds: [] })),
    ...(data?.guidebooks || []).map(guidebookItem),
  ], [data]);
  const names = useMemo(() => {
    const m = { system: new Map(), game: new Map() };
    for (const p of packs) for (const g of p.games) { m.system.set(g.platform, g.platform_name); m.game.set(g.title, g.title); }
    return m;
  }, [packs]);
  const fields = [
    { id: "is", name: "Is", of: (p) => [status(p), p.personal_use ? "own" : null, p.copies?.length ? "copies" : null], label: (x) => STATUS[x] || (x === "own" ? "For your own use" : x === "copies" ? "Has another copy" : x) },
    { id: "game", name: "Game", of: (p) => (p.games?.length ? p.games.map((g) => g.title) : []), long: true, none: "No game yet", groupOrder: "name" },
    { id: "system", name: "System", of: (p) => (p.games || []).map((g) => g.platform), label: (x) => names.system.get(x) || x, none: "No game yet" },
    { id: "source", name: "Site", of: (p) => [packFrom(p)], long: true, none: "No site given" },
    { id: "kind", name: "Has", of: (p) => p.kinds || [], label: (k) => KIND[k] || k },
    { id: "detail", name: "Detail", of: (p) => [p.detail], label: (d) => DETAIL[d] || d },
  ];
  const actions = (list) => {
    if (list.length && list.every((p) => p.guidebook)) return guidebookActions(list, off);
    list = list.filter((p) => !p.guidebook);
    const one = list.length === 1 ? list[0] : null;
    const pack = one && !one.unreadable ? one : null;
    const desk = isDesktop();
    return [
      { id: "open", label: "Open in Atlas", icon: "map", one: true, disabled: !pack || !desk, run: () => pack && openInAtlas(pack) },
      { id: "link", label: pack?.games?.length ? "Link to another game…" : "Link to a game…", icon: "gamepad", one: true, disabled: off || !pack, run: () => pack && linkTo(pack) },
      ...(pack?.games?.length ? [
        { id: "link-also", label: "Link to a second game…", icon: "plus", one: true, disabled: off, run: () => linkTo(pack, true) },
        { id: "take-off", label: pack.games.length > 1 ? "Take off every game" : "Take off the game", icon: "close", one: true, disabled: off, run: () => takeOff(pack) },
      ] : []),
      { id: "picture", label: pack?.picture_path ? "Choose another picture…" : "Choose a picture…", icon: "image", one: true, disabled: off || !pack || !desk, run: () => pack && choosePicture(pack) },
      ...(pack?.picture_path ? [{ id: "picture-off", label: "Take its picture off", icon: "close", one: true, disabled: off, run: () => api("maps_picture", { id: pack.id, picture: null }).then(again, fail) }] : []),
      { id: "show", label: "Show in folder", icon: "folder", one: true, disabled: !desk || !one, run: () => one && reveal(`${s.library.path}/${one.path}`) },
      { id: "remove", label: "Remove…", icon: "trash", disabled: off, run: () => removePacks(list.flatMap((p) => [p.file, ...(p.copies || []).map((x) => x.file)])) },
    ];
  };
  const review = packs.filter((p) => status(p) === "review");
  const more = () => [
    ...(packs.length ? [{ id: "maps-import-more", label: "Import maps and guides…", icon: "inbox", disabled: off || !isDesktop(), run: () => importFrom("import-maps", () => importMaps()) }] : []),
    { id: "maps-import-guidebooks", label: "Import guidebooks…", icon: "book", disabled: off || !isDesktop(), run: () => importGuidebooks() },
    { id: "maps-look-again", label: "Look again", icon: "refresh", disabled: off || reading, run: () => look(true) },
    { id: "maps-show-folder", label: "Show the folder", icon: "folder", disabled: !isDesktop() || !data?.folder, run: () => ctx.platform.library.openPath(`${s.library.path}/${data.folder}`) },
  ];
  const primary = html`<button type="button" class="primary compact" id="maps-import" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-maps", () => importMaps())}>${Icon.inbox(15)} Import maps and guides…</button>`;
  if (!s.library) return html`<${CommandBar} id="maps-head" title="Maps and Guides" /><div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const filter = (v) => set({ f: { ...(st.f || {}), is: [v] } });
  const top = review.length ? html`<div class="maps-to-review" id="maps-to-review" data-count=${review.length}>
    ${Icon.info(15)} <span><b>${plural(review.length, "pack", "packs")} to review:</b> ${review.length === 1 ? "its game isn't certain" : "their games aren't certain"}. Pick ${review.length === 1 ? "it" : "one"} to link it to its game, or leave it in No game yet.</span>
    <button type="button" class="ghost compact" id="maps-show-review" onClick=${() => filter("review")}>Show ${review.length === 1 ? "it" : "them"}</button>
  </div>` : null;
  const empty = html`<div class="library-settings" id="maps-empty">
    <p class="page-lead">Maps, guides and wiki pages of your games, as packs made by Atlas Importer and read by Atlas.</p>
    <p class="muted">Point Atlas Importer at the library's <code>${data?.folder || "Maps and Guides"}/No game yet</code> folder: each pack it writes there is read, linked to its game when the title is certain, and moved into that game's folder. Packs you have already come in with Import maps and guides… (on the <a href=${routeHash("import")}>Import</a> page too). Atlas opens them; Game Repo keeps them with your games.</p>
    <div class="home-actions"><button type="button" class="ghost compact" id="maps-show-folder-empty" disabled=${!isDesktop() || !data?.folder} onClick=${() => ctx.platform.library.openPath(`${s.library.path}/${data.folder}`)}>${Icon.folder(14)} Show the folder</button></div>
  </div>`;
  return html`<${Shelf} page="maps" noun=${["pack", "packs"]} title="Maps and Guides" items=${data ? packs : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} ratio="4 / 3" columns=${COLUMNS} cols=${COLS}
    text=${(p) => [p.title, p.name, p.game_title, p.source_name, ...(p.games || []).map((g) => g.title), ...(p.maps || []).map((m) => m.name), ...(p.guides || []).map((g) => g.title)].join(" ")}
    tile=${(p) => p.guidebook ? guidebookTile(p) : ({ pics: pics(p, 512), none: noPic(p), badge: { text: p.unreadable ? "Can't be read" : p.games?.length ? p.games[0].platform_name : STATUS[status(p)], colour: systemColour(p.games?.[0]?.platform || "maps") }, title: p.title || p.name, meta: p.unreadable ? p.error : [gameTitle(p) !== p.title ? gameTitle(p) : null, packFrom(p), packCounts(p)[0]].filter(Boolean).join(" · "), missing: p.unreadable })}
    line=${(p) => p.guidebook ? { ...guidebookTile(p), meta: guidebookFacts(p) } : ({ pics: pics(p, 256), none: noPic(p), badge: { text: p.games?.length ? p.games.map((g) => g.title).slice(0, 2).join(", ") : STATUS[status(p)] }, title: p.title || p.name, meta: p.unreadable ? p.error : [packFacts(p), ageLine(p)].filter(Boolean).join(" · "), missing: p.unreadable })}
    actions=${actions} onOpen=${(p) => p.guidebook ? isDesktop() && ctx.platform.library.openPath(`${s.library.path}/${p.path}`) : !p.unreadable && openInAtlas(p)} top=${top} primary=${packs.length ? null : primary} more=${more} moreTitle="Import, look again, tile size" empty=${empty}
    panel=${(p) => p.guidebook ? html`<${GuidebookPanel} g=${p} off=${off} />` : html`<${PackPanel} p=${p} off=${off} actions=${actions([p]).filter((a) => ["open", "link", "take-off", "picture", "show", "remove"].includes(a.id))} />`} />`;
}

/** A game's packs on its page (D219): each with Open in Atlas. */
export function GameMaps({ g }) {
  if (!g.maps?.length) return null;
  return html`<section class="ls-section" id="game-maps"><h2>Maps and guides</h2>
    <ul class="ls-list">${g.maps.map((p) => html`<li key=${p.id} data-id=${p.id}><span class="op-paths">
      <span>${Icon.map(14)} ${p.title || p.name}</span>
      <span class="muted">${packFacts(p)}${ageLine(p) ? ` · ${ageLine(p)}` : ""}${p.own_things ? ` · ${p.own_things}` : ""}</span></span>
      <button type="button" class="ghost compact" data-action="open-atlas" disabled=${!isDesktop()} onClick=${() => openInAtlas(p)}>Open in Atlas</button></li>`)}</ul>
    <p class="muted"><a href=${routeHash("maps")}>Maps and Guides</a> has every pack.</p>
  </section>`;
}
