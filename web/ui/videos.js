// Videos (docs/PLAN.md, D146): walkthroughs, reviews, retrospectives and others, kept in the
// library's Videos/<Kind>/ folders or as web links, each with the games it's about. Import
// videos… reads a folder into a review list (each one's title, kind and date from its name,
// its games found in its title, put right here) and brings them in as a change with a
// preview and Undo; Add a link… keeps a web video, opened in the browser. The page filters
// them by kind, system and words, plays a file here (`#/videos/<id>`, video.js), and lets
// each one's details and games be changed; a game's page lists its videos.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, EditBox } from "./layout.js";
import { Dialog, More, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs } from "./organise.js";
import { importFrom } from "./importpage.js";
import { gameHash } from "./browse.js";
import { Video } from "./video.js";
import { useDroppedFiles } from "./documents.js";
import { Icon } from "./icons.js";
import { useRouteArg, argHash } from "./panels.js";

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

/** A video's details and games, changed by hand. */
function VideoEdit({ v, off, onChange }) {
  const [q, setQ] = useState("");
  const [found, setFound] = useState([]);
  useEffect(() => {
    if (q.trim().length < 2) { setFound([]); return; }
    let gone = false;
    const t = setTimeout(() => api("browse_query", { query: { search: q, all: true, one_per_title: true, limit: 8 } })
      .then((r) => { if (!gone) setFound(r.items || []); }).catch(() => { if (!gone) setFound([]); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [q]);
  const edit = async (patch) => {
    try {
      await api("video_edit", { id: v.id, patch });
      onChange();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const game = async (key, on) => {
    try {
      await api("video_game", { id: v.id, key, on });
      onChange();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  return html`<section class="ls-section" id="video-edit" data-id=${v.id}><h2>${v.title}</h2>
    <div class="pick-row">
      <${EditBox} id="video-edit-title" caption="Title" label="Title" value=${v.title} off=${off} onSave=${(x) => x && edit({ title: x })} />
      <label class="edit-row"><span class="edit-cap">Kind</span><${KindSelect} id="video-edit-kind" value=${v.kind} off=${off} onChange=${(k) => edit({ kind: k })} /></label>
      <${EditBox} id="video-edit-date" caption="Date" label="Date (YYYY-MM-DD)" placeholder="YYYY-MM-DD" value=${v.date || ""} off=${off} onSave=${(x) => edit({ date: x })} />
      <${EditBox} id="video-edit-source" caption="Source" label="Who made it, or where it's from" placeholder="A channel, a site" value=${v.source || ""} off=${off} onSave=${(x) => edit({ source: x })} />
      ${!v.file ? html`<${EditBox} id="video-edit-url" caption="Address" label="Its address" value=${v.url || ""} off=${off} onSave=${(x) => x && edit({ url: x })} />` : null}
    </div>
    <h3>Games</h3>
    ${v.games.length ? html`<ul class="ls-list" id="video-games">${v.games.map((g) => html`<li key=${g.key} data-key=${g.key}><a href=${gameHash(g.key)}>${g.title}</a>${" "}<span class="muted">${g.platform_name}</span>
      <button type="button" class="ghost compact" data-action="drop-game" disabled=${off} onClick=${() => game(g.key, false)}>Take off</button></li>`)}</ul>` : html`<p class="muted">None yet: add the games it's about.</p>`}
    <input type="search" class="compact" id="video-game-search" aria-label="A game to add: its title" placeholder="Add a game: its title" value=${q} disabled=${off} onInput=${(e) => setQ(e.target.value)} />
    ${found.length ? html`<ul class="ls-list" id="video-game-found">${found.map((x) => html`<li key=${x.key} data-key=${x.key}><span>${x.title}${" "}<span class="muted">${x.platform_name}</span></span>
      <button type="button" class="ghost compact" data-action="add-game" disabled=${off || v.games.some((g) => g.key === x.key)} onClick=${async () => { await game(x.key, true); setQ(""); }}>Add</button></li>`)}</ul>` : null}
  </section>`;
}

export function Videos() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [kind, setKind] = useState("all");
  const [system, setSystem] = useState("");
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(null);
  const playing = useRouteArg("videos");
  // files dropped here are imported, as videos (D78)
  useDroppedFiles(() => "videos");
  const reload = () => api("videos").then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => { if (s.library) reload(); }, [s.changes, s.library?.path]);
  const videos = data?.videos || [];
  // a link is taken off at once; a file's removal is a change, shown first (its preview asks)
  const remove = async (v) => {
    if (!v.file) return confirmDialog({ title: `Remove ${v.title}?`, text: "Its link is taken off the list.", button: "Remove it", run: async () => { await api("video_remove", { id: v.id }); again(); } });
    try {
      const r = await api("video_remove", { id: v.id });
      if (r.plan) ui.set({ dialog: { type: "organise", plan: r.plan } });
      else again();
    } catch (e) {
      toast(`Couldn't plan it: ${err(e)}`, 8000);
    }
  };
  const more = () => [
    { id: "video-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importFrom("import-videos", () => importVideos(true)) },
    { id: "video-add-link", label: "Add a link…", icon: "external", disabled: off, run: addVideoLink },
  ];
  const head = html`<${CommandBar} id="video-head" title="Videos" count=${data ? plural(videos.length, "video", "videos") : null}
    primary=${html`<button type="button" class="primary compact" id="video-import" disabled=${off || !isDesktop()} onClick=${() => importFrom("import-videos", () => importVideos())}>${Icon.inbox(15)} Import videos…</button>`}
    more=${{ id: "video-menu", title: "Import files, add a link", items: more }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${error}</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading…</p></div>`;
  const systems = [...new Map(videos.flatMap((v) => v.games.map((g) => [g.platform, g.platform_name]))).entries()].sort((a, b) => a[1].localeCompare(b[1]));
  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const shown = videos.filter((v) => (kind === "all" || v.kind === kind) && (!system || v.games.some((g) => g.platform === system))
    && words.every((w) => [v.title, v.source || "", ...v.games.map((g) => g.title)].join(" ").toLowerCase().includes(w)));
  const now = videos.find((v) => v.id === playing && v.file);
  const editing = videos.find((v) => v.id === open);
  return html`${head}<div class="pages videos-page" id="videos-page">
    ${!videos.length ? html`<div class="library-settings" id="video-empty">
      <p class="page-lead">Walkthroughs, reviews, retrospectives and other videos about your games: files kept in the library, or links to videos on the web.</p>
      <p class="muted">Import videos… reads a folder of them, takes each one's title, kind and date from its name, finds the games in its title, and imports them into the library's Videos folder. It's on the <a href=${routeHash("import")}>Import</a> page, where files dropped on the window go too. Add a link… keeps a video on the web, opened in your browser.</p>
      <button type="button" class="ghost compact" id="video-add-link-empty" disabled=${off} onClick=${addVideoLink}>${Icon.external(14)} Add a link…</button>
    </div>` : html`
      ${now ? html`<section class="ls-section video-player" id="video-player" data-id=${now.id}>
        <div class="video-player-head"><h2>${now.title}</h2>
          <button type="button" class="ghost compact" id="video-player-close" onClick=${() => { location.hash = routeHash("videos"); }}>${Icon.close(14)} Close</button></div>
        ${now.gone ? html`<p class="warn-note">Its file isn't in the library any more: ${now.path}.</p>` : html`<${Video} key=${now.id} cls="video-big" rel=${now.path} src=${libraryUrl(now.path)} />`}
        <p class="muted">${videoFacts(now)}${now.url ? html`${" · "}<a href=${now.url} onClick=${(e) => { e.preventDefault(); ctx.platform.openUrl(now.url); }}>On the web</a>` : null}</p>
      </section>` : null}
      <div class="pick-row video-filters" id="video-filters">
        <div class="seg audit-tabs" role="tablist" id="video-kinds">${[["all", "All", videos.length], ...KINDS.map(([k, , label]) => [k, label, videos.filter((v) => v.kind === k).length])].filter(([k, , n]) => k === "all" || n).map(([k, label, n]) => html`<button type="button" role="tab" key=${k} data-kind=${k} aria-selected=${kind === k ? "true" : "false"} onClick=${() => setKind(k)}>${label} <span class="tab-count">${n}</span></button>`)}</div>
        ${systems.length > 1 ? html`<select class="compact" id="video-system" aria-label="System" value=${system} onChange=${(e) => setSystem(e.target.value)}>
          <option value="">Every system</option>${systems.map(([id, name]) => html`<option key=${id} value=${id}>${name}</option>`)}</select>` : null}
        <input type="search" class="compact" id="video-search" aria-label="Search the videos" placeholder="Title, game or source" value=${q} onInput=${(e) => setQ(e.target.value)} />
      </div>
      ${shown.length ? html`<ul class="ls-list video-list" id="video-list">${shown.map((v) => html`<li key=${v.id} class=${playing === v.id ? "video-row playing" : "video-row"} data-id=${v.id} data-kind=${v.kind}>
        <button type="button" class="ghost video-watch" data-action="watch" title=${v.file ? "Watch it here" : "Watch it in your browser"} disabled=${v.file && v.gone} onClick=${() => watch(v)}>${v.file ? Icon.video(18) : Icon.external(18)}</button>
        <span class="video-main"><b class="video-title">${v.title}</b>
          <small class="muted">${videoFacts(v)}</small>
          ${v.games.length ? html`<span class="video-games">${v.games.map((g) => html`<a class="pc-badge video-chip" key=${g.key} href=${gameHash(g.key)} title=${`${g.title} (${g.platform_name})`}>${g.title}</a>`)}</span>` : null}</span>
        <span class="seg">
          <button type="button" class="ghost compact" data-action="edit" aria-pressed=${open === v.id ? "true" : "false"} title="Change its title, kind, date and games" onClick=${() => setOpen(open === v.id ? null : v.id)}>${Icon.edit(14)}</button>
          ${v.file && isDesktop() && !v.gone ? html`<button type="button" class="ghost compact" data-action="show" title="Show its file in its folder" onClick=${() => reveal(`${s.library.path}/${v.path}`)}>${Icon.folder(14)}</button>` : null}
          <button type="button" class="ghost compact" data-action="remove" disabled=${off} title="Remove it" onClick=${() => remove(v)}>${Icon.trash(14)}</button>
        </span>
      </li>`)}</ul>` : html`<p class="muted" id="video-none-shown">No video is ${kind !== "all" ? `a ${KINDS.find((k) => k[0] === kind)[1].toLowerCase()}` : "of these"}${system ? ` about a ${systems.find((x) => x[0] === system)?.[1]} game` : ""}${words.length ? " with those words" : ""}.</p>`}
      ${editing ? html`<${VideoEdit} key=${editing.id} v=${editing} off=${off} onChange=${reload} />` : null}`}
  </div>`;
}
