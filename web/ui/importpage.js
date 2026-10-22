// Import (docs/PLAN.md, "UI pass 3 design", D71 and D73; D78): everything that brings files
// into the library, in one place. Games in folders you keep (the folders with games, scanned
// where they are), games copied into the library from a folder (matched by the audit DATs,
// named as they say, D53's bring-in) or one at a time (Add a game…), PC games, DATs, BIOS
// files, documents, magazines and videos, and the imports made lately. The other pages' import
// buttons open this page at their section (`importFrom`), and files dropped where no page
// takes them come here to be asked what they are (`dropToImport`).
import { html, useState, useEffect } from "../lib/html.js";
import { createStore, useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, followJob, isDesktop, plural, toast } from "./library.js";
import { CommandBar } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { Preview, runPlan, HowControl, howArgs, gatherDialog, IMPORT_KINDS } from "./organise.js";
import { RootsSection, DatsSection, ChangeHistory } from "./settings.js";
import { importPc, addGameDialog } from "./pcgames.js";
import { importBios } from "./bios.js";
import { attachDocs } from "./documents.js";
import { importMagazines } from "./magazines.js";
import { importVideos, addVideoLink } from "./videos.js";
import { importDats, starterDats, addRoot, refreshGames, scan } from "./games.js";
import { Icon } from "./icons.js";
import { argHash, useRouteArg } from "./panels.js";
import { reviewUnknown } from "./variants.js";

/** An import started from another page (D78): the Import page opens at its section
 *  (`card`, the section's id) and `run` starts it there. */
export function importFrom(card, run) {
  const hash = argHash("import", card);
  if (location.hash !== hash) location.hash = hash;
  return run?.();
}

const err = (e) => e.message || String(e);
const baseName = (p) => String(p).replace(/[\\/]+$/, "").split(/[\\/]/).pop();

const dirOf = (p) => { const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")); return i > 0 ? p.slice(0, i) : ""; };

/** Where a system's games go: its folders as `addgame::places` ranks them, or another. */
function PlaceRow({ platform, name, count, places, value, onChange }) {
  const pick = async () => {
    const p = await ctx.platform.library.pickFolder(`The folder ${name}'s games go to`);
    if (p) onChange(p);
  };
  const listed = places || [];
  return html`<li class="import-place" data-platform=${platform} data-dir=${value || ""}>
    <span class="import-place-name"><b>${name}</b> <span class="muted">${plural(count, "game", "games")}</span></span>
    <span class="pick-row">
      <select class="compact" aria-label=${`Where ${name}'s games go`} value=${value || ""} onChange=${(e) => (e.target.value === "\u0000" ? pick() : onChange(e.target.value))}>
        ${value ? null : html`<option value="">Choose a folder…</option>`}
        ${listed.map((p) => html`<option key=${p.dir} value=${p.dir} title=${p.why}>${p.dir}</option>`)}
        ${value && !listed.some((p) => p.dir === value) ? html`<option value=${value}>${value}</option>` : null}
        ${isDesktop() ? html`<option value="\u0000">Another folder…</option>` : null}
      </select>
    </span>
    ${value ? html`<small class="muted">${listed.find((p) => p.dir === value)?.why || "A new folder with games for it"}</small>` : html`<small class="warn">There's no folder with games for ${name} yet: choose where they go.</small>`}
  </li>`;
}

/* Where an import is up to, kept outside the window: closing it (or running the work in
   the background) loses nothing, and From a folder… or the status bar brings it back. */
const blank = () => ({ folders: [], typed: "", only: "", mode: "copy", review: null, off: [], where: {}, lists: {}, given: {}, work: null, plan: null, error: "", into: "", intoName: "" });
const imp = createStore(blank());
// another library opened: what was picked for this one is dropped (review M37)
let impLibrary = ui.get().library?.path;
ui.subscribe((st) => {
  const now = st.library?.path;
  if (now === impLibrary) return;
  impLibrary = now;
  if (!imp.get().work) {
    imp.set(blank());
    if (importShown()) close();
  }
});
const showImport = () => ui.set({ dialog: { type: "import-games" } });

/** Import games from `paths` (dropped): added to the folders to read, unless an import is
 *  under way, which is shown instead. */
function importGamesFrom(paths) {
  const s = imp.get();
  if (!s.work && !s.review && !s.plan) imp.set({ folders: [...new Set([...s.folders, ...paths])] });
  showImport();
}
const importShown = () => ui.get().dialog?.type === "import-games";
const hide = () => { if (importShown()) close(); };

/** Run a job of the import's, followed in the window and the status bar. Its result, or
 *  null when it failed or was stopped. */
async function work(cmd, args, label) {
  imp.set({ error: "", work: { label, progress: null, job: null } });
  try {
    const { job } = await api(cmd, args);
    imp.set((s) => ({ work: s.work && { ...s.work, job } }));
    const done = await followJob(job, label, (j) => imp.set((s) => ({ work: s.work && { ...s.work, progress: j.progress || null } })), showImport);
    imp.set({ work: null });
    if (done.error) {
      if (/^Stopped/.test(done.error)) toast(`${label}: stopped.`);
      else imp.set({ error: done.error });
      return null;
    }
    return done.result;
  } catch (e) {
    imp.set({ work: null, error: err(e) });
    return null;
  }
}

/** Says a result is ready when the window isn't up, with a button that brings it back. */
const ready = (text) => { if (!importShown()) toast(text, { ms: 12000, action: { label: "Show", run: showImport } }); };

/** Leave them where they are: each folder becomes a folder with games (for the system
 *  chosen, or every system) and is scanned; nothing is copied. */
async function leaveFolders() {
  const s = imp.get();
  const typed = s.typed.trim();
  const paths = typed && !s.folders.includes(typed) ? [...s.folders, typed] : s.folders;
  const added = [], failed = [];
  for (const path of paths) {
    try {
      await api("roots_add", { path, platform: s.only || null });
      added.push(path);
    } catch (e) {
      // one that's a folder with games already (or inside one) is scanned as it is
      if (/already/.test(e.message || "")) added.push(path);
      else failed.push(`${baseName(path)}: ${e.message || e}`);
    }
  }
  if (failed.length) return imp.set({ folders: paths, typed: "", error: failed.join(" ") });
  imp.set(blank());
  close();
  await refreshGames();
  toast(`${plural(added.length, "folder is", "folders are")} in Folders with games now, and scanned; the games stay where they are. Import › Move into the library… brings them in when you want.`, 12000);
  scan();
}

async function readFolders() {
  const s = imp.get();
  const typed = s.typed.trim();
  const paths = typed && !s.folders.includes(typed) ? [...s.folders, typed] : s.folders;
  imp.set({ folders: paths, typed: "" });
  const r = await work("collection_read_folders", { paths }, "Reading folders");
  if (!r) return;
  // the folders no DAT knows the files of: each the system it seems to be (or the one chosen)
  const groups = {};
  for (const u of r.unknown || []) if (u.file) (groups[dirOf(u.path)] ||= []).push(u);
  const given = {};
  for (const [dir, us] of Object.entries(groups)) {
    const seems = [...new Set(us.map((u) => u.system || ""))];
    given[dir] = s.only || (seems.length === 1 ? seems[0] : "");
  }
  imp.set({
    review: { ...r, groups }, off: [], given,
    lists: Object.fromEntries(r.systems.map((x) => [x.platform, x.places || []])),
    where: Object.fromEntries(r.systems.map((x) => [x.platform, x.places?.[0]?.dir || ""])),
  });
  fetchPlaces(given);
  ready(`Read the folders: ${plural(wanted().length, "game", "games")} to import.`);
}

/** The places of a system the read found no games of, once its files are given it. */
function fetchPlaces(given) {
  for (const p of new Set(Object.values(given).filter(Boolean))) {
    if (imp.get().lists[p]) continue;
    imp.set((s) => ({ lists: { ...s.lists, [p]: [] } }));
    api("add_places", { platform: p }).then((ps) => imp.set((s) => ({ lists: { ...s.lists, [p]: ps }, where: s.where[p] ? s.where : { ...s.where, [p]: ps[0]?.dir || "" } }))).catch(() => {});
  }
}

const systemsOf = (s) => (s.review?.systems || []).filter((x) => !s.only || x.platform === s.only);
// what can come in: games the library doesn't have, with the files that are them
const wanted = (s = imp.get()) => systemsOf(s).flatMap((x) => x.games.map((g) => ({ ...g, platform: x.platform }))).filter((g) => g.status === "missing" && g.files?.length);
const looseFiles = (s = imp.get()) => Object.entries(s.review?.groups || {}).filter(([d]) => s.given[d]).map(([d, us]) => ({ platform: s.given[d], files: us.map((u) => u.file) }));

// ---- into a collection too (D140)

/** The games read that go in the collection: every one found (the library's own too), the
 *  ones it lacks as ticked. */
const collected = (s = imp.get()) => systemsOf(s).flatMap((x) => x.games).filter((g) => !(g.status === "missing" && g.files?.length && s.off.includes(g.id)));
/** The files no DAT knows that were given a system: they wait on the collection. */
const waitingOf = (s = imp.get()) => Object.entries(s.review?.groups || {}).filter(([d]) => s.given[d])
  .flatMap(([d, us]) => us.map((u) => ({ platform: s.given[d], name: baseName(u.path), roms: u.roms || [] })));
const intoName = (s) => (s.intoName || "").trim() || baseName(s.folders[0] || "Imported");

/** Make the collection (or the existing one's new group) of the games read; null when none
 *  was asked for. */
async function intoCollection(s) {
  if (!s.into || !s.review) return null;
  const made = s.into === "new";
  const name = intoName(s);
  const r = await api("collection_from", {
    name: made ? name : "", id: made ? null : s.into, group: "", ...(made ? {} : { group_name: name }),
    games: collected(s).map((g) => g.id), waiting: waitingOf(s), made_from: { ...s.review.made_from, kind: "import" },
  });
  ui.set((st) => ({ changes: (st.changes || 0) + 1 }));
  return { collection: r.collection, name, made };
}

/** What's said after the import about the collection, with a button to it. */
function collectionNote(done, waiting) {
  if (!done) return null;
  const c = done.collection;
  const what = done.made ? `${c.name} is a collection of them` : `${c.name} has a new group of them, ${done.name}`;
  return { text: `${what}${waiting ? `; ${plural(waiting, "file waits", "files wait")} on Audit › Unknown to join it` : ""}.`, actions: [{ label: "Open it", run: () => { location.hash = argHash("collections", c.id); } }] };
}

/** Nothing to import, only a collection to make of the games the library has. */
async function collectOnly() {
  const s = imp.get();
  try {
    const done = await intoCollection(s);
    imp.set(blank());
    close();
    const note = collectionNote(done, waitingOf(s).length);
    toast(note.text, { ms: 12000, action: note.actions[0] });
  } catch (e) {
    imp.set({ error: err(e) });
  }
}

async function planImport() {
  const s = imp.get();
  const games = wanted(s).filter((g) => !s.off.includes(g.id));
  const plan = await work("collection_bring_plan", {
    games: games.map((g) => ({ id: g.id, files: g.files })), files: looseFiles(s), places: s.where,
    ...howArgs(s.mode),
  }, "Checking the files to import");
  if (!plan) return;
  imp.set({ plan });
  ready("The import's preview is ready.");
}

/** While a job of the import's runs: how far it's got, Stop, and Run in the background. */
function Working({ w }) {
  const p = w.progress || {};
  const pct = p.items ? Math.round((100 * Math.min(p.item + 1, p.items)) / p.items) : null;
  return html`<${Dialog} title=${w.label} id="import-games-working" sticky=${true} onSubmit=${hide} submitLabel="Run in the background"
      cancelLabel="Stop" onCancel=${() => w.job && api("job_cancel", { id: w.job })} onDismiss=${hide}>
    <p id="import-games-progress">${p.items ? `${p.text || "Working"}: ${Math.min(p.item + 1, p.items)} of ${p.items}` : p.text || "Starting…"}${p.name ? html` <span class="muted">${p.name}</span>` : null}</p>
    <div class="audit-bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow=${pct ?? 0}><span style=${`width:${pct ?? 0}%`}></span></div>
    <p class="muted">The window stays until it's done or stopped. Run in the background to carry on elsewhere: the status bar, bottom right, brings you back here.</p>
  <//>`;
}

/** Games into the library: folders read and matched against the audit DATs (every
 *  system's, or one system's), then each system's games copied or moved into its own
 *  folder with games under their DAT names; files no DAT knows go in if given a system. */
function ImportGamesDialog() {
  const platforms = (useStore(ui, (s) => s.platforms) || []).filter((p) => p.id !== "pc").sort((a, b) => a.name.localeCompare(b.name));
  const s = useStore(imp);
  const nameOf = (id) => platforms.find((p) => p.id === id)?.name || id;
  // the collections the games can go in (D140)
  const [collections, setCollections] = useState([]);
  useEffect(() => { api("collections").then(setCollections).catch(() => {}); }, []);
  if (s.work) return html`<${Working} w=${s.work} />`;
  if (s.plan) return html`<${ImportGamesPreview} plan=${s.plan} />`;
  const add = (p) => { if (p && !s.folders.includes(p)) imp.set({ folders: [...s.folders, p] }); };
  const how = s.mode;
  if (s.review) {
    const review = s.review;
    const can = wanted(s);
    const picked = can.filter((g) => !s.off.includes(g.id));
    const tick = (id) => imp.set({ off: s.off.includes(id) ? s.off.filter((x) => x !== id) : [...s.off, id] });
    const groups = Object.entries(review.groups);
    const loose = looseFiles(s).reduce((n, f) => n + f.files.length, 0);
    // a row for each system something goes to
    const count = {};
    for (const g of picked) count[g.platform] = (count[g.platform] || 0) + 1;
    for (const f of looseFiles(s)) count[f.platform] = (count[f.platform] || 0) + f.files.length;
    const going = Object.keys(count).sort((a, b) => nameOf(a).localeCompare(nameOf(b)));
    const unplaced = going.filter((p) => !s.where[p]);
    const unknownCount = (review.unknown || []).length;
    const give = (d, p) => { const given = { ...s.given, [d]: p }; imp.set({ given }); fetchPlaces(given); };
    // into a collection too (D140): with nothing to import, only the collection is made
    const inColl = s.into ? collected(s) : [];
    const waits = s.into ? waitingOf(s).length : 0;
    const only = !(picked.length + loose) && inColl.length + waits > 0;
    const had = inColl.filter((g) => g.status === "have").length;
    const intoLine = s.into ? html`<p id="import-into-line">${s.into === "new" ? html`Into a new collection, <b>${intoName(s)}</b>` : html`Into <b>${collections.find((c) => c.id === s.into)?.name || "the collection"}</b>, as a new group, <b>${intoName(s)}</b>`}:${" "}
      ${plural(inColl.length, "game", "games")}${had ? ` (${had} in the library already)` : ""}${waits ? `, and ${plural(waits, "file", "files")} no DAT knows, which ${waits === 1 ? "waits" : "wait"} on Audit › Unknown to join it once a DAT knows ${waits === 1 ? "it" : "them"}` : ""}.</p>` : null;
    // what was found first, then where it goes (review M42)
    return html`<${Dialog} title="The folders' games" id="import-games-review" wide=${true} sticky=${true} onDismiss=${hide} onSubmit=${only ? collectOnly : planImport} busy=${!(picked.length + loose + inColl.length + waits) || unplaced.length > 0} error=${s.error}
      submitLabel=${only ? (s.into === "new" ? "Make the collection" : "Add the group") : `Preview (${plural(picked.length + loose, "game", "games")})`} cancelLabel="Back" onCancel=${() => imp.set({ review: null, error: "" })}>
      <p id="import-games-totals">Read ${plural(review.read, "file", "files")}: <b>${plural(can.length, "game", "games")}</b> the library doesn't have${s.only ? ` for ${nameOf(s.only)}` : ""}${unknownCount ? `, ${plural(unknownCount, "file", "files")} no DAT knows` : ""}.
        ${s.mode === "move" ? " They'll be moved." : s.mode === "always" ? " They'll be copied in full." : " They'll be copied (hardlinked on the same drive)."}</p>
      ${intoLine}
      <div class="subset-systems" id="import-games-systems">${systemsOf(s).map((x) => html`<section key=${x.platform} class="subset-system" data-platform=${x.platform}>
        <h3>${x.name} <span class="muted">${plural(x.games.length, "game", "games")}, ${x.have} in the library</span></h3>
        <ul class="ls-list organise-list">${x.games.map((g) => html`<li key=${g.id} data-name=${g.name} data-status=${g.status}>
          ${g.status === "missing" && g.files?.length ? html`<label class="check-row"><input type="checkbox" checked=${!s.off.includes(g.id)} onChange=${() => tick(g.id)} /> ${g.name}</label>`
            : html`<span class="muted">${g.name}</span>`}
          <span class="muted" title=${g.from.join("\n")}>${g.status === "have" ? "in the library · " : g.status === "partial" ? "partly in the library · " : ""}${g.from.length > 1 ? plural(g.from.length, "file", "files") : g.from[0]}</span></li>`)}</ul>
      </section>`)}</div>
      ${groups.length ? html`<div class="field-block subset-unknown" id="import-games-unknown"><span class="field-label">Files no DAT knows</span>
        <small class="muted">Give a folder's files a system to import them as they're named, or leave them out.</small>
        <ul class="ls-list">${groups.map(([d, us]) => html`<li key=${d} class="import-unknown" data-folder=${d}>
          <span title=${us.map((u) => u.path).join("\n")}><span class="op-to">${d || "(the folder read)"}</span> <span class="muted">${plural(us.length, "file", "files")}</span></span>
          <select class="compact" aria-label=${`The system of the files in ${d}`} value=${s.given[d] || ""} onChange=${(e) => give(d, e.target.value)}>
            <option value="">Leave them out</option>
            ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
          </select></li>`)}</ul></div>` : null}
      ${(review.unknown || []).some((u) => !u.file) ? html`<details class="field-block"><summary>${plural(review.unknown.filter((u) => !u.file).length, "file", "files")} that couldn't be read</summary>
        <ul class="ls-list">${review.unknown.filter((u) => !u.file).slice(0, 200).map((u, i) => html`<li key=${i}><span class="op-to">${u.path}</span> <span class="muted">${u.why}</span></li>`)}</ul></details>` : null}
      ${going.length ? html`<div class="field-block"><span class="field-label">Where each system's games go</span>
        <ul class="ls-list import-places" id="import-places">${going.map((p) => html`<${PlaceRow} key=${p} platform=${p} name=${nameOf(p)} count=${count[p]} places=${s.lists[p]} value=${s.where[p]}
          onChange=${(d) => imp.set((x) => ({ where: { ...x.where, [p]: d } }))} />`)}</ul></div>` : null}
      <p class="muted">Each game is named as its DAT says. Nothing changes until you've seen the preview.</p>
    <//>`;
  }
  const typed = s.typed.trim();
  const leave = how === "leave";
  return html`<${Dialog} title="Import games from folders" id="import-games-dialog" sticky=${true} onDismiss=${hide} onSubmit=${leave ? leaveFolders : readFolders} busy=${!(s.folders.length || typed)} error=${s.error}
      submitLabel=${leave ? "Add the folders" : "Read"}
      onCancel=${() => { imp.set(blank()); close(); }}>
    <div class="field-block"><span class="field-label">Folders</span>
      ${s.folders.length ? html`<ul class="ls-list" id="import-folders">${s.folders.map((f) => html`<li key=${f}><span class="op-to">${f}</span>
        <button type="button" class="ghost compact" aria-label=${`Take ${baseName(f)} off`} onClick=${() => imp.set({ folders: s.folders.filter((x) => x !== f) })}>${Icon.close(13)}</button></li>`)}</ul>` : null}
      <div class="pick-row">
        <input type="text" id="import-folder-path" placeholder="A folder's path" value=${s.typed} onInput=${(e) => imp.set({ typed: e.target.value })} onKeyDown=${(e) => { if (e.key === "Enter" && typed) { e.preventDefault(); add(typed); imp.set({ typed: "" }); } }} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="import-add-folder" onClick=${async () => add(await ctx.platform.library.pickFolder("A folder of games to import"))}>Add a folder…</button>` : null}
      </div>
      <small class="muted">${leave ? "They're added to Folders with games and scanned, and their games are found by your DATs where they are." : "Every file in them is read and matched against your DATs."}</small></div>
    <label class="field-block"><span class="field-label">System</span>
      <select id="import-system" class="compact" value=${s.only} onChange=${(e) => imp.set({ only: e.target.value })}>
        <option value="">Every system (each game's own, found by its DAT)</option>
        ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      <small class="muted">${s.only ? `Only ${nameOf(s.only)}'s games, and files no DAT knows are taken as ${nameOf(s.only)}'s.` : "Each system's games go to its own folder; you'll be asked about files no DAT knows."}</small></label>
    <${HowControl} id="import-mode" value=${how} onChange=${(mode) => imp.set({ mode })} leave=${true} />
    ${!leave ? html`<div class="field-block" id="import-into"><span class="field-label">Also put these games in a collection</span>
      <div class="pick-row">
        <select id="import-into-pick" class="compact" aria-label="Also put these games in a collection" value=${s.into} onChange=${(e) => imp.set({ into: e.target.value })}>
          <option value="">No</option>
          <option value="new">A new collection</option>
          ${collections.map((c) => html`<option key=${c.id} value=${c.id}>A new group of ${c.name}</option>`)}
        </select>
        ${s.into ? html`<input type="text" id="import-into-name" class="compact" maxlength="120" aria-label=${s.into === "new" ? "The collection's name" : "The group's name"}
          placeholder=${s.folders[0] || typed ? baseName(s.folders[0] || typed) : s.into === "new" ? "The collection's name" : "The group's name"} value=${s.intoName} onInput=${(e) => imp.set({ intoName: e.target.value })} />` : null}
      </div>
      <small class="muted">${s.into ? "Every game read goes in it, the library's own too. Files no DAT knows that you give a system wait on Audit › Unknown, and join it once a DAT knows them (added to the system's custom DAT, say)." : "The games are imported into the library, and nothing more."}</small></div>` : null}
  <//>`;
}

/** The preview of the import; Run starts it, Back goes back to the folders' games. */
function ImportGamesPreview({ plan }) {
  const error = useStore(imp, (st) => st.error);
  const nothing = !plan.ops;
  const run = async () => {
    const s = imp.get();
    // the collection first (D140): one that can't be made (its name taken, say) stops here
    let done = null;
    try {
      done = await intoCollection(s);
    } catch (e) {
      imp.set({ error: err(e) });
      return;
    }
    // files no DAT knows that come in wait on Audit's Unknown tab (D123)
    const loose = looseFiles(s);
    const n = loose.reduce((a, x) => a + x.files.length, 0);
    const review = n ? { text: `${plural(n, "file", "files")} no DAT knows ${n === 1 ? "needs" : "need"} review.`, actions: [{ label: "Review them", run: () => reviewUnknown(loose[0].platform) }] } : null;
    const note = collectionNote(done, done ? waitingOf(s).length : 0);
    const more = review || note ? { text: [review?.text, note?.text].filter(Boolean).join(" "), actions: [...(review?.actions || []), ...(note?.actions || [])] } : null;
    close();
    imp.set(blank());
    if (!nothing) runPlan(plan, more);
    else if (note) toast(note.text, { ms: 12000, action: note.actions[0] });
  };
  return html`<${Dialog} title=${plan.title || "Import the games"} id="bring-dialog" wide=${true} sticky=${true} onDismiss=${hide} onSubmit=${run} submitLabel=${nothing ? "OK" : "Import"} busy=${plan.short} error=${error}
      cancelLabel="Back" onCancel=${() => imp.set({ plan: null, error: "" })}>
    <${Preview} plan=${plan} />
    ${nothing ? html`<p id="organise-nothing">Nothing to import: ${plan.left?.length ? "see what's left alone." : "the library has them all."}</p>`
      : html`<p class="muted">${plan.mode === "move" ? "Moved, then checked and scanned." : "Copied (or hardlinked, as chosen), then checked and scanned."}${plan.new_roots?.length ? ` ${plan.new_roots.length === 1 ? "A new folder becomes a folder with games" : "The new folders become folders with games"} for ${plan.new_roots.length === 1 ? "its system" : "their systems"}.` : ""} History can undo it.</p>`}
  <//>`;
}

// ------------------------------------------------------------ files dropped (D78)

/** What files dropped on the window are taken for: [id, label, about, the Import page's section]. */
const DROPPED = [
  ["games", "Games", "Matched against your DATs and copied or moved into each system's folder with games.", "import-copy"],
  ["pc", "PC games", "Installers, archives and game folders, each into its own folder under PC/.", "import-pc"],
  ["docs", "Documents", "Manuals, guides and scans, each paired with its game.", "import-docs"],
  ["mags", "Magazines", "Issues (PDF, CBZ or CBR) into Magazines/, by magazine.", "import-mags"],
  ["videos", "Videos", "Walkthroughs, reviews and the like into Videos/, by kind.", "import-videos"],
  ["dats", "DATs", "Lists of games to audit your files against.", "settings-dats"],
  ["bios-dat", "A BIOS DAT", "A list of BIOS files to audit them against.", "import-bios"],
];

const extOf = (p) => (/\.([^.\\/]+)$/.exec(p)?.[1] || "").toLowerCase();

/** What dropped files most likely are, from their extensions (games when it can't tell). */
export function guessDropped(paths) {
  const all = (exts) => paths.every((p) => exts.includes(extOf(p)));
  if (all(["dat", "xml"])) return "dats";
  if (all(["cbz", "cbr"])) return "mags";
  if (all(["mp4", "m4v", "mov", "webm", "mkv", "avi"])) return "videos";
  if (all(["exe", "msi"])) return "pc";
  if (all(["pdf", "txt", "md", "htm", "html", "doc", "docx", "rtf"])) return "docs";
  return "games";
}

/** Each kind's import, given the paths. */
function startDropped(kind, paths) {
  if (kind === "games") return importGamesFrom(paths);
  if (kind === "pc") return importPc("paths", paths);
  if (kind === "docs") return attachDocs({ how: "files", paths });
  if (kind === "mags") return importMagazines(false, paths);
  if (kind === "videos") return importVideos(false, paths);
  if (kind === "dats") { close(); return importDats(paths); }
  if (kind === "bios-dat") { close(); return importDats(paths, { "*": "bios" }); }
}

/** Files dropped where no page takes them (D78): to the Import page, asked what they are.
 *  `hint` is the page's guess (Magazines', say). */
export function dropToImport(paths, hint = null) {
  const lib = ui.get().library;
  if (!lib) return toast("Open a library first, then drop the files on the window again.", 8000);
  if (lib.read_only) return toast("This library is open read-only, so nothing can be imported into it.", 8000);
  importFrom("import-dropped");
  ui.set({ dialog: { type: "import-drop", paths, kind: typeof hint === "string" ? hint : guessDropped(paths) } });
}

function DropDialog({ paths, kind: guessed }) {
  const [kind, setKind] = useState(guessed);
  const names = paths.map((p) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop());
  return html`<${Dialog} title=${`Import ${paths.length === 1 ? names[0] : plural(paths.length, "thing", "things")}`} id="import-drop-dialog" onSubmit=${() => startDropped(kind, paths)} submitLabel="Next">
    <p class="muted" id="import-drop-names" title=${paths.join("\n")}>${names.slice(0, 3).join(", ")}${names.length > 3 ? ` and ${names.length - 3} more` : ""}</p>
    <div class="field-block way-pick" role="radiogroup" aria-label="What they are" id="import-drop-kind">${DROPPED.filter(([k]) => isDesktop() || k !== "pc").map(([k, label, about]) => html`<label key=${k} class="way" data-kind=${k}>
      <input type="radio" name="import-drop-kind" value=${k} checked=${kind === k} onChange=${() => setKind(k)} />
      <span><b>${label}</b><small class="muted">${about}</small></span></label>`)}</div>
    <p class="muted">Each kind is read and shown before anything changes.</p>
  <//>`;
}

export function ImportDialogs({ dialog }) {
  if (dialog.type === "import-games" || dialog.type === "bring") return html`<${ImportGamesDialog} />`;
  if (dialog.type === "import-drop") return html`<${DropDialog} key=${dialog.paths.join("|")} ...${dialog} />`;
  return null;
}

export const importGamesDialog = showImport;

/** One kind of thing to import: what it is, where it goes, its buttons. `here` when another
 *  page opened the Import page at it. */
function Card({ id, icon, title, where, here, children }) {
  return html`<section class=${here ? "import-card import-here" : "import-card"} id=${id}>
    <h2>${Icon[icon](16)} ${title}</h2>
    <p class="muted">${where}</p>
    <div class="home-actions">${children}</div>
  </section>`;
}

export function ImportPage() {
  const lib = useStore(ui, (s) => s.library);
  const roots = useStore(ui, (s) => s.roots) || [];
  const at = useRouteArg("import");
  useEffect(() => {
    if (at) document.getElementById(at)?.scrollIntoView({ block: "nearest" });
  }, [at, lib?.path]);
  const head = html`<${CommandBar} id="import-head" title="Import" />`;
  if (!lib) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const off = !!lib.read_only;
  const desk = isDesktop();
  const away = roots.filter((r) => r.outside && r.found);
  return html`${head}<div class="pages import-page" id="import-page" data-at=${at || ""}><div class="library-settings">
    <p class="page-lead">${Icon.inbox(18)} Everything that comes into the library comes in here, and files dropped on the window too. Checking it is the <a href=${routeHash("audit")}>Audit</a>'s; choosing games to export is <a href=${routeHash("collections")}>Collections</a>'.</p>
    <div class="import-cards">
      <${Card} id="import-copy" icon="move" title="Games into the library" here=${at === "import-copy"} where="Matched against your DATs and copied or moved into each system's folder with games, named as the DAT says, or left where they are.">
        <button type="button" class="primary" id="import-games" disabled=${off} onClick=${importGamesDialog}>From a folder…</button>
        <button type="button" class="ghost" id="import-one" disabled=${off} onClick=${() => addGameDialog()}>One game…</button>
        ${away.length ? html`<button type="button" class="ghost" id="import-gather" disabled=${off} title=${`Every game in ${away.map((r) => r.dir).join(", ")}, moved into each system's folder in the library, with the preview first`}
          onClick=${() => gatherDialog()}>Move into the library…</button>` : null}
      <//>
      <${Card} id="import-pc" icon="monitor" title="PC games" here=${at === "import-pc"} where="Installers, archives and game folders, each into its own folder under PC/.">
        <button type="button" class="ghost" id="import-pc-files" disabled=${off || !desk} onClick=${() => importPc("files")}>Files…</button>
        <button type="button" class="ghost" id="import-pc-folder" disabled=${off || !desk} onClick=${() => importPc("folder")}>A game's folder…</button>
        <button type="button" class="ghost" id="import-pc-inbox" disabled=${off} title="Read the PC games inbox (Settings › PC games)" onClick=${() => importPc("inbox")}>The inbox…</button>
      <//>
      <${Card} id="import-bios" icon="chip" title="BIOS files" here=${at === "import-bios"} where="Copied or moved into the library's BIOS folder, by system, from your folders with games.">
        <button type="button" class="ghost" id="import-bios-files" disabled=${off} onClick=${importBios}>Import BIOS files…</button>
        <button type="button" class="ghost" id="import-bios-list" disabled=${off} onClick=${() => starterDats(["bios"])}>Get libretro's list…</button>
        ${desk ? html`<button type="button" class="ghost" id="import-bios-dat" disabled=${off} onClick=${() => importDats(null, { "*": "bios" })}>A BIOS DAT…</button>` : null}
      <//>
      <${Card} id="import-docs" icon="book" title="Documents" here=${at === "import-docs"} where="Manuals, guides and scans, each paired with its game, into the game's library folder.">
        <button type="button" class="ghost" id="import-docs-folder" disabled=${off || !desk} onClick=${() => attachDocs({ how: "folder" })}>A folder…</button>
        <button type="button" class="ghost" id="import-docs-files" disabled=${off || !desk} onClick=${() => attachDocs({ how: "files" })}>Files…</button>
      <//>
      <${Card} id="import-mags" icon="stack" title="Magazines" here=${at === "import-mags"} where="Issues (PDF, CBZ or CBR) into Magazines/, by magazine.">
        <button type="button" class="ghost" id="import-mags-folder" disabled=${off || !desk} onClick=${() => importMagazines()}>A folder…</button>
        <button type="button" class="ghost" id="import-mags-files" disabled=${off || !desk} onClick=${() => importMagazines(true)}>Files…</button>
      <//>
      <${Card} id="import-videos" icon="video" title="Videos" here=${at === "import-videos"} where="Walkthroughs, reviews, retrospectives and others into Videos/, by kind, each with the games it's about; or a link to one on the web.">
        <button type="button" class="ghost" id="import-videos-folder" disabled=${off || !desk} onClick=${() => importVideos()}>A folder…</button>
        <button type="button" class="ghost" id="import-videos-files" disabled=${off || !desk} onClick=${() => importVideos(true)}>Files…</button>
        <button type="button" class="ghost" id="import-videos-link" disabled=${off} onClick=${addVideoLink}>A link…</button>
      <//>
    </div>
    <section class="ls-section" id="import-recent">
      <h2>Recent imports</h2>
      <p class="muted">What's come in lately, a day at a time; Steps lists what an import did. The newest change can be undone here or in <a href=${routeHash("history")}>History</a>, which has every change.</p>
      <${ChangeHistory} lib=${lib} id="import-recent-list" kinds=${IMPORT_KINDS} limit=${12} none="Nothing imported yet." />
    </section>
    <${RootsSection} lib=${lib} key=${`f-${lib.path}`} />
    <${DatsSection} lib=${lib} />
  </div></div>`;
}
