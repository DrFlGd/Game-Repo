// Videos (docs/PLAN.md, D146, D153): walkthroughs, reviews, retrospectives and others, kept
// in the library's Videos/<Kind>/ folders or as web links, each with the games it's about.
// Import videos… reads a folder into a review list (each one's title, kind and date from its
// name, its games found in its title, put right here) and brings them in as a change with a
// preview and Undo; Add a link… keeps a web video, opened in the browser. The page is laid
// out as the Library is (shelf.js): filtered by kind, series, creator, tag, system, game and
// year, grouped and sorted, as tiles, lines or a table; what's picked is changed together
// (its series, creator, kind, tags, games) or one at a time in the panel. A file plays above
// the list (`#/videos/<id>`, video.js); a game's page lists its videos.
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, EditBox } from "./layout.js";
import { Dialog, More, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs } from "./organise.js";
import { importFrom } from "./importpage.js";
import { gameHash, systemColour } from "./browse.js";
import { Video } from "./video.js";
import { useDroppedFiles } from "./documents.js";
import { Icon } from "./icons.js";
import { useRouteArg, argHash } from "./panels.js";
import { Shelf, useShelfState, Pic, PanelActions, TagEdit, askValue, askGame, byName } from "./shelf.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
/** The kinds: id, name, its folder's name. */
const KINDS = [["walkthrough", "Walkthrough", "Walkthroughs"], ["review", "Review", "Reviews"], ["retrospective", "Retrospective", "Retrospectives"], ["other", "Other", "Other"]];

/** A length in seconds, as "1:02:03" or "4:05". */
export function duration(s) {
  if (!s) return null;
  const two = (n) => String(n).padStart(2, "0");
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return h ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

/** What's said under a video's title: its kind, date, length, source and size. */
export const videoFacts = (v) => [v.kind_name, v.date, duration(v.length), v.source, v.file ? (v.gone ? "its file is gone" : size(v.size)) : "a link"].filter(Boolean).join(" · ");

/** Watch a video: a file on the Videos page, a link in the browser. */
export function watch(v) {
  if (v.file) location.hash = argHash("videos", v.id);
  else ctx.platform.openUrl(v.url);
}

const KindSelect = ({ cls, id, value, off, onChange, guess = false }) => html`<select class=${`compact ${cls || ""}`} id=${id} aria-label="Kind" value=${value} disabled=${off} onChange=${(e) => onChange(e.target.value)}>
  ${guess ? html`<option value="">From its title</option>` : null}
  ${KINDS.map(([k, name]) => html`<option key=${k} value=${k}>${name}</option>`)}</select>`;

/** Import videos…: the files picked read into a review list, then planned and run. */
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
        const { job } = await api("video_items", { paths });
        const done = await followJob(job, "Reading the videos", (j) => {
          const p = j.progress || {};
          if (!gone) setProgress(p.items ? `${p.item} of ${p.items}: ${p.name || ""}` : p.text || "");
        });
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
      setPlan(await api("video_plan", { items, ...howArgs(how) }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="video-import-dialog" wide=${true} sticky=${true} onSubmit=${() => { close(); runPlan(plan).then(again); }} busy=${plan.short || !plan.ops} submitLabel="Import"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Each goes into its kind's folder in the library. ${how !== "move" ? "Your files stay where they are." : "The files are moved; Undo puts them back."}</p>
      <${More} id="video-import-more"><p>That's <code>Videos/${"<kind>"}/</code>. Each video is named by its title, and listed in videos.json.</p><//>
    <//>`;
  }
  const included = (items || []).filter((i) => i.include).length;
  return html`<${Dialog} title="Import videos" id="video-import-dialog" wide=${true} sticky=${!items || items.length > 0} onSubmit=${preview} busy=${busy || !items || !included} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${!items ? html`<p class="muted" id="video-reading">Reading the files${progress ? html` (${progress})` : null}…</p>` : !items.length ? html`<p class="muted" id="video-none-found">No MP4, M4V, MOV, WebM, MKV or AVI files there.</p>` : html`
      <ul class="ls-list pc-items video-items" id="video-items">${items.map((it) => html`<li key=${it.path} class="pc-item" data-name=${it.name} data-include=${it.include ? "yes" : "no"}>
        <label class="check-row pc-item-include"><input type="checkbox" data-action="include" aria-label=${`Import ${it.name}`} checked=${it.include} onChange=${(e) => set(it.path, { include: e.target.checked })} /></label>
        <div class="pc-item-main">
          <span class="pc-item-name" title=${it.path}>${it.name} <span class="muted">${[duration(it.length), size(it.size)].filter(Boolean).join(" · ")}</span>${it.have ? html` <span class="pc-badge">Have it</span>` : null}</span>
          <span class="pc-item-edit">
            <input type="text" class="compact video-item-title" aria-label="Title" placeholder="Title" value=${it.title} disabled=${!it.include} onInput=${(e) => set(it.path, { title: e.target.value })} />
            <${KindSelect} cls="video-item-kind" value=${it.kind} off=${!it.include} onChange=${(k) => set(it.path, { kind: k })} />
            <input type="text" class="compact video-item-date" aria-label="Date (YYYY-MM-DD)" placeholder="YYYY-MM-DD" value=${it.date || ""} disabled=${!it.include} onInput=${(e) => set(it.path, { date: e.target.value || null })} />
          </span>
          <span class="video-item-games">${it.games.length ? it.games.map((g) => html`<span class="pc-badge video-chip" key=${g.key} data-key=${g.key}>${g.title}${" "}<span class="muted">${g.platform_name}</span>
            <button type="button" class="linkish" data-action="drop-game" aria-label=${`It isn't about ${g.title}`} title="It isn't about this game" disabled=${!it.include} onClick=${() => set(it.path, { games: it.games.filter((x) => x.key !== g.key) })}>×</button></span>`)
            : html`<span class="muted">No game's title is in its name: add its games on the Videos page.</span>`}</span>
        </div></li>`)}</ul>
      <${HowControl} id="video-import-mode" value=${how} onChange=${setHow} />
      <p class="muted"><b>${plural(included, "video", "videos")}</b> to import. The title, kind and date come from each file's name, and the games from its title: put them right before the preview.</p>`}
  <//>`;
}

/** Add a link…: a video on the web, kept by its address and opened in the browser. */
function LinkDialog() {
  const [f, setF] = useState({ url: "", title: "", kind: "", date: "" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const set = (patch) => setF((x) => ({ ...x, ...patch }));
  const add = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await api("video_link", { url: f.url, title: f.title, kind: f.kind || null, date: f.date });
      close();
      again();
      location.hash = routeHash("videos");
      toast(`Added ${f.title.trim()}.`, 5000);
      return r;
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  return html`<${Dialog} title="Add a link" id="video-link-dialog" onSubmit=${add} busy=${busy || !f.url.trim() || !f.title.trim()} error=${error} submitLabel="Add">
    <label class="field-block">Its address<input type="url" id="video-link-url" placeholder="https://" value=${f.url} onInput=${(e) => set({ url: e.target.value })} /></label>
    <label class="field-block">Title<input type="text" id="video-link-title" value=${f.title} onInput=${(e) => set({ title: e.target.value })} /></label>
    <div class="pick-row">
      <label class="field-block">Kind <${KindSelect} id="video-link-kind" value=${f.kind} guess=${true} onChange=${(k) => set({ kind: k })} /></label>
      <label class="field-block">Date<input type="text" class="compact" id="video-link-date" placeholder="YYYY-MM-DD" value=${f.date} onInput=${(e) => set({ date: e.target.value })} /></label>
    </div>
    <p class="muted">The games whose titles are in its title are added to it; change them on the Videos page.</p>
  <//>`;
}

export function VideoDialogs({ dialog }) {
  if (dialog.type === "video-import") return html`<${ImportDialog} key=${dialog.paths.join("|")} ...${dialog} />`;
  if (dialog.type === "video-link") return html`<${LinkDialog} />`;
  return null;
}

/** Import videos… from a folder (or `files`, picked one by one, or `given`, dropped). */
export async function importVideos(files = false, given = null) {
  const lib = ctx.platform.library;
  const paths = given || (files ? (await lib.pickFiles("Videos (MP4, M4V, MOV, WebM, MKV, AVI)")) || [] : [await lib.pickFolder("A folder of videos")].filter(Boolean));
  if (paths.length) ui.set({ dialog: { type: "video-import", paths } });
}

export const addVideoLink = () => ui.set({ dialog: { type: "video-link" } });

// ------------------------------------------------------------ the page (D153)

const kindOrder = (k) => { const i = KINDS.findIndex(([x]) => x === k); return i < 0 ? 99 : i; };
const kindName = (k) => KINDS.find(([x]) => x === k)?.[1] || k;
const IS = { file: "A file", link: "A link", nogame: "About no game", gone: "Its file is gone" };
const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
/** A video's pictures: the one chosen for it, else its games' covers. */
const pics = (v, px) => [thumb(v.picture_path, px), ...v.games.map((g) => thumb(g.picture, px))];
const noPic = (v) => ({ icon: v.file ? "video" : "external", colour: systemColour(v.kind) });

const SORTS = [
  ["title", "Title", (a, b) => byName(a.title, b.title)],
  ["date", "Date", (a, b) => (a.date ? 0 : 1) - (b.date ? 0 : 1) || byName(a.date, b.date)],
  ["length", "Length", (a, b) => (b.length || 0) - (a.length || 0)],
  ["added", "Added", (a, b) => byName(b.added, a.added)],
];
const GROUPS = [["", "None"], ["kind", "Kind"], ["series", "Series"], ["creator", "Creator"], ["tag", "Tag"], ["system", "System"], ["year", "Year"]];
const COLUMNS = [
  ["title", "Title", (v) => v.title, "title"],
  ["kind", "Kind", (v) => v.kind_name, null],
  ["series", "Series", (v) => v.series || "", null],
  ["creator", "Creator", (v) => v.source || "", null],
  ["games", "Games", (v) => v.games.map((g) => g.title).join(", "), null],
  ["date", "Date", (v) => v.date || "", "date"],
  ["length", "Length", (v) => duration(v.length) || "", "length"],
  ["size", "Size", (v) => (v.file ? size(v.size) : "A link"), null],
];
const COLS = "minmax(160px, 3fr) 104px minmax(80px, 1.3fr) minmax(80px, 1.3fr) minmax(100px, 2fr) 92px 64px 72px";

/** What's changed of several videos at once (D153): their kind, series or creator, a tag, a game. */
async function editMany(ids, patch) {
  const r = await api("video_edit_many", { ids, patch });
  again();
  return r;
}

/** Remove videos: links are taken off the list at once (asked first); files are a change with the preview and Undo. */
async function removeVideos(list) {
  const ids = list.map((v) => v.id);
  const title = list.length === 1 ? `Remove ${list[0].title}?` : `Remove ${plural(list.length, "video", "videos")}?`;
  if (!list.some((v) => v.file)) return confirmDialog({ title, text: list.length === 1 ? "Its link is taken off the list." : "Their links are taken off the list.", button: "Remove them", run: async () => { await api("video_remove", { ids }); again(); } });
  try {
    const r = await api("video_remove", { ids });
    if (r.plan) ui.set({ dialog: { type: "organise", plan: r.plan } });
    else again();
  } catch (e) {
    toast(`Couldn't plan it: ${err(e)}`, 8000);
  }
}

/** One video in the panel: its picture, its details and tags and games, changed here. */
function VideoPanel({ v, off, tags, actions }) {
  const edit = async (patch) => {
    try {
      await api("video_edit", { id: v.id, patch });
      again();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const many = (patch) => editMany([v.id], patch).catch((e) => toast(err(e), 8000));
  return html`<div class="shelf-panel" id="video-edit" data-id=${v.id}>
    <${Pic} srcs=${pics(v, 512)} none=${noPic(v)} cls="lib-panel-cover" px=${64} />
    <h2 id="video-panel-title">${v.title}</h2>
    <p class="muted">${videoFacts(v)}</p>
    <${PanelActions} page="video" actions=${actions} />
    <${EditBox} id="video-edit-title" caption="Title" label="Title" value=${v.title} off=${off} onSave=${(x) => x && edit({ title: x })} />
    <label class="edit-row"><span class="edit-cap">Kind</span><${KindSelect} id="video-edit-kind" value=${v.kind} off=${off} onChange=${(k) => edit({ kind: k })} /></label>
    <${EditBox} id="video-edit-series" caption="Series" label="The series it's part of" placeholder="A show, a let's play" value=${v.series || ""} off=${off} onSave=${(x) => edit({ series: x })} />
    <${EditBox} id="video-edit-source" caption="Creator" label="Who made it, or where it's from" placeholder="A channel, a site" value=${v.source || ""} off=${off} onSave=${(x) => edit({ source: x })} />
    <${EditBox} id="video-edit-date" caption="Date" label="Date (YYYY-MM-DD)" placeholder="YYYY-MM-DD" value=${v.date || ""} off=${off} onSave=${(x) => edit({ date: x })} />
    ${!v.file ? html`<${EditBox} id="video-edit-url" caption="Address" label="Its address" value=${v.url || ""} off=${off} onSave=${(x) => x && edit({ url: x })} />` : null}
    <h3>Tags</h3>
    <${TagEdit} id="video-tags" tags=${v.tags} all=${tags} off=${off} onAdd=${(t) => many({ tag_add: t })} onRemove=${(t) => many({ tag_remove: t })} />
    <h3>Games</h3>
    ${v.games.length ? html`<ul class="ls-list shelf-games" id="video-games">${v.games.map((g) => html`<li key=${g.key} data-key=${g.key}><a href=${gameHash(g.key)}>${g.title}</a>${" "}<span class="muted">${g.platform_name}</span>
      <button type="button" class="ghost compact" data-action="drop-game" disabled=${off} title="It isn't about this game" onClick=${() => many({ game_remove: g.key })}>Remove</button></li>`)}</ul>` : html`<p class="muted">None yet: add the games it's about.</p>`}
    <button type="button" class="ghost compact" id="video-add-game" disabled=${off} onClick=${() => askGame({ title: "Add a game", button: "Add it", run: (g) => g && editMany([v.id], { game_add: g.key }) })}>${Icon.plus(14)} Add a game…</button>
  </div>`;
}

export function Videos() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("videos", { sort: "title" });
  const playing = useRouteArg("videos");
  // files dropped here are imported, as videos (D78)
  useDroppedFiles(() => "videos");
  useEffect(() => {
    if (!s.library) return;
    api("videos").then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path]);
  const videos = data?.videos || [];
  const names = useMemo(() => {
    const m = { system: new Map(), game: new Map() };
    for (const v of videos) for (const g of v.games) { m.system.set(g.platform, g.platform_name); m.game.set(g.key, `${g.title} (${g.platform_name})`); }
    return m;
  }, [data]);
  const words = useMemo(() => {
    const of = (f) => [...new Set(videos.flatMap(f).filter(Boolean))].sort(byName);
    return { tags: of((v) => v.tags || []), series: of((v) => [v.series]), creators: of((v) => [v.source]) };
  }, [data]);
  const tags = words.tags;
  const fields = [
    { id: "kind", name: "Kind", of: (v) => [v.kind], label: kindName, order: (a, b) => kindOrder(a.value) - kindOrder(b.value), groupOrder: (a, b) => kindOrder(a.value) - kindOrder(b.value) },
    { id: "series", name: "Series", of: (v) => [v.series], long: true, none: "In no series" },
    { id: "creator", name: "Creator", of: (v) => [v.source], long: true, none: "No creator given" },
    { id: "tag", name: "Tag", of: (v) => v.tags, long: true, none: "No tags" },
    { id: "system", name: "System", of: (v) => v.games.map((g) => g.platform), label: (p) => names.system.get(p) || p, none: "About no game" },
    { id: "game", name: "Game", of: (v) => v.games.map((g) => g.key), label: (k) => names.game.get(k) || k, long: true },
    { id: "year", name: "Year", of: (v) => [v.date?.slice(0, 4)], order: "newest", groupOrder: "newest", none: "No date" },
    { id: "is", name: "Is", of: (v) => [v.file ? "file" : "link", v.games.length ? null : "nogame", v.gone ? "gone" : null], label: (x) => IS[x] || x },
  ];
  const actions = (list) => {
    const one = list.length === 1 ? list[0] : null;
    const ids = list.map((v) => v.id);
    const theirTags = [...new Set(list.flatMap((v) => v.tags || []))].sort(byName);
    const common = (k) => (list.every((v) => (v[k] || "") === (list[0][k] || "")) ? list[0][k] || "" : "");
    const fail = (e) => toast(err(e), 8000);
    return [
      { id: "watch", label: "Watch", icon: one && !one.file ? "external" : "video", one: true, disabled: !!one?.gone, run: () => one && watch(one) },
      { id: "series", label: "Set the series…", icon: "stack", disabled: off, run: () => askValue({ title: "Set the series", label: "The series they're part of", value: common("series"), list: words.series, empty: true, button: "Set it", note: "Leave it empty to take them out of a series.", run: (x) => editMany(ids, { series: x }) }) },
      { id: "creator", label: "Set the creator…", icon: "edit", disabled: off, run: () => askValue({ title: "Set the creator", label: "Who made them, or where they're from", value: common("source"), list: words.creators, empty: true, button: "Set it", run: (x) => editMany(ids, { source: x }) }) },
      { id: "kind", label: "Set the kind…", icon: "layers", disabled: off, run: () => askValue({ title: "Set the kind", label: "Kind", value: common("kind") || "walkthrough", options: KINDS.map(([k, n]) => [k, n]), button: "Set it", run: (x) => editMany(ids, { kind: x }) }) },
      { id: "tag-add", label: "Add a tag…", icon: "pin", disabled: off, run: () => askValue({ title: "Add a tag", label: "The tag", list: tags, button: "Add it", note: "A tag is a label of your own, such as Favourites or To watch.", run: (x) => editMany(ids, { tag_add: x }) }) },
      { id: "tag-remove", label: "Remove a tag…", icon: "close", disabled: off || !theirTags.length, run: () => askValue({ title: "Remove a tag", label: "The tag", value: theirTags[0], options: theirTags.map((t) => [t, t]), button: "Remove it", run: (x) => editMany(ids, { tag_remove: x }) }) },
      { id: "game-add", label: "Add a game…", icon: "gamepad", disabled: off, run: () => askGame({ title: "Add a game", button: "Add it", note: list.length > 1 ? `It's added to each of the ${list.length} videos.` : null, run: (g) => g && editMany(ids, { game_add: g.key }).catch(fail) }) },
      { id: "picture", label: one?.picture ? "Choose another picture…" : "Choose a picture…", icon: "image", one: true, disabled: off || !isDesktop(), run: () => one && choosePicture(one) },
      ...(one?.picture ? [{ id: "picture-off", label: "Take its picture off", icon: "close", one: true, disabled: off, run: () => api("video_picture", { id: one.id, picture: null }).then(again, fail) }] : []),
      { id: "show", label: "Show in folder", icon: "folder", one: true, disabled: !isDesktop() || !one?.file || one?.gone, run: () => one && reveal(`${s.library.path}/${one.path}`) },
      { id: "remove", label: "Remove…", icon: "trash", disabled: off, run: () => removeVideos(list) },
    ];
  };
  const choosePicture = async (v) => {
    const p = await ctx.platform.library.pickFile(`A picture for ${v.title}`, ["jpg", "jpeg", "png", "webp"]);
    if (!p) return;
    try {
      await api("video_picture", { id: v.id, picture: p });
      again();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  // with videos to show, Import videos… is under ⋯, as the Library's Import… is
  const more = () => [
    ...(videos.length ? [{ id: "video-import-more", label: "Import videos…", icon: "inbox", disabled: off || !isDesktop(), run: () => importFrom("import-videos", () => importVideos()) }] : []),
    { id: "video-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importFrom("import-videos", () => importVideos(true)) },
    { id: "video-add-link", label: "Add a link…", icon: "external", disabled: off, run: addVideoLink },
  ];
  const primary = html`<button type="button" class="primary compact" id="video-import" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-videos", () => importVideos())}>${Icon.inbox(15)} Import videos…</button>`;
  if (!s.library) return html`<${CommandBar} id="videos-head" title="Videos" /><div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const now = videos.find((v) => v.id === playing && v.file);
  const top = now ? html`<section class="video-player" id="video-player" data-id=${now.id}>
    <div class="video-player-head"><h2>${now.title}</h2>
      <button type="button" class="ghost compact" id="video-player-close" onClick=${() => { location.hash = routeHash("videos"); }}>${Icon.close(14)} Close</button></div>
    ${now.gone ? html`<p class="warn-note">Its file isn't in the library any more: ${now.path}.</p>` : html`<${Video} key=${now.id} cls="video-big" rel=${now.path} src=${libraryUrl(now.path)} />`}
    <p class="muted">${videoFacts(now)}${now.url ? html`${" · "}<a href=${now.url} onClick=${(e) => { e.preventDefault(); ctx.platform.openUrl(now.url); }}>On the web</a>` : null}</p>
  </section>` : null;
  const empty = html`<div class="library-settings" id="video-empty">
    <p class="page-lead">Walkthroughs, reviews, retrospectives and other videos about your games: files kept in the library, or links to videos on the web.</p>
    <p class="muted">Import videos… reads a folder of them, takes each one's title, kind and date from its name, finds the games in its title, and imports them into the library's Videos folder. It's on the <a href=${routeHash("import")}>Import</a> page, where files dropped on the window go too. Add a link… keeps a video on the web, opened in your browser.</p>
    <div class="home-actions"><button type="button" class="ghost compact" id="video-add-link-empty" disabled=${off} onClick=${addVideoLink}>${Icon.external(14)} Add a link…</button></div>
  </div>`;
  return html`<${Shelf} page="videos" noun=${["video", "videos"]} title="Videos" items=${data ? videos : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} ratio="16 / 9" columns=${COLUMNS} cols=${COLS}
    text=${(v) => [v.title, v.source, v.series, ...(v.tags || []), ...v.games.map((g) => g.title)].join(" ")}
    tile=${(v) => ({ pics: pics(v, 512), none: noPic(v), badge: { text: v.kind_name, colour: systemColour(v.kind) }, title: v.title, meta: [v.series, v.source, duration(v.length)].filter(Boolean).join(" · ") || (v.file ? v.date : "A link"), missing: v.gone })}
    line=${(v) => ({ pics: pics(v, 256), none: noPic(v), badge: v.games.length ? { text: v.games.map((g) => g.title).slice(0, 2).join(", ") } : null, title: v.title, meta: [videoFacts(v), v.series].filter(Boolean).join(" · "), missing: v.gone })}
    actions=${actions} onOpen=${watch} top=${top} primary=${videos.length ? null : primary} more=${more} moreTitle="Import files, add a link, tile size" empty=${empty}
    panel=${(v) => html`<${VideoPanel} v=${v} off=${off} tags=${tags} actions=${actions([v]).filter((a) => ["watch", "show", "picture", "remove"].includes(a.id))} />`} />`;
}
