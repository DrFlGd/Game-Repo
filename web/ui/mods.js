// PC mods (docs/PLAN.md, "Cheats and patches", D205): the Patches page's third tab. A mod is
// kept with its PC game in the library (its metadata folder's patches/mods/), as it came
// (an archive, a folder, an installer), with how it goes in: into the game's folder, into a
// folder of the game's, a file to run (never run by Game Repo), or by hand. Mods have an
// order. Install the mods puts the ticked ones where the game is installed on this computer,
// what they replace set aside first, as a change History can undo; never into the library.
import { html, useState, useEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, isDesktop, libraryUrl, toast } from "./library.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { gameHash, systemColour } from "./browse.js";
import { Icon } from "./icons.js";
import { argHash } from "./panels.js";
import { Shelf, useShelfState, Pic, byName, askGame } from "./shelf.js";

const err = (e) => e.message || String(e);
const again = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
const thumb = (p, px = 512) => (p ? libraryUrl(`~thumb/${px}/${p}`) : null);
const size = (n) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round((n || 0) / 1e3))} KB`);
const HOWS = [["game", "Into the game's folder"], ["folder", "Into a folder of the game's"], ["run", "A file to run"], ["hand", "By hand"]];
const howName = (h) => HOWS.find((x) => x[0] === h)?.[1] || "By hand";
const installed = (p) => (p.installed?.to ? "Installed" : "Not installed");

/** Add a mod…: its archive, installer or file (or its folder, `folder`) for `key`'s PC game. */
export async function importMods({ key, folder = false, onDone = null }) {
  const lib = ctx.platform.library;
  const paths = folder ? [await lib.pickFolder("The mod's folder")].filter(Boolean) : (await lib.pickFiles("Mods: their archives (.zip, .7z), installers or files")) || [];
  if (!paths.length) return;
  ui.set({ dialog: { type: "mods-import", k: key, paths, onDone } });
}

function ImportDialog({ k, paths, onDone }) {
  const [how, setHow] = useState("copy");
  const [plan, setPlan] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const preview = async () => {
    setBusy(true);
    setError("");
    try {
      setPlan(await api("mods_import", { key: k, paths, how }));
    } catch (e) {
      setError(err(e));
    } finally {
      setBusy(false);
    }
  };
  if (plan) {
    return html`<${Dialog} title=${plan.title || plan.label} id="mods-import-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel="Bring it in"
      cancelLabel="Back" onCancel=${() => setPlan(null)} onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
      <${Preview} plan=${plan} />
      <p class="muted">The mod goes with the game in the library (<code>patches/mods/</code>), with a readme found beside it. How it goes in is guessed: check it on the game's mods. History can undo it.</p>
    <//>`;
  }
  return html`<${Dialog} title=${paths.length === 1 ? "Add a mod" : `Add ${paths.length} mods`} id="mods-import-dialog" onSubmit=${preview} busy=${busy} error=${error} submitLabel=${busy ? "Planning…" : "Preview"}>
    <ul class="ls-list" id="mods-import-paths">${paths.map((p) => html`<li key=${p}><code>${p}</code></li>`)}</ul>
    <fieldset class="field-block" id="mods-import-how"><legend class="field-label">How</legend>
      <label class="check-row"><input type="radio" name="mods-how" value="copy" checked=${how === "copy"} onChange=${() => setHow("copy")} /> Hardlinked on the same drive, else copied: the download stays where it is</label>
      <label class="check-row"><input type="radio" name="mods-how" value="move" checked=${how === "move"} onChange=${() => setHow("move")} /> Moved into the library</label>
    </fieldset>
  <//>`;
}

/** Install the mods: the preview of the ticked mods going where the game is installed. */
function InstallDialog({ plan, onDone }) {
  const skipped = plan.skipped || [];
  return html`<${Dialog} title=${plan.title || plan.label} id="mods-install-dialog" wide=${true} sticky=${true} busy=${plan.short || !plan.ops} submitLabel="Install them"
    onSubmit=${async () => { close(); await runPlan(plan); onDone?.(); again(); }}>
    <${Preview} plan=${plan} />
    <p class="muted" id="mods-install-to">Into <code>${plan.to}</code>, in their order: a later mod's file goes over an earlier one's. The game's files they replace are set aside, and History's Undo puts them back.</p>
    ${skipped.length ? html`<p class="warn-note" id="mods-install-skipped">Not by Game Repo: ${skipped.map((x) => `${x.name} (${howName(x.how).toLowerCase()})`).join(", ")}. Game Repo never runs a mod's installer: run it yourself, after these.</p>` : null}
  <//>`;
}

/** Edit details…: a mod's title, author, version, address, notes and the folder in its archive. */
function EditDialog({ k, file, details, onDone }) {
  const [d, setD] = useState(details);
  const [error, setError] = useState("");
  const f = (key, label, ph = "") => html`<label class="field-block"><span class="field-label">${label}</span>
    <input type="text" data-field=${key} value=${d[key] || ""} placeholder=${ph} onInput=${(e) => setD({ ...d, [key]: e.target.value })} /></label>`;
  const save = () => api("mods_edit", { key: k, name: file, details: d }).then((g) => { close(); onDone?.(g); again(); }).catch((e) => setError(err(e)));
  return html`<${Dialog} title=${`Edit ${file}`} id="mods-edit-dialog" onSubmit=${save} error=${error} submitLabel="Save">
    ${f("title", "Title")}${f("author", "Author")}${f("version", "Version")}${f("url", "Where it came from", "https://…")}
    ${f("inside", "The folder in its archive its files are in", "Leave empty for the archive's top")}
    <label class="field-block"><span class="field-label">Notes</span><textarea data-field="notes" rows="3" value=${d.notes || ""} onInput=${(e) => setD({ ...d, notes: e.target.value })}></textarea></label>
  <//>`;
}

export function ModDialogs({ dialog }) {
  if (dialog.type === "mods-import") return html`<${ImportDialog} ...${dialog} />`;
  if (dialog.type === "mods-install") return html`<${InstallDialog} ...${dialog} />`;
  if (dialog.type === "mods-edit") return html`<${EditDialog} ...${dialog} />`;
  return null;
}

/** A PC game's mods in order: each ticked or not, how it goes in, moved up or down. */
export function ModsEditor({ k, off, focus = null }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const load = () => api("mods_game", { key: k }).then((x) => { setG(x); setError(""); }).catch((e) => setError(err(e)));
  useEffect(() => { setG(null); load(); }, [k]);
  if (error) return html`<p class="warn-note">${error}</p>`;
  if (!g) return html`<p class="muted">Reading…</p>`;
  const list = g.mods || [];
  const set = (m, patch) => api("mods_edit", { key: k, name: m.name, details: { ...m.details, ...patch } }).then(setG).catch((e) => toast(err(e), 8000));
  const move = (i, by) => {
    const names = list.map((m) => m.name);
    const [x] = names.splice(i, 1);
    names.splice(i + by, 0, x);
    api("mods_order", { key: k, names }).then(setG).catch((e) => toast(err(e), 8000));
  };
  const where = async () => {
    const f = await ctx.platform.library.pickFolder(`Where ${g.title} is installed`);
    if (!f) return;
    try {
      await api("pc_update", { id: k.replace(/^pc\//, ""), patch: { installed: f } });
      load();
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  const install = async () => {
    try {
      ui.set({ dialog: { type: "mods-install", plan: await api("mods_install", { key: k }), onDone: load } });
    } catch (e) {
      toast(`Couldn't install them: ${err(e)}`, 10000);
    }
  };
  const ticked = list.filter((m) => m.details.on && (m.details.how === "game" || m.details.how === "folder")).length;
  return html`<div class="patches-editor" id="mods-editor" data-key=${k}>
    <div class="lib-panel-actions">
      <button type="button" class="ghost compact" id="mods-add" disabled=${off || !isDesktop()} onClick=${() => importMods({ key: k, onDone: load })}>${Icon.plus(14)} Add a mod…</button>
      <button type="button" class="ghost compact" id="mods-add-folder" disabled=${off || !isDesktop()} onClick=${() => importMods({ key: k, folder: true, onDone: load })}>Add a mod's folder…</button>
    </div>
    <p class="muted" id="mods-where" data-found=${g.installed_found ? "yes" : "no"}>${g.installed_at
      ? html`Installed on this computer in <code>${g.installed_at}</code>${g.installed_found ? "" : html` <span class="warn-note">(not there now)</span>`}.`
      : "Mods go where the game is installed, never into the library's copy: say where it's installed on this computer to install them here."}
      ${isDesktop() && !off ? html` <button type="button" class="ghost compact" id="mods-where-choose" onClick=${where}>${g.installed_at ? "Change…" : "Where it's installed…"}</button>` : null}</p>
    ${list.length ? html`<div class="lib-panel-actions"><button type="button" class="primary compact" id="mods-install" disabled=${off || !ticked || !g.installed_found || !isDesktop()} onClick=${install}>Install the ticked mods…</button></div>` : null}
    ${!list.length ? html`<p class="muted" id="mods-none">${g.title} has no mods yet. Add a mod… brings one in: its archive, its installer or its folder.</p>` : null}
    ${list.map((m, i) => html`<section class=${`patch-file${focus === m.name ? " on" : ""}`} key=${m.name} data-file=${m.name} data-how=${m.details.how}>
      <h3><label class="check-row"><input type="checkbox" data-action="on" aria-label=${`Install ${m.name}`} checked=${m.details.on} disabled=${off || m.details.how === "run" || m.details.how === "hand"} onChange=${(e) => set(m, { on: e.target.checked })} /> ${i + 1}. ${m.details.title || m.name}</label></h3>
      <p class="muted">${[m.name, m.archive ? "an archive" : m.folder ? "a folder" : "a file", size(m.size), m.details.version ? `v${m.details.version}` : null, m.details.author ? `by ${m.details.author}` : null].filter(Boolean).join(" · ")}</p>
      <span class="patch-found-row">
        <select class="compact" aria-label="How it goes in" data-action="how" value=${m.details.how} disabled=${off} onChange=${(e) => set(m, { how: e.target.value, on: e.target.value === "game" || (e.target.value === "folder" && !!m.details.folder) })}>${HOWS.map(([v, l]) => html`<option value=${v} key=${v}>${l}</option>`)}</select>
        ${m.details.how === "folder" ? html`<input type="text" class="compact" aria-label="The game's folder it goes into" data-action="folder" placeholder="Mods/" value=${m.details.folder} disabled=${off} onChange=${(e) => set(m, { folder: e.target.value, on: !!e.target.value })} />` : null}
      </span>
      ${m.details.how === "run" ? html`<p class="muted">Game Repo never runs it: run it yourself once the game is installed (and its other mods are in).</p>` : null}
      ${m.details.how === "hand" ? html`<p class="muted">Kept here: put it in as its readme says.</p>` : null}
      <p class="muted" data-installed=${m.installed?.to ? "yes" : "no"}>${m.installed?.to ? html`Installed in <code>${m.installed.to}</code>.` : "Not installed here yet."}</p>
      ${m.details.url ? html`<p class="muted"><a href=${m.details.url} target="_blank" rel="noopener">${m.details.url}</a></p>` : null}
      ${m.details.notes ? html`<p>${m.details.notes}</p>` : null}
      ${m.readme ? html`<details><summary>Its readme</summary><pre class="patch-readme">${m.readme}</pre></details>` : null}
      <div class="lib-panel-actions">
        <button type="button" class="ghost compact" data-action="up" aria-label="Earlier" title="Earlier" disabled=${off || i === 0} onClick=${() => move(i, -1)}>↑</button>
        <button type="button" class="ghost compact" data-action="down" aria-label="Later" title="Later" disabled=${off || i === list.length - 1} onClick=${() => move(i, 1)}>↓</button>
        <button type="button" class="ghost compact" data-action="edit" disabled=${off} onClick=${() => ui.set({ dialog: { type: "mods-edit", k, file: m.name, details: m.details, onDone: setG } })}>Edit details…</button>
        <button type="button" class="ghost compact" data-action="remove" disabled=${off} onClick=${() => confirmDialog({ title: `Remove ${m.name}?`, text: "It's kept aside in the game's patches/mods folder (.removed). What it put where the game is installed stays: History undoes that.", button: "Remove it", run: () => api("mods_remove", { key: k, name: m.name }).then((x) => { setG(x); again(); }) })}>${Icon.trash(14)} Remove</button>
      </div>
    </section>`)}
  </div>`;
}

const pics = (p, px) => [thumb(p.picture, px)];
const noPic = (p) => ({ text: "PC", colour: systemColour("pc") });
const SORTS = [
  ["game", "Game", (a, b) => byName(a.game_title, b.game_title) || a.order - b.order],
  ["title", "Mod", (a, b) => byName(a.title, b.title)],
  ["size", "Size", (a, b) => (b.size || 0) - (a.size || 0)],
];
const GROUPS = [["", "None"], ["game", "Game"], ["how", "How it goes in"]];
const COLUMNS = [
  ["title", "Mod", (p) => p.title, "title"],
  ["game", "Game", (p) => p.game_title, "game"],
  ["how", "How", (p) => p.how_name, null],
  ["size", "Size", (p) => size(p.size), "size"],
  ["installed", "Installed", installed, null],
];
const COLS = "minmax(140px, 2.4fr) minmax(140px, 2fr) 170px 90px 110px";

/** The PC mods tab: every PC game's mods, a game's opened above them (`opened`). */
export function PcMods({ switches, opened, off }) {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const [data, setData] = useState(null);
  const [error, setError] = useState("");
  const [st, set] = useShelfState("mods", { sort: "game", view: "list" });
  useEffect(() => {
    if (!s.library) return;
    setData(null);
    api("mods_list", {}).then((d) => { setData(d); setError(""); }).catch((e) => setError(err(e)));
  }, [s.changes, s.library?.path]);
  const items = data?.items || [];
  const games = useMemo(() => new Map(items.map((p) => [p.game_key, p.game_title])), [data]);
  const fields = [
    { id: "game", name: "Game", of: (p) => [p.game_key], label: (v) => games.get(v) || v },
    { id: "how", name: "How it goes in", of: (p) => [p.how_name] },
    { id: "installed", name: "Installed", of: (p) => [installed(p)] },
  ];
  const add = (folder) => askGame({
    title: "Add a mod", button: folder ? "Choose its folder…" : "Choose the mod…", note: "The PC game the mod is for.",
    run: async (g) => {
      if (!g) return;
      if (!g.key.startsWith("pc/")) throw new Error(`${g.title} isn't a PC game: mods are for PC games (a ROM's hacks are ROM patches).`);
      close();
      await importMods({ key: g.key, folder });
    },
  });
  const actions = (list) => {
    const one = list.length === 1 ? list[0] : null;
    return [
      { id: "open", label: "Its game's mods", icon: "check", one: true, run: () => one && (location.hash = argHash("patches", one.game_key)) },
      { id: "game", label: "Its game", icon: "gamepad", one: true, run: () => one && (location.hash = gameHash(one.game_key)) },
    ];
  };
  const more = () => [
    { id: "mods-add-folder-page", label: "Add a mod's folder…", icon: "inbox", disabled: off || !isDesktop(), run: () => add(true) },
  ];
  const primary = html`<button type="button" class="primary compact" id="mods-import" disabled=${off || !isDesktop()} onClick=${() => add(false)}>${Icon.inbox(15)} Add a mod…</button>`;
  const top = opened && opened.startsWith("pc/") ? html`<section class="cheats-top" id="patches-top" data-key=${opened}>
    <div class="video-player-head"><h2><a href=${gameHash(opened)}>${items.find((p) => p.game_key === opened)?.game_title || "Its mods"}</a></h2>
      <button type="button" class="ghost compact" id="patches-top-close" onClick=${() => { location.hash = routeHash("patches"); }}>${Icon.close(14)} Close</button></div>
    <${ModsEditor} k=${opened} off=${off} />
  </section>` : null;
  const empty = html`<div class="library-settings" id="mods-empty">
    <p class="page-lead">Mods for your PC games, kept with each game in the library, in an order, and installed where the game is.</p>
    <p class="muted">Add a mod… brings in a mod (its archive, its installer or its folder) for a PC game, with its readme. Each says how it goes in: into the game's folder, into a folder of the game's (such as Mods/), a file to run, or by hand. Install the ticked mods… puts them where the game is installed on this computer, never into the library's copy, and History can undo it.</p>
    <div class="home-actions">${primary}</div>
    <div class="lib-switches">${switches}</div>
  </div>`;
  return html`<${Shelf} page="mods" noun=${["mod", "mods"]} title="Patches" items=${data ? items : null} error=${error} st=${st} set=${set}
    fields=${fields} groups=${GROUPS} sorts=${SORTS} columns=${COLUMNS} cols=${COLS} ratio="1"
    text=${(p) => [p.title, p.file, p.game_title, p.author].join(" ")}
    tile=${(p) => ({ pics: pics(p, 512), none: noPic(p), badge: { text: p.how_name, colour: systemColour("pc") }, title: p.title, meta: [p.game_title, installed(p)].join(" · ") })}
    line=${(p) => ({ pics: pics(p, 256), none: noPic(p), badge: { text: `${p.order}`, colour: systemColour("pc") }, title: p.title, meta: [p.game_title, p.how_name, size(p.size), installed(p)].join(" · ") })}
    actions=${actions} onOpen=${(p) => { location.hash = argHash("patches", p.game_key); }} switches=${switches} top=${top}
    primary=${items.length ? null : primary} more=${more} moreTitle="Add a mod's folder, tile size" empty=${top ? null : empty}
    panel=${(p) => html`<div class="shelf-panel" id="mods-panel" data-key=${p.game_key} data-file=${p.file}>
      <${Pic} srcs=${pics(p, 512)} none=${noPic(p)} cls="lib-panel-cover" px=${64} />
      <h2 id="mods-panel-title"><a href=${gameHash(p.game_key)}>${p.game_title}</a></h2>
      <${ModsEditor} k=${p.game_key} off=${off} focus=${p.file} />
    </div>`} />`;
}
