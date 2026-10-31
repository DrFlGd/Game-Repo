// Cheats (docs/PLAN.md, "Cheats and patches", D197–D199): each game's cheat files, kept in
// its folder's cheats/ with cheats.json for their formats and sources. Get cheats… fetches
// RetroArch's own cheat pack (libretro-database, CC BY-SA 4.0), matches its files to the
// games you have by name (a close name waits for a yes), and brings them in as a change with
// Undo; a file edited here is never replaced, libretro's newer copy is offered beside it.
// Import cheat files… does the same for files of your own. The page is laid out as the
// Library is (shelf.js): a game per item, filtered by system, format, source; the one picked
// is edited in the panel: a RetroArch .cht as a list of cheats (name, code, on at start),
// the emulators' own formats as text, each save keeping the version before (Versions…).
// `#/cheats/<key>` opens a game's cheats above the list (a game's page links there).
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, followJob, isDesktop, plural, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar } from "./layout.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { gameHash, systemColour } from "./browse.js";
import { Icon } from "./icons.js";
import { useRouteArg, argHash } from "./panels.js";
import { Shelf, useShelfState, Pic, askValue, byName } from "./shelf.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
const SHOW = [["with", "Games with cheats"], ["without", "Games without cheats"], ["edited", "Edited by you"]];
const PAIRING = { sure: ["Sure", "The file is named after this very game"], likely: ["Likely", "The same title, another release: check it"], you: ["You chose", "Added to the game you chose"], none: ["No game", "No game you have is named like it"] };

/** The cheats a game has, said shortly. */
const cheatsLine = (g) => (g.count ? plural(g.count, "cheat", "cheats") : g.files?.length ? plural(g.files.length, "file", "files") : "No cheats yet");

/** Get cheats… for `platforms` (every system with games when empty) or the games `keys`. */
export async function getCheats({ platforms = [], keys = [], again: fresh = false } = {}) {
  try {
    const { job } = await api("cheats_get", { platforms, keys, again: fresh });
    const done = await followJob(job, "Getting cheats");
    if (done.error) throw new Error(done.error);
    ui.set({ dialog: { type: "cheats-found", result: done.result } });
  } catch (e) {
    toast(`Couldn't get the cheats: ${err(e)}`, 10000);
  }
}

/** Import cheat files… (a folder, or `files`), paired with games by name, or all for `key`'s game. */
export async function importCheats({ files = false, key = null } = {}) {
  const lib = ctx.platform.library;
  const paths = files || key ? (await lib.pickFiles("Cheat files (.cht, .pnach, .ini, .txt, .mch)")) || [] : [await lib.pickFolder("A folder of cheat files")].filter(Boolean);
  if (!paths.length) return;
  try {
    const { job } = await api("cheats_files", { paths, key });
    const done = await followJob(job, "Reading the cheat files");
    if (done.error) throw new Error(done.error);
    ui.set({ dialog: { type: "cheats-found", result: { found: done.result.found, files: true } } });
  } catch (e) {
    toast(`Couldn't read them: ${err(e)}`, 10000);
  }
}

/** What Get cheats… or an import found: the sure ones ticked, the likely ones to tick, then the preview. */
function FoundDialog({ result }) {
  const [found, setFound] = useState(() => result.found || []);
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const set = (i, patch) => setFound((all) => all.map((f, j) => (j === i ? { ...f, ...patch } : f)));
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("cheats_plan", { found }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="cheats-found-dialog" wide=${true} sticky=${true} onSubmit=${() => { close(); runPlan(plan).then(again); }} busy=${plan.short || !plan.ops} submitLabel="Bring them in"
      cancelLabel="Back" onCancel=${() => setPlan(null)}>
      <${Preview} plan=${plan} />
      <p class="muted">Each game's cheats go into its folder in the library (<code>cheats/</code>). History can undo it.</p>
    <//>`;
  }
  const n = found.filter((f) => f.include).length;
  const likely = found.filter((f) => f.pairing === "likely").length;
  const said = result.files ? null : [
    result.games != null ? `${plural(result.games, "game", "games")} looked for` : null,
    result.same ? `${plural(result.same, "game has", "games have")} libretro's cheats already` : null,
    result.kept ? `${plural(result.kept, "file", "files")} you've edited or made kept as they are` : null,
    result.none ? `${plural(result.none, "game", "games")} with none in libretro's` : null,
  ].filter(Boolean).join(" · ");
  return html`<${Dialog} title=${result.files ? "Import cheat files" : "Cheats found"} id="cheats-found-dialog" wide=${true} sticky=${found.length > 0} onSubmit=${preview} busy=${busy || !n} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    ${said ? html`<p class="muted" id="cheats-found-said">${said}.</p>` : null}
    ${!found.length ? html`<p class="muted" id="cheats-found-none">${result.files ? "No cheat files there." : "Nothing new: every game that libretro has cheats for has them already."}</p>` : html`
      ${likely ? html`<p class="warn-note" id="cheats-found-likely">${plural(likely, "game was", "games were")} matched by a close name, not its own: tick the ones that are right.</p>` : null}
      <ul class="ls-list pc-items" id="cheats-found">${found.map((f, i) => html`<li key=${`${f.key}|${f.name}|${i}`} class="pc-item" data-key=${f.key} data-pairing=${f.pairing} data-include=${f.include ? "yes" : "no"}>
        <label class="check-row pc-item-include"><input type="checkbox" data-action="include" aria-label=${`Bring in ${f.name}`} checked=${f.include} disabled=${!f.key} onChange=${(e) => set(i, { include: e.target.checked })} /></label>
        <div class="pc-item-main">
          <span class="pc-item-name">${f.title || html`<span class="muted">No game</span>`} <span class="muted">${f.platform}</span>
            <span class="pc-badge" title=${PAIRING[f.pairing]?.[1]}>${PAIRING[f.pairing]?.[0] || f.pairing}</span>${f.state === "newer" ? html` <span class="pc-badge">A newer copy</span>` : null}</span>
          <span class="muted">${f.name} · ${plural(f.count, "cheat", "cheats")}</span>
        </div></li>`)}</ul>
      <p class="muted"><b>${plural(n, "file", "files")}</b> to bring in.${result.credit ? ` ${result.credit}` : ""}</p>`}
  <//>`;
}

/** A file's earlier versions, each with *Go back to this one*. */
function VersionsDialog({ k, name, onDone }) {
  const [list, setList] = useState(null);
  const [error, setError] = useState("");
  useEffect(() => { api("cheats_versions", { key: k, name }).then((v) => setList(v.versions)).catch((e) => setError(err(e))); }, []);
  const back = async (when) => {
    try {
      const g = await api("cheats_restore", { key: k, name, when });
      close();
      onDone?.(g);
      again();
    } catch (e) {
      setError(err(e));
    }
  };
  return html`<${Dialog} title=${`Versions of ${name}`} id="cheats-versions" error=${error} submitLabel="Close" onSubmit=${close} cancelLabel=${null}>
    ${!list ? html`<p class="muted">Reading…</p>` : !list.length ? html`<p class="muted">No earlier versions: one is kept each time it's saved.</p>` : html`
      <ul class="ls-list" id="cheats-versions-list">${list.map((v) => html`<li key=${v.when} data-when=${v.when}><span>${v.when} <span class="muted">${plural(v.count, "cheat", "cheats")}</span></span>
        <button type="button" class="ghost compact" data-action="restore" onClick=${() => back(v.when)}>Go back to this one</button></li>`)}</ul>`}
  <//>`;
}

export function CheatDialogs({ dialog }) {
  if (dialog.type === "cheats-found") return html`<${FoundDialog} ...${dialog} />`;
  if (dialog.type === "cheats-versions") return html`<${VersionsDialog} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ the editor

/** A .cht's cheats as a list: each one's name, code and whether it's on at start. */
function ChtEditor({ file, off, onSave }) {
  const [cht, setCht] = useState(file.cht);
  const [dirty, setDirty] = useState(false);
  useEffect(() => { setCht(file.cht); setDirty(false); }, [file.text]);
  const change = (f) => { setCht((c) => ({ ...c, cheats: f(c.cheats) })); setDirty(true); };
  const edit = (i, patch) => change((l) => l.map((c, j) => (j === i ? { ...c, ...patch } : c)));
  const move = (i, by) => change((l) => { const n = [...l]; const [c] = n.splice(i, 1); n.splice(Math.max(0, Math.min(n.length, i + by)), 0, c); return n; });
  return html`<div class="cheat-list" data-file=${file.name}>
    ${cht.cheats.length ? html`<ol class="ls-list cheat-rows">${cht.cheats.map((c, i) => html`<li key=${i} class="cheat-row" data-index=${i}>
      <div class="cheat-fields">
        <input type="text" class="compact cheat-desc" aria-label="Name" placeholder="What it does" value=${c.desc} disabled=${off} onInput=${(e) => edit(i, { desc: e.target.value })} />
        <textarea class="cheat-code" aria-label="Code" rows=${Math.min(4, Math.max(1, String(c.code).split(/[+\n]/).length))} placeholder="The code" disabled=${off} onInput=${(e) => edit(i, { code: e.target.value })}>${c.code}</textarea>
        <label class="check-row"><input type="checkbox" class="cheat-enable" checked=${c.enable} disabled=${off} onChange=${(e) => edit(i, { enable: e.target.checked })} /> On at start</label>
      </div>
      <span class="cheat-moves">
        <button type="button" class="ghost compact" aria-label="Up" title="Up" disabled=${off || i === 0} onClick=${() => move(i, -1)}>↑</button>
        <button type="button" class="ghost compact" aria-label="Down" title="Down" disabled=${off || i === cht.cheats.length - 1} onClick=${() => move(i, 1)}>↓</button>
        <button type="button" class="ghost compact" data-action="drop-cheat" aria-label=${`Remove ${c.desc || "this cheat"}`} title="Remove" disabled=${off} onClick=${() => change((l) => l.filter((_, j) => j !== i))}>${Icon.close(12)}</button>
      </span></li>`)}</ol>` : html`<p class="muted">No cheats in it yet.</p>`}
    <div class="lib-panel-actions">
      <button type="button" class="ghost compact" data-action="add-cheat" disabled=${off} onClick=${() => change((l) => [...l, { desc: "", code: "", enable: false, extra: {} }])}>${Icon.plus(14)} Add a cheat</button>
      <button type="button" class="primary compact" data-action="save" disabled=${off || !dirty} onClick=${() => onSave({ cht })}>Save</button>
    </div>
  </div>`;
}

/** Any file as its text. */
function TextEditor({ file, off, onSave }) {
  const [text, setText] = useState(file.text);
  useEffect(() => setText(file.text), [file.text]);
  return html`<div class="cheat-text" data-file=${file.name}>
    <textarea class="cheat-text-box" aria-label=${`${file.name} as text`} rows="14" spellcheck="false" disabled=${off} onInput=${(e) => setText(e.target.value)}>${text}</textarea>
    <div class="lib-panel-actions"><button type="button" class="primary compact" data-action="save" disabled=${off || text === file.text} onClick=${() => onSave({ text })}>Save</button></div>
  </div>`;
}

/** One game's cheat files, edited. */
export function CheatsEditor({ k, off }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const [asText, setAsText] = useState({});
  const [open, setOpen] = useState(null);
  const load = () => api("cheats_game", { key: k }).then((x) => { setG(x); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => { setG(null); load(); }, [k]);
  const fail = (e) => toast(err(e), 8000);
  const done = (x) => { setG(x); again(); };
  const save = (f, body) => api("cheats_save", { key: k, name: f.name, ...body }).then((x) => { done(x); toast(`Saved ${f.name}.`); }).catch(fail);
  const make = () => askValue({
    title: "New cheat file", label: "Its format", value: g.formats[0]?.id, options: g.formats.map((f) => [f.id, f.name]), button: "Make it",
    note: "RetroArch's .cht is the one most emulators' cores read. The others are the emulators' own files, kept as text.",
    run: (format) => api("cheats_new", { key: k, format }).then((x) => { done(x); setOpen(x.made); }),
  });
  if (error) return html`<p class="warn-note">${error}</p>`;
  if (!g) return html`<p class="muted">Reading…</p>`;
  const files = g.files || [];
  const shown = open && files.some((f) => f.name === open) ? open : files[0]?.name;
  return html`<div class="cheats-editor" id="cheats-editor" data-key=${k}>
    <div class="lib-panel-actions">
      <button type="button" class="ghost compact" id="cheats-new" disabled=${off} onClick=${make}>${Icon.plus(14)} New cheat file…</button>
      ${g.kind === "dat" ? html`<button type="button" class="ghost compact" id="cheats-get-one" disabled=${off} onClick=${() => getCheats({ keys: [k] }).then(load)}>${Icon.download(14)} Get cheats…</button>` : null}
      <button type="button" class="ghost compact" id="cheats-add-files" disabled=${off || !isDesktop()} onClick=${() => importCheats({ key: k }).then(load)}>${Icon.file(14)} Add files…</button>
    </div>
    ${g.removed?.length ? html`<p class="muted" id="cheats-removed">Taken away: ${g.removed.map((n) => html`<button type="button" class="linkish" key=${n} onClick=${() => ui.set({ dialog: { type: "cheats-versions", k, name: n, onDone: done } })}>${n}</button>`)} (Versions… brings one back)</p>` : null}
    ${!files.length ? html`<p class="muted" id="cheats-none">${g.title} has no cheats yet.${g.kind === "dat" ? " Get cheats… looks for them in libretro's, or make a file of your own." : " Make a file of notes: codes, console commands."}</p>` : null}
    ${files.length > 1 ? html`<div class="game-strip" role="tablist" aria-label="Cheat files">${files.map((f) => html`<button type="button" role="tab" key=${f.name} data-file=${f.name} aria-selected=${f.name === shown ? "true" : "false"} onClick=${() => setOpen(f.name)}>${f.name}</button>`)}</div>` : null}
    ${files.filter((f) => f.name === shown).map((f) => html`<section class="cheat-file" key=${f.name} data-file=${f.name} data-format=${f.format}>
      <h3>${f.name}</h3>
      <p class="muted">${[f.format_name, plural(f.names.length, "cheat", "cheats"), f.source_name === "You" ? "made by you" : `from ${f.source_name}`, f.edited ? "edited by you" : null].filter(Boolean).join(" · ")}</p>
      ${f.newer ? html`<p class="warn-note" id="cheats-newer">libretro has a newer copy of this file. Yours is kept;
        <button type="button" class="linkish" disabled=${off} onClick=${() => confirmDialog({ title: "Use libretro's copy?", text: "Yours is kept in its versions, to go back to.", button: "Use libretro's", run: () => api("cheats_newer", { key: k, name: f.name }).then(done) })}>use libretro's instead</button>.</p>` : null}
      ${f.cht && !asText[f.name] ? html`<${ChtEditor} file=${f} off=${off} onSave=${(b) => save(f, b)} />` : html`<${TextEditor} file=${f} off=${off} onSave=${(b) => save(f, b)} />`}
      <div class="lib-panel-actions">
        ${f.cht ? html`<button type="button" class="ghost compact" data-action="as-text" onClick=${() => setAsText((a) => ({ ...a, [f.name]: !a[f.name] }))}>${asText[f.name] ? "As a list" : "As text"}</button>` : null}
        <button type="button" class="ghost compact" data-action="versions" disabled=${!f.versions} title=${f.versions ? "" : "One is kept each time it's saved"} onClick=${() => ui.set({ dialog: { type: "cheats-versions", k, name: f.name, onDone: done } })}>Versions…${f.versions ? ` (${f.versions})` : ""}</button>
        ${isDesktop() ? html`<button type="button" class="ghost compact" data-action="show" onClick=${() => reveal(f.full)}>Show the file</button>` : null}
        <button type="button" class="ghost compact" data-action="remove" disabled=${off} onClick=${() => confirmDialog({ title: `Remove ${f.name}?`, text: "It's kept with its versions, so Versions… can bring it back.", button: "Remove it", run: () => api("cheats_remove", { key: k, name: f.name }).then(done) })}>${Icon.trash(14)} Remove</button>
      </div>
    </section>`)}
  </div>`;
}

// ------------------------------------------------------------ the page

const pics = (g, px) => [thumb(g.picture, px)];
const noPic = (g) => ({ text: g.platform_name || g.platform, colour: systemColour(g.platform) });
const SORTS = [
  ["title", "Title", (a, b) => byName(a.title, b.title)],
  ["count", "Cheats", (a, b) => (b.count || 0) - (a.count || 0)],
  ["system", "System", (a, b) => byName(a.platform_name, b.platform_name)],
];
const GROUPS = [["", "None"], ["system", "System"], ["format", "Format"], ["source", "Source"]];
const COLUMNS = [
  ["title", "Game", (g) => g.title, "title"],
  ["system", "System", (g) => g.platform_name, "system"],
  ["count", "Cheats", (g) => String(g.count || 0), "count"],
  ["formats", "Formats", (g) => (g.formats || []).join(", "), null],
  ["source", "From", (g) => (g.sources || []).join(", "), null],
];
const COLS = "minmax(160px, 3fr) minmax(100px, 1.4fr) 72px minmax(80px, 1.3fr) minmax(80px, 1fr)";

export function Cheats() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("cheats", { sort: "title", view: "list", show: "with" });
  const opened = useRouteArg("cheats");
  const show = st.show || "with";
  useEffect(() => {
    if (!s.library) return;
    setData(null);
    api("cheats_list", { show }).then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path, show]);
  const items = data?.items || [];
  const systems = useMemo(() => new Map(items.map((g) => [g.platform, g.platform_name])), [data]);
  const fields = [
    { id: "system", name: "System", of: (g) => [g.platform], label: (p) => systems.get(p) || p },
    { id: "format", name: "Format", of: (g) => g.formats || [], none: "No cheats" },
    { id: "source", name: "Source", of: (g) => g.sources || [], none: "No cheats" },
  ];
  const actions = (list) => {
    const one = list.length === 1 ? list[0] : null;
    const keys = list.map((g) => g.key);
    return [
      { id: "open", label: "Edit its cheats", icon: "edit", one: true, run: () => one && (location.hash = argHash("cheats", one.key)) },
      { id: "get", label: "Get cheats…", icon: "download", disabled: off, run: () => getCheats({ keys }) },
      { id: "game", label: "Its page", icon: "gamepad", one: true, run: () => one && (location.hash = gameHash(one.key)) },
    ];
  };
  const groupMenu = ({ value, name }) => (st.group === "system" ? [{ id: "get-system", label: `Get cheats for ${name}…`, icon: "download", disabled: off, run: () => getCheats({ platforms: [value] }) }] : null);
  const more = () => [
    { id: "cheats-get-again", label: "Get the newest from libretro…", icon: "download", disabled: off, run: () => getCheats({ again: true }) },
    { id: "cheats-import", label: "Import cheat files…", icon: "inbox", disabled: off || !isDesktop(), run: () => importCheats() },
    { id: "cheats-import-files", label: "Import files…", icon: "file", disabled: off || !isDesktop(), run: () => importCheats({ files: true }) },
  ];
  const primary = html`<button type="button" class="primary compact" id="cheats-get" disabled=${off} onClick=${() => getCheats()}>${Icon.download(15)} Get cheats…</button>`;
  if (!s.library) return html`<${CommandBar} id="cheats-head" title="Cheats" /><div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const switches = html`<span class="seg" role="group" aria-label="Show" id="cheats-show">${SHOW.map(([v, label]) => html`<button type="button" key=${v} class=${`ghost compact${show === v ? " on" : ""}`} data-show=${v} aria-pressed=${show === v ? "true" : "false"} onClick=${() => set({ show: v })}>${label}</button>`)}</span>`;
  const top = opened ? html`<section class="cheats-top" id="cheats-top" data-key=${opened}>
    <div class="video-player-head"><h2><a href=${gameHash(opened)}>${items.find((g) => g.key === opened)?.title || opened.split("/").slice(1).join("/")}</a></h2>
      <button type="button" class="ghost compact" id="cheats-top-close" onClick=${() => { location.hash = routeHash("cheats"); }}>${Icon.close(14)} Close</button></div>
    <${CheatsEditor} k=${opened} off=${off} />
  </section>` : null;
  const empty = html`<div class="library-settings" id="cheats-empty">
    <p class="page-lead">${show === "with" ? "Cheats for your games, kept with each game in the library." : show === "edited" ? "No cheat files edited here yet." : "Every game you have has cheats."}</p>
    ${show === "with" ? html`<p class="muted">Get cheats… looks in RetroArch's own cheat database for the games you have and brings in each one's file; Import cheat files… brings in your own (.cht, .pnach, .ini, .txt). Games without cheats lists the rest, to make files for.</p>
    <div class="home-actions">${primary}<button type="button" class="ghost compact" id="cheats-import-empty" disabled=${off || !isDesktop()} onClick=${() => importCheats()}>${Icon.inbox(14)} Import cheat files…</button></div>
    <p class="muted">${data?.credit || ""}</p>` : null}
    <div class="lib-switches">${switches}</div>
  </div>`;
  return html`<${Shelf} page="cheats" noun=${["game", "games"]} title="Cheats" items=${data ? items : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} columns=${COLUMNS} cols=${COLS} ratio="1"
    text=${(g) => [g.title, g.name, g.platform_name, g.names].join(" ")}
    tile=${(g) => ({ pics: pics(g, 512), none: noPic(g), badge: { text: g.platform_name, colour: systemColour(g.platform) }, title: g.title, meta: cheatsLine(g) })}
    line=${(g) => ({ pics: pics(g, 256), none: noPic(g), badge: { text: g.platform_name, colour: systemColour(g.platform) }, title: g.title, meta: [cheatsLine(g), (g.formats || []).join(", "), g.edited ? "edited by you" : null].filter(Boolean).join(" · ") })}
    actions=${actions} onOpen=${(g) => { location.hash = argHash("cheats", g.key); }} groupMenu=${groupMenu} switches=${switches} top=${top}
    primary=${items.length ? null : primary} more=${more} moreTitle="Import cheat files, the newest from libretro, tile size" empty=${top ? null : empty}
    panel=${(g) => html`<div class="shelf-panel" id="cheats-panel" data-key=${g.key}>
      <${Pic} srcs=${pics(g, 512)} none=${noPic(g)} cls="lib-panel-cover" px=${64} />
      <h2 id="cheats-panel-title"><a href=${gameHash(g.key)}>${g.title}</a></h2>
      <p class="muted">${g.platform_name} · ${cheatsLine(g)}</p>
      <${CheatsEditor} k=${g.key} off=${off} />
    </div>`} />`;
}
