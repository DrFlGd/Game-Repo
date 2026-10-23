// Settings: the open library (name, folder, other libraries), its index on this
// computer, how the folders with games are scanned and watched, the systems (built-in and added), how games are
// named, the look, and the app's version with a link to About. Storage formats and external tools
// (Phase 2b), PC games, Identify's sources and Steam (Phase 4), and the sources' accounts and keys
// (Phase 5) have sections of their own, and so does Metadata (Phase 6b: which source each
// detail is taken from, and the space the library's metadata takes), and so does Serve to
// devices (Phase 9a, ui/serve.js: Game Repo Server with the open library), and so do Updates
// (D134, ui/updates.js). The folders with games and the DAT files are
// on the Import page and the history of changes on its own (D71), from the sections exported here.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, THEMES, SKINS, setTheme, setSkin } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, isDesktop, openLibrary, renameLibrary, showLibraryFolder, checkIndex, undoable, size, plural, followJob } from "./library.js";
import { undoChange, finishChange, CONVERT_FORMATS, ChangeSteps, AsideFiles } from "./organise.js";
import { addRoot, updateRoot, repointRoot, removeRoot, scanSettings, setScanSettings, importDats, starterDats, useDat, removeDat, refreshGames, reveal } from "./games.js";
import { addSystemDialog, redumpDialog } from "./addsystem.js";
import { confirmDialog } from "./dialogs.js";
import { SystemsSection } from "./systems.js";
import { CommandBar, DayGroups } from "./layout.js";
import { PcSettings, SourcesSettings, SteamSettings } from "./pcgames.js";
import { Icon } from "./icons.js";
import { ServePanel } from "./serve.js";
import { UpdatePanel } from "./updates.js";
import { readMagazines } from "./magazines.js";

function RenameForm({ lib }) {
  const [name, setName] = useState(lib.name || "");
  const [error, setError] = useState("");
  const submit = async (e) => {
    e.preventDefault();
    setError("");
    try {
      const before = lib.name;
      await renameLibrary(name);
      undoable(`Renamed the library ${name.trim()}.`, async () => { await renameLibrary(before); setName(before); });
    } catch (err) {
      setError(err.message || String(err));
    }
  };
  return html`<form class="inline-form" onSubmit=${submit}>
    <input id="library-name" type="text" maxlength="80" value=${name} onInput=${(e) => setName(e.target.value)} aria-label="Library name" disabled=${!!lib.read_only} />
    <button type="submit" class="ghost" disabled=${!!lib.read_only || name.trim() === lib.name}>Rename</button>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
  </form>`;
}

function IndexSettings({ index }) {
  const [busy, setBusy] = useState(false);
  if (!index) return null;
  const check = async () => { setBusy(true); await checkIndex(); setBusy(false); };
  return html`<div class="field-block" id="settings-index"><span class="field-label">Index on this computer</span>
    ${index.error ? html`<p class="warn-note" role="alert">${index.error}</p>` : html`
      <div class="pick-row"><span class="pick-path" id="index-path">${index.path}</span>
        <span class="muted" id="settings-index-size">${size(index.size)}</span>
        <button type="button" class="ghost" id="index-check" disabled=${busy} onClick=${check}>Check</button></div>`}
    <small class="muted">The index holds what the app works out from your files (hashes, DAT matches, games), so it's kept on each computer rather than in the library, and made again when it's missing.</small>
    ${index.tables ? html`<details class="index-details"><summary>What's in it</summary>
      <dl class="about-versions">
        <div><dt>Tables</dt><dd id="index-tables">${Object.entries(index.tables).map(([t, n]) => `${t} (${plural(n, "row", "rows")})`).join(", ")}</dd></div>
        <div><dt>Schema</dt><dd id="index-version">step ${index.version} of ${index.schema_version}</dd></div>
      </dl></details>` : null}
  </div>`;
}

/** Folders with games (kept in the library), on the Import page (D73): games kept where they are. */
export function RootsSection({ lib }) {
  const s = useStore(ui, (st) => ({ roots: st.roots, platforms: st.platforms }));
  useEffect(() => { refreshGames(); }, [lib.path]);
  const roots = s.roots || [];
  const off = !!lib.read_only;
  return html`<section class="ls-section" id="settings-folders">
    <h2>Folders with games</h2>
    <p class="muted">Games you keep in folders of your own, on this computer or a network drive: they stay where they are, and are scanned and audited there. Nothing in them changes unless you organise it. The list is kept in the library; on another computer, Point it elsewhere… says where a folder is there.</p>
    ${roots.length ? html`<ul class="ls-list root-list" id="roots">${roots.map((r) => html`<li key=${r.id} data-root=${r.id}>
      <span class="root-path"><span title=${r.dir}>${r.dir}</span>
        ${r.found ? null : html`<span class="warn-text">Not found on this computer</span>`}
        ${r.network ? html`<span class="muted root-note">A network drive: changes there are read when you scan.</span>` : null}</span>
      <select class="compact" aria-label="System of the files no DAT knows" title="The system of files in it that no DAT recognises (matched files take their DAT's)" value=${r.platform || ""} disabled=${off}
        onChange=${(e) => updateRoot(r.id, { platform: e.target.value || null })}>
        <option value="">System: worked out</option>
        ${(s.platforms || []).map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      ${isDesktop() && !r.found ? html`<button type="button" class="ghost" disabled=${off} onClick=${() => repointRoot(r)}>Point it elsewhere…</button>` : null}
      ${isDesktop() && r.found ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => ctx.platform.library.openPath(r.dir)}>${Icon.folder(14)}</button>` : null}
      <button type="button" class="ghost" data-action="remove-root" disabled=${off} onClick=${() => confirmDialog({
        title: "Stop scanning this folder?", text: `${r.dir} leaves the list, with what scans found in it. Nothing in the folder is changed or deleted.`,
        button: "Remove", run: () => removeRoot(r) })}>Remove…</button>
    </li>`)}</ul>` : html`<p id="roots-none">No folders yet.</p>`}
    ${isDesktop() ? html`<div><button type="button" class="ghost" id="add-root" disabled=${off} onClick=${() => addRoot()}>Add a folder…</button></div>` : null}
  </section>`;
}

/** How scans read the folders with games (kept on this computer). */
function ScanSection({ lib }) {
  const [scanOpts, setScanOpts] = useState(null);
  useEffect(() => { scanSettings().then(setScanOpts).catch(() => {}); }, [lib.path]);
  const change = async (patch) => setScanOpts(await setScanSettings(patch));
  return html`<section class="ls-section" id="settings-scan">
    <h2>Scanning</h2>
    <p class="muted">How the folders with games (<a href=${routeHash("import")}>Import</a>) are read. Kept on this computer.</p>
    ${scanOpts ? html`
      <label class="field-block"><span>Scan ZIPs</span>
        <select id="scan-zips" class="compact" value=${scanOpts.full ? "full" : "trust"} onChange=${(e) => change({ full: e.target.value === "full" })}>
          <option value="trust">Trust their checksums (quick)</option>
          <option value="full">Unpack and hash every file</option>
        </select>
        <small class="muted">A ZIP records each file's CRC32, so the quick scan matches ZIPs without unpacking them. Unpacking reads every file in full and matches by SHA1, which catches the rare ZIP whose checksums are wrong. Kept on this computer.</small></label>
      <label class="field-block"><span>Files read at once</span>
        <select id="scan-readers" class="compact" value=${String(scanOpts.readers)} onChange=${(e) => change({ readers: Number(e.target.value) })}>
          ${[1, 2, 3, 4, 6, 8].map((n) => html`<option key=${n} value=${String(n)}>${n}</option>`)}
        </select>
        <small class="muted">More is faster on SSDs and network drives; 1 suits a single spinning disk.</small></label>
      <div class="field-block"><label class="check-row"><input type="checkbox" id="scan-watch" checked=${scanOpts.watch} onChange=${(e) => change({ watch: e.target.checked })} /> Watch for changes</label>
        <small class="muted">While the app is open, files added, changed or removed in these folders are read a few seconds later, without a scan. Network drives don't report changes reliably, so they're read when you scan. Kept on this computer.</small></div>` : null}
  </section>`;
}

/** A system's main DAT, chosen here as on the Audit page (D131): the DATs it's audited with
 *  too stay ticked, less the new main one. */
async function makeMain(d, also) {
  try {
    await api("audit_dats_set", { platform: d.platform, main: d.id, also: also.filter((n) => n !== d.name) });
    ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
    await refreshGames();
    toast(`${d.name} is ${d.platform_name}'s main DAT now: Audit's totals are its counts.`);
  } catch (e) {
    toast(`Couldn't change the main DAT: ${e.message || e}`, 8000);
  }
}

/** The DAT files imported into the library, by platform and version; with more than one
 *  for a system, which is its main DAT and which it's audited with too (D118, D131). */
export function DatsSection({ lib }) {
  const dats = useStore(ui, (st) => st.dats) || [];
  const off = !!lib.read_only;
  // read again when shown: Audit changes the main DAT and those included in Audit
  useEffect(() => { api("dats").then((d) => ui.set({ dats: d })).catch(() => {}); }, [lib.path]);
  const byPlatform = [];
  for (const d of dats) {
    let g = byPlatform.find((x) => x.platform === d.platform);
    if (!g) byPlatform.push((g = { platform: d.platform, name: d.platform_name, dats: [] }));
    g.dats.push(d);
  }
  byPlatform.sort((a, b) => a.name.localeCompare(b.name));
  // the DATs a system can be audited against: each one's version in use, not its custom DAT
  const usable = (g) => g.dats.filter((d) => d.current && d.version !== "custom");
  const openDats = (g) => ui.set({ dialog: { type: "audit-dats", platform: g.platform, platformName: g.name, dats: usable(g), main: usable(g).find((d) => d.main)?.id,
    custom: g.dats.find((d) => d.version === "custom")?.name, onDone: () => refreshGames() } });
  return html`<section class="ls-section" id="settings-dats">
    <h2>DAT files</h2>
    <p class="muted">The DATs your games are audited against. Each is copied into the library (<code>_library/dats</code>); importing a newer version keeps the older one, and the newest is used unless you choose another. A system with several DATs has a main one, whose counts are Audit's totals.</p>
    ${byPlatform.length ? byPlatform.map((g) => {
      const several = usable(g).length > 1;
      const also = usable(g).filter((d) => d.also).map((d) => d.name);
      return html`<div class="dat-group" key=${g.platform} data-platform=${g.platform}><div class="dat-group-head"><h3>${g.name}</h3>
        ${several ? html`<button type="button" class="ghost compact dat-roles" title="The main DAT, and the DATs included in Audit" disabled=${off} onClick=${() => openDats(g)}>${Icon.stack(14)} DATs…</button>` : null}</div>
      <ul class="ls-list dat-list">${g.dats.map((d) => html`<li key=${d.id} data-dat=${d.id} data-name=${d.name}>
        <span class="dat-name"><span>${d.name}</span> <span class="muted">${d.version} · ${plural(d.games, "game", "games")}${d.author ? ` · ${d.author}` : ""}${d.libretro ? " · libretro's copy" : ""}</span>
          ${d.current && g.dats.filter((x) => x.name === d.name).length > 1 ? html`<span class="badge" title="The version the audit uses; the others are kept">Version used</span>` : null}
          ${several && d.main ? html`<span class="badge dat-main" title="Audit's totals are this DAT's counts">Main DAT</span>` : null}
          ${several && d.also ? html`<span class="badge dat-also" title="Its games are audited too, and are in the Library beside the main DAT's">Included in Audit</span>` : null}</span>
        ${d.current ? null : html`<button type="button" class="ghost" data-action="use-dat" disabled=${off} onClick=${() => useDat(d)}>Use this version</button>`}
        ${several && d.current && !d.main && d.version !== "custom" ? html`<button type="button" class="ghost" data-action="main-dat" disabled=${off} onClick=${() => makeMain(d, also)}>Make it the main DAT</button>` : null}
        ${isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => reveal(`${lib.path}/${d.path}`)}>${Icon.folder(14)}</button>` : null}
        <button type="button" class="ghost" data-action="remove-dat" disabled=${off} onClick=${() => confirmDialog({
          title: "Remove this DAT?", text: `${d.name} (${d.version}) and its copy in the library are removed, with what was matched to it. Your ROMs aren't touched; you can import it again.`,
          button: "Remove", run: () => removeDat(d) })}>Remove…</button>
      </li>`)}</ul></div>`;
    }) : html`<p id="dats-none">No DATs yet.</p>`}
    <div class="home-actions">
      ${isDesktop() ? html`<button type="button" class="ghost" id="import-dats" disabled=${off} onClick=${() => importDats()}>Import DATs…</button>` : null}
      <button type="button" class="ghost" id="starter-dats" disabled=${off} title="No-Intro's and Redump's DATs, from libretro's public copy" onClick=${() => starterDats()}>Get starter DATs…</button>
      <button type="button" class="ghost" id="redump-dats" disabled=${off} title="Redump's own DATs for the disc systems, from redump.org: every track of each disc" onClick=${() => redumpDialog()}>Get Redump's DATs…</button>
      <button type="button" class="ghost" id="dats-add-system" disabled=${off} onClick=${() => addSystemDialog()}>Add a system…</button>
      <button type="button" class="ghost" id="make-dat" disabled=${off} onClick=${() => ui.set({ dialog: { type: "make-dat" } })}>Make a DAT from a folder…</button>
    </div>
  </section>`;
}

/** How each platform's games are named when they're organised (kept in the library). */
function NamingSection({ lib }) {
  const dats = useStore(ui, (st) => st.dats) || [];
  const platforms = [...new Map(dats.map((d) => [d.platform, d.platform_name])).entries()].sort((a, b) => a[1].localeCompare(b[1]));
  const [platform, setPlatform] = useState(null);
  const [n, setN] = useState(null);
  const [template, setTemplate] = useState("");
  const [example, setExample] = useState(null);
  const [error, setError] = useState("");
  const p = platform || platforms[0]?.[0];
  useEffect(() => {
    if (!p) return;
    api("naming", { platform: p }).then((r) => { setN(r); setTemplate(r.template); setExample(r); setError(""); }).catch((e) => setError(e.message || String(e)));
  }, [p, lib.path]);
  useEffect(() => {
    if (!n || template === n.template) return;
    const t = setTimeout(() => api("naming_example", { template }).then((r) => { setExample(r); setError(""); }).catch((e) => setError(e.message || String(e))), 250);
    return () => clearTimeout(t);
  }, [template]);
  const save = async (patch = {}) => {
    try {
      const next = { template, folder: n.folder, ...patch };
      await api("naming_set", { platform: p, ...next });
      setN({ ...n, ...next });
      toast(`Saved how ${platforms.find(([id]) => id === p)?.[1]} games are named.`);
    } catch (e) {
      setError(e.message || String(e));
    }
  };
  const off = !!lib.read_only;
  return html`<section class="ls-section" id="settings-naming">
    <h2>Naming</h2>
    <p class="muted">How organised games are named, for each system with a DAT. The DAT's own name (<code>{name}</code>) is what other collectors' tools expect; the fields below make shorter names. Existing files change only when you organise.</p>
    ${!platforms.length ? html`<p class="muted">Import a DAT first.</p>` : html`
      <label class="field-inline"><span>System</span>
        <select id="naming-platform" class="compact" value=${p} onChange=${(e) => setPlatform(e.target.value)}>
          ${platforms.map(([id, name]) => html`<option key=${id} value=${id}>${name}</option>`)}
        </select></label>
      ${n ? html`<form class="field-block" onSubmit=${(e) => { e.preventDefault(); save(); }}>
        <span class="field-label">Name</span>
        <div class="pick-row">
          <input type="text" id="naming-template" value=${template} disabled=${off} onInput=${(e) => setTemplate(e.target.value)} aria-label="Name template" />
          <button type="submit" class="ghost" id="naming-save" disabled=${off || template === n.template || !!error}>Save</button>
          ${template !== n.default ? html`<button type="button" class="ghost" disabled=${off} onClick=${() => { setTemplate(n.default); save({ template: n.default }); }}>Use the DAT's names</button>` : null}
        </div>
        <small class="muted">Fields: ${(example?.fields || n.fields).map((f, i) => html`${i ? ", " : ""}<code key=${f}>{${f}}</code>`)}. Brackets left empty by a field a game doesn't have are dropped.</small>
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : html`<ul class="naming-examples" id="naming-examples">${(example?.examples || []).map(([from, to]) => html`<li key=${from}><span class="muted">${from}</span> → <code>${to}</code></li>`)}</ul>`}
      </form>
      <label class="check-row"><input type="checkbox" id="naming-folder" checked=${n.folder} disabled=${off} onChange=${(e) => save({ folder: e.target.checked })} /> A folder for each game</label>` : null}`}
  </section>`;
}

/** The changes made to files (organising, setting aside), newest first: Undo, and the space set-aside files take. */
/** Metadata: which source each detail comes from (D46), and what the library's metadata
 *  takes, by system and kind (D47), worked out when asked. */
function MetadataSection({ lib }) {
  const [c, setC] = useState(null);
  const [sizes, setSizes] = useState(null);
  const [busy, setBusy] = useState(false);
  const off = !!lib.read_only;
  useEffect(() => { api("meta_details_from").then(setC).catch(() => setC(null)); }, [lib.path]);
  const choose = async (field, source) => {
    setBusy(true);
    try {
      const { job } = await api("meta_set_details_from", { field, source: source || null });
      setC(await api("meta_details_from"));
      const done = await followJob(job, "Making details again");
      if (done.error) throw new Error(done.error);
      ui.set((st) => ({ changes: (st.changes || 0) + 1 }));
      toast(`Made the details of ${plural(done.result.games, "game", "games")} again.`, 6000);
    } catch (e) {
      toast(`Couldn't change it: ${e.message || e}`, 10000);
    } finally {
      setBusy(false);
    }
  };
  const work = async () => {
    setSizes("busy");
    try { setSizes(await api("meta_sizes")); } catch (e) { setSizes({ error: e.message || String(e) }); }
  };
  const names = Object.fromEntries((sizes?.names || []).map((n) => [n.id, n.name]));
  // a column for each kind that takes any space, biggest first
  const kindsShown = sizes?.kinds ? Object.entries(sizes.kinds).filter(([, n]) => n > 0).sort((a, b) => b[1] - a[1]).map(([k]) => k) : [];
  return html`<section class="ls-section" id="settings-metadata">
    <h2>Details and pictures</h2>
    <p class="muted">Each source's answer is kept in the game's library folder, and each detail is taken from the first source in the usual order (the Scrape dialog's) that has it, unless you choose one here or on a game's page.</p>
    ${c ? html`<div class="prefer-grid" id="details-from">${c.fields.map((f) => html`<label class="field-inline" key=${f.id}><span>${f.name}</span>
      <select class="compact" data-field=${f.id} value=${c.from[f.id] || ""} disabled=${off || busy} onChange=${(e) => choose(f.id, e.target.value)}>
        <option value="">Usual order</option>
        ${c.sources.map((x) => html`<option key=${x.id} value=${x.id}>${x.name}</option>`)}
      </select></label>`)}</div>` : null}
    <h3>Space taken</h3>
    ${sizes && sizes !== "busy" && !sizes.error ? html`<p id="meta-size-total"><b>${size(sizes.total)}</b> in all.</p>
      ${sizes.systems.length ? html`<div class="table-scroll"><table class="size-table" id="meta-sizes">
        <thead><tr><th>System</th><th class="num">Games</th>${kindsShown.map((k) => html`<th class="num" key=${k}>${names[k] || k}</th>`)}<th class="num">In all</th></tr></thead>
        <tbody>${sizes.systems.map((x) => html`<tr key=${x.platform} data-platform=${x.platform}><th scope="row">${x.name}</th><td class="num">${x.games}</td>
          ${kindsShown.map((k) => html`<td class="num" key=${k}>${x.kinds[k] ? size(x.kinds[k]) : ""}</td>`)}<td class="num"><b>${size(x.total)}</b></td></tr>`)}</tbody>
      </table></div>` : null}`
      : sizes?.error ? html`<p class="warn-note" role="alert">${sizes.error}</p>` : null}
    <div class="home-actions"><button type="button" class="ghost" id="meta-sizes-work" disabled=${sizes === "busy"} onClick=${work}>${sizes === "busy" ? "Measuring…" : sizes ? "Measure again" : "Measure the space taken"}</button></div>
    <small class="muted">Details, each kind of picture and video, scraped manuals and your own documents, for each system. Delete metadata… on the Metadata page or a game's page frees it.</small>
  </section>`;
}

const STATE = { done: "Done", undone: "Undone", interrupted: "Interrupted", running: "Running" };

/** One change in a history: what it was, when, its state and buttons, and its steps
 *  (D78) or what it set aside (D168) opened under it. */
function ChangeRow({ c, busy, off, act, empty }) {
  const [open, setOpen] = useState("");
  const flip = (k) => setOpen(open === k ? "" : k);
  return html`<li key=${c.id} data-change=${c.id} data-state=${c.state} data-kind=${c.kind}>
    <span class="change-label"><span>${c.label}</span><span class="muted">${new Date(c.created).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} · ${plural(c.ops, "step", "steps")}${c.held ? ` · ${size(c.held)} set aside` : ""}</span></span>
    <span class=${c.state === "interrupted" ? "badge review" : "badge"} title=${c.running_on ? `Game Repo on ${c.running_on} is running it: it can be undone here once it's finished` : null}>${c.running_on ? `Running on ${c.running_on}` : STATE[c.state] || c.state}</span>
    ${c.ops ? html`<button type="button" class="ghost" data-action="steps" aria-expanded=${open === "steps" ? "true" : "false"} onClick=${() => flip("steps")}>${open === "steps" ? "Hide the steps" : "Steps"}</button>` : null}
    ${c.held && empty ? html`<button type="button" class="ghost" data-action="aside" aria-expanded=${open === "aside" ? "true" : "false"} title="What it set aside: look through it, and delete what you don't want" onClick=${() => flip("aside")}>${open === "aside" ? "Hide what's set aside" : "Set aside"}</button>` : null}
    ${c.state === "interrupted" ? html`<button type="button" class="ghost" data-action="finish" disabled=${busy || off} onClick=${() => act(() => finishChange(c.id).then(() => "Finished it."))}>Finish</button>` : null}
    ${c.can_undo ? html`<button type="button" class="ghost" data-action="undo" disabled=${busy || off} title=${c.state === "interrupted" ? "Put back what it had done, as it was before it started" : null} onClick=${() => act(() => undoChange(c.id))}>${c.state === "interrupted" ? "Put back" : "Undo"}</button>` : null}
    ${c.held && c.state !== "running" && empty ? html`<button type="button" class="ghost" data-action="empty" disabled=${busy || off} onClick=${() => empty(c)}>Empty…</button>` : null}
    ${open === "steps" ? html`<${ChangeSteps} id=${c.id} />` : null}
    ${open === "aside" ? html`<${AsideFiles} id=${c.id} label=${c.label} off=${off || busy || c.state === "running"} />` : null}
  </li>`;
}

/** The changes of `kinds` (all when left out), newest first, grouped by day: History's
 *  list, and Recent imports on the Import page (D78). `limit` keeps the newest few. */
export function ChangeHistory({ lib, id, kinds = null, limit = 0, none = "No changes yet.", children }) {
  const changes = useStore(ui, (st) => st.changes);
  const [list, setList] = useState(null);
  const [busy, setBusy] = useState(false);
  const load = () => api("change_history").then(setList).catch((e) => setList({ error: e.message || String(e) }));
  useEffect(() => { load(); }, [lib.path, changes]);
  const act = async (fn) => {
    setBusy(true);
    try {
      const said = await fn();
      if (typeof said === "string") toast(said, 8000);
    } catch (e) {
      toast(e.message || String(e), 10000);
    } finally {
      setBusy(false);
      load();
    }
  };
  const all = Array.isArray(list) ? list.filter((c) => !kinds || kinds.includes(c.kind)) : [];
  const items = limit ? all.slice(0, limit) : all;
  const held = all.reduce((t, c) => t + (c.held || 0), 0);
  const empty = kinds ? null : (c) => confirmDialog({
    title: c ? "Empty what this change set aside?" : "Empty everything set aside?",
    text: c ? `The ${size(c.held)} of files "${c.label}" set aside are deleted, and it can't be undone any more.` : `The ${size(held)} of files set aside by these changes are deleted, and they can't be undone any more.`,
    button: "Empty", run: async () => { const r = await api("change_empty", { id: c?.id || null }); toast(`Freed ${size(r.freed)}.`); load(); },
  });
  const off = !!lib.read_only;
  const sum = (cs) => `${plural(cs.length, "change", "changes")}${cs.some((c) => c.games) ? `, ${plural(cs.reduce((n, c) => n + (c.games || 0), 0), "game", "games")}` : ""}`;
  return html`
    ${list?.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${list && !items.length ? html`<p class="muted" id=${`${id}-none`}>${none}</p>` : null}
    ${items.length ? html`<${DayGroups} id=${id} items=${items} at=${(c) => c.created} sum=${sum}
      row=${(c) => html`<${ChangeRow} key=${c.id} c=${c} busy=${busy} off=${off} act=${act} empty=${empty} />`} />` : null}
    ${limit && all.length > limit ? html`<p class="muted">${plural(all.length - limit, "older one is", "older ones are")} in <a href=${routeHash("history")}>History</a>.</p>` : null}
    ${children ? children({ held, empty, busy, off }) : null}`;
}

/** Everything set aside (D168), change by change, newest first: each file picked to be
 *  deleted for good. */
function SetAside({ lib }) {
  const changes = useStore(ui, (st) => st.changes);
  const [list, setList] = useState(null);
  useEffect(() => {
    let gone = false;
    api("change_aside").then((r) => { if (!gone) setList(r); }).catch((e) => { if (!gone) setList({ error: e.message || String(e) }); });
    return () => { gone = true; };
  }, [lib.path, changes]);
  if (!list) return html`<p class="muted">Reading…</p>`;
  if (list.error) return html`<p class="warn-note" role="alert">${list.error}</p>`;
  if (!list.length) return html`<p class="muted" id="aside-none">Nothing is set aside.</p>`;
  const total = list.reduce((t, c) => t + c.files.reduce((n, f) => n + (f.size || 0), 0), 0);
  const off = !!lib.read_only;
  return html`<p id="aside-total"><b>${size(total)}</b> set aside by ${plural(list.length, "change", "changes")}. Pick what to delete for good; the rest stays, so Undo can still bring it back.</p>
    <div class="aside-changes" id="aside-changes">${list.map((c) => html`<section class="aside-change" key=${c.id} data-change=${c.id}>
      <h3>${c.label} <span class="muted">${new Date(c.created).toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })}</span></h3>
      <${AsideFiles} id=${c.id} label=${c.label} files=${c.files} off=${off || c.state === "running"} /></section>`)}</div>`;
}

export function HistorySection({ lib }) {
  const [tab, setTab] = useState("changes");
  return html`<section class="ls-section" id="settings-history">
    <h2>Changes to your files</h2>
    <p class="muted">The changes the app has made to your files, kept in the library (<code>_library/journal</code>), a day at a time. Steps lists what a change did. The newest can be undone. Files a change replaced are set aside in <code>.gamerepo-set-aside</code> at the top of their folder until you empty them, or delete them one by one.</p>
    <div class="seg audit-tabs history-tabs" role="tablist" id="history-tabs">${[["changes", "Changes"], ["aside", "Set aside"]].map(([k, label]) => html`<button type="button" role="tab" key=${k} data-tab=${k} aria-selected=${tab === k ? "true" : "false"} onClick=${() => setTab(k)}>${label}</button>`)}</div>
    ${tab === "aside" ? html`<${SetAside} lib=${lib} />` : html`<${ChangeHistory} lib=${lib} id="history">${({ held, empty, busy, off }) => (held ? html`<div class="home-actions"><button type="button" class="ghost" id="history-empty" disabled=${busy || off} onClick=${() => empty(null)}>Empty everything set aside (${size(held)})…</button></div>` : null)}<//>`}
  </section>`;
}

/** Each platform's preferred format (D12): what Convert… offers first. Kept in the library. */
function FormatsSection({ lib }) {
  const [list, setList] = useState(null);
  useEffect(() => { api("formats").then(setList).catch((e) => setList({ error: e.message || String(e) })); }, [lib.path]);
  const choose = async (p, format) => {
    try {
      setList(await api("format_set", { platform: p.platform, format }));
      toast(`${p.name} games convert to ${CONVERT_FORMATS[format]?.[0] || format} first.`);
    } catch (e) {
      toast(e.message || String(e), 8000);
    }
  };
  const items = Array.isArray(list) ? list.filter((p) => p.formats.length > 1) : [];
  return html`<section class="ls-section" id="settings-formats">
    <h2>Preferred formats</h2>
    <p class="muted">What Convert… offers first for each system. CD and DVD systems keep CHD, GameCube and Wii RVZ, and cartridges a ZIP for each game; the systems below have a choice.</p>
    ${list?.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${items.length ? html`<ul class="ls-list" id="formats">${items.map((p) => html`<li key=${p.platform} data-platform=${p.platform}>
      <span>${p.name}</span>
      <select class="compact" aria-label=${`${p.name}'s preferred format`} value=${p.preferred} disabled=${!!lib.read_only} onChange=${(e) => choose(p, e.target.value)}>
        ${p.formats.map((f) => html`<option key=${f} value=${f}>${CONVERT_FORMATS[f]?.[0] || f.toUpperCase()}</option>`)}
      </select></li>`)}</ul>` : null}
  </section>`;
}

/** The external tools (D14): where each was found, and a path to set for it. Kept on this computer. */
function ToolsSection() {
  const [r, setR] = useState(null);
  const [error, setError] = useState("");
  useEffect(() => { api("tools").then(setR).catch((e) => setError(e.message || String(e))); }, []);
  const set = async (id, path) => {
    setError("");
    try {
      setR(await api("tools_set", { id, path }));
    } catch (e) {
      setError(e.message || String(e));
    }
  };
  const pick = async (t) => {
    const p = await ctx.platform.library.pickFile(`Where is ${t.id}?`, ["exe"]);
    if (p) set(t.id, p);
  };
  return html`<section class="ls-section" id="settings-tools">
    <h2>Tools</h2>
    <p class="muted">Game Repo reads and writes CHD, CSO and ZSO itself. For RVZ it uses Dolphin's own tool, and for GOG installers innoextract, found in the app's <code>tools</code> folder${r?.app_dir ? html` (<code>${r.app_dir}</code>)` : null}, on the PATH, or where you set it here. Nothing is downloaded.</p>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${r ? html`<ul class="ls-list change-list" id="tools">${r.tools.map((t) => html`<li key=${t.id} data-tool=${t.id} data-found=${t.path ? "yes" : "no"}>
      <span class="change-label"><span><b>${t.id}</b> <span class="muted">${t.what}</span></span>
        <span class="muted tool-where">${t.path ? `${t.path}${t.version ? ` · ${t.version}` : ""}` : "Not found"}</span></span>
      ${isDesktop() ? html`<button type="button" class="ghost" data-action="choose" onClick=${() => pick(t)}>Choose…</button>` : null}
      ${t.set ? html`<button type="button" class="ghost" data-action="clear" onClick=${() => set(t.id, null)}>Clear</button>` : null}
    </li>`)}</ul>
    <p class="muted" id="tools-text">Magazines' scanned pages are read with Windows' own text recognition, or with Tesseract when it's found (on Linux and the Steam Deck, only Tesseract).${" "}
      <button type="button" class="linkish" id="settings-read-magazines" disabled=${!!ui.get().library?.read_only} onClick=${() => readMagazines()}>Read every magazine's text</button></p>
    ${!isDesktop() ? html`<form class="pick-row" id="tools-form" onSubmit=${(e) => { e.preventDefault(); const f = e.target; set(f.tool.value, f.path.value); }}>
      <select name="tool" class="compact" aria-label="Tool">${r.tools.map((t) => html`<option key=${t.id} value=${t.id}>${t.id}</option>`)}</select>
      <input type="text" name="path" id="tools-path" placeholder="Its path" aria-label="Its path" />
      <button type="submit" class="ghost" id="tools-set">Set</button></form>` : null}` : null}
  </section>`;
}

/** The export layouts: a link to their editor. */
function LayoutsSection() {
  return html`<section class="ls-section" id="settings-layouts">
    <h2>Export layouts</h2>
    <p class="muted">Where an export puts the games, gamelists, pictures, manuals and BIOS files for ES-DE, muOS, Knulli, ROCKNIX, RetroArch, Pegasus, LaunchBox, Playnite or a plain folder. Copy one and change it for a new firmware or frontend, seeing what it would write as you go.</p>
    <div class="home-actions"><a class="button ghost" id="settings-layouts-link" href=${routeHash("layouts")}>${Icon.layers(15)} Export layouts</a></div>
  </section>`;
}

/** The sections down the side: each one's heading, a click away; the one in view marked. */
function SectionIndex() {
  const [list, setList] = useState([]);
  const [here, setHere] = useState("");
  useEffect(() => {
    const root = document.getElementById("settings-sections");
    if (!root) return;
    const read = () => setList([...root.querySelectorAll(":scope > section.ls-section[id]")].map((x) => [x.id, x.querySelector("h2")?.textContent || x.id]));
    read();
    const mo = new MutationObserver(read);
    mo.observe(root, { childList: true });
    const pages = root.closest(".pages");
    const onScroll = () => {
      const top = pages.getBoundingClientRect().top + 40;
      let cur = "";
      for (const x of root.querySelectorAll(":scope > section.ls-section[id]")) if (x.getBoundingClientRect().top <= top) cur = x.id;
      if (pages.scrollTop + pages.clientHeight >= pages.scrollHeight - 4) cur = root.querySelector(":scope > section.ls-section[id]:last-of-type")?.id || cur;
      setHere(cur || root.querySelector(":scope > section.ls-section[id]")?.id || "");
    };
    onScroll();
    pages.addEventListener("scroll", onScroll, { passive: true });
    return () => { mo.disconnect(); pages.removeEventListener("scroll", onScroll); };
  }, []);
  return html`<nav class="settings-index" id="settings-index-nav" aria-label="Settings sections"><ul>${list.map(([id, name]) => html`<li key=${id}>
    <a href=${`#/settings`} data-section=${id} aria-current=${here === id ? "true" : null} onClick=${(e) => { e.preventDefault(); document.getElementById(id)?.scrollIntoView({ block: "start" }); }}>${name}</a></li>`)}</ul></nav>`;
}

export function Settings() {
  const s = useStore(ui, (st) => ({ library: st.library, recent: st.recent, skin: st.skin, theme: st.theme, index: st.index }));
  const lib = s.library;
  const others = (s.recent || []).filter((p) => p !== lib?.path);
  return html`<${CommandBar} id="settings-head" title="Settings" /><div class="pages settings-page"><div class="settings-layout"><${SectionIndex} /><div class="library-settings" id="settings-sections">
    <section class="ls-section" id="settings-library">
      <h2>Library</h2>
      ${lib ? html`
        <div class="field-block"><label class="field-label" for="library-name">Name</label>
          <${RenameForm} lib=${lib} key=${lib.path} /></div>
        <div class="field-block"><span class="field-label">Folder</span>
          <div class="pick-row"><span class="pick-path" id="library-path">${lib.path}</span>
            ${isDesktop() ? html`<button type="button" class="ghost" onClick=${showLibraryFolder}>Show in folder</button>
              <button type="button" class="ghost" id="open-library" title="Open a library, or make a new one in an empty folder" onClick=${() => openLibrary()}>Open another library…</button>` : null}</div>
          <small class="muted">The library's settings, each game's details and pictures, and your documents are kept in this folder, so you can move, copy or sync it and open it again here.</small>
        </div>
        <${IndexSettings} index=${s.index} />` : html`<p class="muted">No library is open.</p>`}
      ${!lib && isDesktop() ? html`<div><button type="button" class="ghost" id="open-library" title="Open a library, or make a new one in an empty folder" onClick=${() => openLibrary()}>Open another library…</button></div>` : null}
      ${others.length ? html`<h3>Libraries opened before</h3>
        <ul class="ls-list" id="recent-libraries">${others.map((p) => html`<li key=${p}><span>${p}</span>
          <button type="button" class="ghost" onClick=${() => openLibrary(p)}>Open</button></li>`)}</ul>` : null}
    </section>
    ${lib ? html`<${ScanSection} lib=${lib} key=${`s-${lib.path}`} /><${SystemsSection} lib=${lib} /><${NamingSection} lib=${lib} key=${`n-${lib.path}`} /><${FormatsSection} lib=${lib} /><${LayoutsSection} /><${PcSettings} lib=${lib} key=${`pc-${lib.path}`} /><${MetadataSection} lib=${lib} key=${`m-${lib.path}`} /><${ServePanel} call=${api} />` : null}
    <${SourcesSettings} />
    <${SteamSettings} />
    <${ToolsSection} />
    ${isDesktop() ? html`<section class="ls-section" id="settings-updates">
      <h2>Updates</h2>
      <${UpdatePanel} call=${api} settings=${true} />
    </section>` : null}
    <section class="ls-section" id="settings-appearance">
      <h2>Appearance</h2>
      <label class="field-block"><span>Skin</span>
        <select id="skin-select" class="compact" value=${s.skin} onChange=${(e) => setSkin(e.target.value)}>
          ${SKINS.map(([v, label]) => html`<option value=${v} key=${v}>${label}</option>`)}
        </select>
        <small class="muted" id="skin-about">${(SKINS.find(([v]) => v === s.skin) || SKINS[0])[2]}</small>
        <small class="muted">The app's look: its colours, lettering, shapes and icons. Most have light and dark colours, chosen by Theme below.</small></label>
      <label class="field-block"><span>Theme</span>
        <select id="theme-select" class="compact" value=${s.theme} onChange=${(e) => setTheme(e.target.value)}>
          ${THEMES.map(([v, label]) => html`<option value=${v} key=${v}>${label}</option>`)}
        </select></label>
    </section>
  </div></div></div>`;
}
