// Import (docs/PLAN.md, "UI pass 3 design", D71 and D73): everything that brings files into
// the library, in one place. Games in folders you keep (the folders with games, scanned
// where they are), games copied into the library from a folder (matched by the audit DATs,
// named as they say, D53's bring-in) or one at a time (Add a game…), PC games, DATs, BIOS
// files, documents and magazines. Other pages link here instead of repeating these.
import { html } from "../lib/html.js";
import { createStore, useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, followJob, isDesktop, plural, toast } from "./library.js";
import { CommandBar } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { Preview, runPlan } from "./organise.js";
import { RootsSection, DatsSection } from "./settings.js";
import { importPc, addGameDialog } from "./pcgames.js";
import { importBios } from "./bios.js";
import { attachDocs } from "./documents.js";
import { importMagazines } from "./magazines.js";
import { importDats, starterDats } from "./games.js";
import { Icon } from "./icons.js";

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
const blank = () => ({ folders: [], typed: "", only: "", mode: "copy", review: null, off: [], where: {}, lists: {}, given: {}, work: null, plan: null, error: "" });
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

async function planImport() {
  const s = imp.get();
  const games = wanted(s).filter((g) => !s.off.includes(g.id));
  const plan = await work("collection_bring_plan", {
    games: games.map((g) => ({ id: g.id, files: g.files })), files: looseFiles(s), places: s.where,
    mode: s.mode === "move" ? "move" : "copy", always_copy: s.mode === "always",
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
  if (s.work) return html`<${Working} w=${s.work} />`;
  if (s.plan) return html`<${ImportGamesPreview} plan=${s.plan} />`;
  const add = (p) => { if (p && !s.folders.includes(p)) imp.set({ folders: [...s.folders, p] }); };
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
    return html`<${Dialog} title="The folders' games" id="import-games-review" wide=${true} sticky=${true} onSubmit=${planImport} busy=${!(picked.length + loose) || unplaced.length > 0} error=${s.error}
      submitLabel=${`Preview (${plural(picked.length + loose, "game", "games")})`} cancelLabel="Back" onCancel=${() => imp.set({ review: null, error: "" })}>
      <p id="import-games-totals">Read ${plural(review.read, "file", "files")}: <b>${plural(can.length, "game", "games")}</b> the library doesn't have${s.only ? ` for ${nameOf(s.only)}` : ""}${unknownCount ? `, ${plural(unknownCount, "file", "files")} no DAT knows` : ""}.
        ${s.mode === "move" ? " They'll be moved." : s.mode === "always" ? " They'll be copied in full." : " They'll be copied (hardlinked on the same drive)."}</p>
      ${going.length ? html`<div class="field-block"><span class="field-label">Where each system's games go</span>
        <ul class="ls-list import-places" id="import-places">${going.map((p) => html`<${PlaceRow} key=${p} platform=${p} name=${nameOf(p)} count=${count[p]} places=${s.lists[p]} value=${s.where[p]}
          onChange=${(d) => imp.set((x) => ({ where: { ...x.where, [p]: d } }))} />`)}</ul></div>` : null}
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
      <p class="muted">Each game is named as its DAT says. Nothing changes until you've seen the preview.</p>
    <//>`;
  }
  const typed = s.typed.trim();
  return html`<${Dialog} title="Import games from folders" id="import-games-dialog" sticky=${true} onSubmit=${readFolders} busy=${!(s.folders.length || typed)} error=${s.error} submitLabel="Read"
      onCancel=${() => { imp.set(blank()); close(); }}>
    <div class="field-block"><span class="field-label">Folders</span>
      ${s.folders.length ? html`<ul class="ls-list" id="import-folders">${s.folders.map((f) => html`<li key=${f}><span class="op-to">${f}</span>
        <button type="button" class="ghost compact" aria-label=${`Take ${baseName(f)} off`} onClick=${() => imp.set({ folders: s.folders.filter((x) => x !== f) })}>${Icon.close(13)}</button></li>`)}</ul>` : null}
      <div class="pick-row">
        <input type="text" id="import-folder-path" placeholder="A folder's path" value=${s.typed} onInput=${(e) => imp.set({ typed: e.target.value })} onKeyDown=${(e) => { if (e.key === "Enter" && typed) { e.preventDefault(); add(typed); imp.set({ typed: "" }); } }} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="import-add-folder" onClick=${async () => add(await ctx.platform.library.pickFolder("A folder of games to import"))}>Add a folder…</button>` : null}
      </div>
      <small class="muted">Every file in them is read and matched against your DATs.</small></div>
    <label class="field-block"><span class="field-label">System</span>
      <select id="import-system" class="compact" value=${s.only} onChange=${(e) => imp.set({ only: e.target.value })}>
        <option value="">Every system (each game's own, found by its DAT)</option>
        ${platforms.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      <small class="muted">${s.only ? `Only ${nameOf(s.only)}'s games, and files no DAT knows are taken as ${nameOf(s.only)}'s.` : "Each system's games go to its own folder; you'll be asked about files no DAT knows."}</small></label>
    <div class="field-block" role="radiogroup" aria-label="How" id="import-mode">
      <label class="check-row"><input type="radio" name="import-mode" value="copy" checked=${s.mode === "copy"} onChange=${() => imp.set({ mode: "copy" })} /> Copy them, hardlinked on the same drive (they take no space; the originals stay)</label>
      <label class="check-row"><input type="radio" name="import-mode" value="always" checked=${s.mode === "always"} onChange=${() => imp.set({ mode: "always" })} /> Copy them, always (a full copy, even on the same drive)</label>
      <label class="check-row"><input type="radio" name="import-mode" value="move" checked=${s.mode === "move"} onChange=${() => imp.set({ mode: "move" })} /> Move them into the library</label>
    </div>
  <//>`;
}

/** The preview of the import; Run starts it, Back goes back to the folders' games. */
function ImportGamesPreview({ plan }) {
  const nothing = !plan.ops;
  const run = () => { close(); imp.set(blank()); if (!nothing) runPlan(plan); };
  return html`<${Dialog} title=${plan.title || "Import the games"} id="bring-dialog" wide=${true} sticky=${true} onSubmit=${run} submitLabel=${nothing ? "OK" : "Run"} busy=${plan.short}
      cancelLabel="Back" onCancel=${() => imp.set({ plan: null })}>
    <${Preview} plan=${plan} />
    ${nothing ? html`<p id="organise-nothing">Nothing to import: ${plan.left?.length ? "see what's left alone." : "the library has them all."}</p>`
      : html`<p class="muted">${plan.mode === "move" ? "Moved, then checked and scanned." : "Copied (or hardlinked, as chosen), then checked and scanned."}${plan.new_roots?.length ? ` ${plan.new_roots.length === 1 ? "A new folder becomes a folder with games" : "The new folders become folders with games"} for ${plan.new_roots.length === 1 ? "its system" : "their systems"}.` : ""} History can undo it.</p>`}
  <//>`;
}

export function ImportDialogs({ dialog }) {
  if (dialog.type === "import-games" || dialog.type === "bring") return html`<${ImportGamesDialog} />`;
  return null;
}

export const importGamesDialog = showImport;

/** One kind of thing to import: what it is, where it goes, its buttons. */
function Card({ id, icon, title, where, children }) {
  return html`<section class="import-card" id=${id}>
    <h2>${Icon[icon](16)} ${title}</h2>
    <p class="muted">${where}</p>
    <div class="home-actions">${children}</div>
  </section>`;
}

export function ImportPage() {
  const lib = useStore(ui, (s) => s.library);
  const head = html`<${CommandBar} id="import-head" title="Import" />`;
  if (!lib) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  const off = !!lib.read_only;
  const desk = isDesktop();
  return html`${head}<div class="pages import-page" id="import-page"><div class="library-settings">
    <p class="page-lead">${Icon.inbox(18)} Everything that comes into the library comes in here. Checking it is the <a href=${routeHash("audit")}>Audit</a>'s; choosing games to export is <a href=${routeHash("collections")}>Collections</a>'.</p>
    <div class="import-cards">
      <${Card} id="import-copy" icon="move" title="Games into the library" where="Matched against your DATs and copied or moved into each system's folder with games, named as the DAT says.">
        <button type="button" class="primary" id="import-games" disabled=${off} onClick=${importGamesDialog}>From a folder…</button>
        <button type="button" class="ghost" id="import-one" disabled=${off} onClick=${() => addGameDialog()}>One game…</button>
      <//>
      <${Card} id="import-pc" icon="monitor" title="PC games" where="Installers, archives and game folders, each into its own folder under PC/.">
        <button type="button" class="ghost" id="import-pc-files" disabled=${off || !desk} onClick=${() => importPc("files")}>Files…</button>
        <button type="button" class="ghost" id="import-pc-folder" disabled=${off || !desk} onClick=${() => importPc("folder")}>A game's folder…</button>
      <//>
      <${Card} id="import-bios" icon="chip" title="BIOS files" where="Copied into the library's BIOS folder, by system, from your folders with games.">
        <button type="button" class="ghost" id="import-bios-files" disabled=${off} onClick=${importBios}>Import BIOS files…</button>
        <button type="button" class="ghost" id="import-bios-list" disabled=${off} onClick=${() => starterDats(["bios"])}>Get libretro's list…</button>
        ${desk ? html`<button type="button" class="ghost" id="import-bios-dat" disabled=${off} onClick=${() => importDats(null, { "*": "bios" })}>A BIOS DAT…</button>` : null}
      <//>
      <${Card} id="import-docs" icon="book" title="Documents" where="Manuals, guides and scans, each paired with its game, into the game's folder.">
        <button type="button" class="ghost" id="import-docs-folder" disabled=${off || !desk} onClick=${() => attachDocs({ how: "folder" })}>A folder…</button>
        <button type="button" class="ghost" id="import-docs-files" disabled=${off || !desk} onClick=${() => attachDocs({ how: "files" })}>Files…</button>
      <//>
      <${Card} id="import-mags" icon="stack" title="Magazines" where="Issues (PDF, CBZ or CBR) into Magazines/, by magazine.">
        <button type="button" class="ghost" id="import-mags-folder" disabled=${off || !desk} onClick=${() => importMagazines()}>A folder…</button>
        <button type="button" class="ghost" id="import-mags-files" disabled=${off || !desk} onClick=${() => importMagazines(true)}>Files…</button>
      <//>
    </div>
    <${RootsSection} lib=${lib} key=${`f-${lib.path}`} />
    <${DatsSection} lib=${lib} />
    <p class="muted" id="import-history">What's been imported can be undone from <a href=${routeHash("history")}>History</a>.</p>
  </div></div>`;
}
