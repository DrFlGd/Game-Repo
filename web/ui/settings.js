// Settings: the open library (name, folder, other libraries), its index on this
// computer, the folders with games and how they're scanned and watched, the DAT
// files, the systems (built-in and added), how games are named, the history of changes (Undo, set-aside files), the
// look, and the app's version with a link to About. Storage formats and external tools
// (Phase 2b), PC games, Identify's sources and Steam (Phase 4), and the sources' accounts and keys
// (Phase 5) have sections of their own.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, THEMES, SKINS, setTheme, setSkin } from "./state.js";
import { ctx } from "./context.js";
import { api, toast, isDesktop, openLibrary, renameLibrary, showLibraryFolder, checkIndex, undoable, size, plural } from "./library.js";
import { undoChange, finishChange, CONVERT_FORMATS } from "./organise.js";
import { addRoot, updateRoot, repointRoot, removeRoot, scanSettings, setScanSettings, importDats, starterDats, useDat, removeDat, refreshGames, reveal } from "./games.js";
import { confirmDialog } from "./dialogs.js";
import { SystemsSection } from "./systems.js";
import { CommandBar } from "./layout.js";
import { PcSettings, SourcesSettings, SteamSettings } from "./pcgames.js";
import { Icon } from "./icons.js";

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
  </div>`;
}

/** Folders with games (kept in the library), and how scans read them (kept on this computer). */
function FoldersSection({ lib }) {
  const s = useStore(ui, (st) => ({ roots: st.roots, platforms: st.platforms }));
  const [scanOpts, setScanOpts] = useState(null);
  useEffect(() => { refreshGames(); scanSettings().then(setScanOpts).catch(() => {}); }, [lib.path]);
  const roots = s.roots || [];
  const change = async (patch) => setScanOpts(await setScanSettings(patch));
  const off = !!lib.read_only;
  return html`<section class="ls-section" id="settings-folders">
    <h2>Folders with games</h2>
    <p class="muted">The folders your ROMs are in, anywhere on this computer or a network drive. Scanning reads them; nothing in them is changed. The list is kept in the library; a folder that's somewhere else on another computer can be pointed there.</p>
    ${roots.length ? html`<ul class="ls-list root-list" id="roots">${roots.map((r) => html`<li key=${r.id} data-root=${r.id}>
      <span class="root-path"><span title=${r.dir}>${r.dir}</span>
        ${r.found ? null : html`<span class="warn-text">Not found on this computer</span>`}
        ${r.network ? html`<span class="muted root-note">A network drive: changes there are read when you scan.</span>` : null}</span>
      <select class="compact" aria-label="Platform of the files no DAT knows" title="The platform of files in it that no DAT recognises (matched files take their DAT's)" value=${r.platform || ""} disabled=${off}
        onChange=${(e) => updateRoot(r.id, { platform: e.target.value || null })}>
        <option value="">Platform: worked out</option>
        ${(s.platforms || []).map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      </select>
      ${isDesktop() && !r.found ? html`<button type="button" class="ghost" disabled=${off} onClick=${() => repointRoot(r)}>Point it elsewhere…</button>` : null}
      ${isDesktop() && r.found ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => ctx.platform.library.openPath(r.dir)}>${Icon.folder(14)}</button>` : null}
      <button type="button" class="ghost" data-action="remove-root" disabled=${off} onClick=${() => confirmDialog({
        title: "Stop scanning this folder?", text: `${r.dir} leaves the list, with what scans found in it. Nothing in the folder is changed or deleted.`,
        button: "Remove", run: () => removeRoot(r) })}>Remove</button>
    </li>`)}</ul>` : html`<p id="roots-none">No folders yet.</p>`}
    ${isDesktop() ? html`<div><button type="button" class="ghost" id="add-root" disabled=${off} onClick=${() => addRoot()}>Add a folder…</button></div>` : null}
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

/** The DAT files imported into the library, by platform and version. */
function DatsSection({ lib }) {
  const dats = useStore(ui, (st) => st.dats) || [];
  const off = !!lib.read_only;
  const byPlatform = [];
  for (const d of dats) {
    let g = byPlatform.find((x) => x.platform === d.platform);
    if (!g) byPlatform.push((g = { platform: d.platform, name: d.platform_name, dats: [] }));
    g.dats.push(d);
  }
  byPlatform.sort((a, b) => a.name.localeCompare(b.name));
  return html`<section class="ls-section" id="settings-dats">
    <h2>DAT files</h2>
    <p class="muted">The DATs your sets are audited against. Each is copied into the library (<code>_library/dats</code>); importing a newer version keeps the older one, and the newest is used unless you choose another.</p>
    ${byPlatform.length ? byPlatform.map((g) => html`<div class="dat-group" key=${g.platform}><h3>${g.name}</h3>
      <ul class="ls-list dat-list">${g.dats.map((d) => html`<li key=${d.id} data-dat=${d.id}>
        <span class="dat-name"><span>${d.name}</span> <span class="muted">${d.version} · ${plural(d.games, "game", "games")}${d.author ? ` · ${d.author}` : ""}</span>
          ${d.current ? html`<span class="badge">In use</span>` : null}</span>
        ${d.current ? null : html`<button type="button" class="ghost" data-action="use-dat" disabled=${off} onClick=${() => useDat(d)}>Use this version</button>`}
        ${isDesktop() ? html`<button type="button" class="ghost compact" title="Show in folder" aria-label="Show in folder" onClick=${() => reveal(`${lib.path}/${d.path}`)}>${Icon.folder(14)}</button>` : null}
        <button type="button" class="ghost" data-action="remove-dat" disabled=${off} onClick=${() => confirmDialog({
          title: "Remove this DAT?", text: `${d.name} (${d.version}) and its copy in the library are removed, with what was matched to it. Your ROMs aren't touched; you can import it again.`,
          button: "Remove", run: () => removeDat(d) })}>Remove</button>
      </li>`)}</ul></div>`) : html`<p id="dats-none">No DATs yet.</p>`}
    <div class="home-actions">
      ${isDesktop() ? html`<button type="button" class="ghost" id="import-dats" disabled=${off} onClick=${() => importDats()}>Import DATs…</button>` : null}
      <button type="button" class="ghost" id="starter-dats" disabled=${off} title="The No-Intro cartridge DATs, from libretro's public copy" onClick=${starterDats}>Get starter DATs…</button>
      <button type="button" class="ghost" id="make-dat" disabled=${off} onClick=${() => ui.set({ dialog: { type: "make-dat" } })}>Make a DAT from a set…</button>
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
    <p class="muted">How organised games are named, for each platform with a DAT. The DAT's own name (<code>{name}</code>) is what other collectors' tools expect; the fields below make shorter names. Existing files change only when you organise.</p>
    ${!platforms.length ? html`<p class="muted">Import a DAT first.</p>` : html`
      <label class="field-inline"><span>Platform</span>
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
        <small class="muted">Fields: ${(example?.fields || n.fields).map((f) => html`<code key=${f}>{${f}}</code> `)}. Brackets left empty by a field a game doesn't have are dropped.</small>
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : html`<ul class="naming-examples" id="naming-examples">${(example?.examples || []).map(([from, to]) => html`<li key=${from}><span class="muted">${from}</span> → <code>${to}</code></li>`)}</ul>`}
      </form>
      <label class="check-row"><input type="checkbox" id="naming-folder" checked=${n.folder} disabled=${off} onChange=${(e) => save({ folder: e.target.checked })} /> A folder for each game</label>` : null}`}
  </section>`;
}

/** The changes made to files (organising, setting aside), newest first: Undo, and the space set-aside files take. */
function HistorySection({ lib }) {
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
  const empty = (c) => confirmDialog({
    title: c ? "Empty what this change set aside?" : "Empty everything set aside?",
    text: c ? `The ${size(c.held)} of files "${c.label}" set aside are deleted, and it can't be undone any more.` : `The ${size(held)} of files set aside by these changes are deleted, and they can't be undone any more.`,
    button: "Empty", run: async () => { const r = await api("change_empty", { id: c?.id || null }); toast(`Freed ${size(r.freed)}.`); load(); },
  });
  const items = Array.isArray(list) ? list : [];
  const held = items.reduce((t, c) => t + (c.held || 0), 0);
  const state = { done: "Done", undone: "Undone", interrupted: "Interrupted", running: "Running" };
  const off = !!lib.read_only;
  return html`<section class="ls-section" id="settings-history">
    <h2>History</h2>
    <p class="muted">The changes the app has made to your files, kept in the library (<code>_library/journal</code>). The newest can be undone. Files a change replaced are set aside in <code>.gamerepo-set-aside</code> at the top of their folder until you empty them.</p>
    ${list?.error ? html`<p class="warn-note" role="alert">${list.error}</p>` : null}
    ${list && !items.length ? html`<p id="history-none">No changes yet.</p>` : null}
    ${items.length ? html`<ul class="ls-list change-list" id="history">${items.map((c) => html`<li key=${c.id} data-change=${c.id} data-state=${c.state}>
      <span class="change-label"><span>${c.label}</span><span class="muted">${c.created?.replace("T", " ").slice(0, 16)} · ${plural(c.ops, "step", "steps")}${c.held ? ` · ${size(c.held)} set aside` : ""}</span></span>
      <span class=${c.state === "interrupted" ? "badge review" : "badge"}>${state[c.state] || c.state}</span>
      ${c.state === "interrupted" ? html`<button type="button" class="ghost" data-action="finish" disabled=${busy || off} onClick=${() => act(() => finishChange(c.id).then(() => "Finished it."))}>Finish</button>` : null}
      ${c.can_undo ? html`<button type="button" class="ghost" data-action="undo" disabled=${busy || off} onClick=${() => act(() => undoChange(c.id))}>${c.state === "interrupted" ? "Put back" : "Undo"}</button>` : null}
      ${c.held && c.state !== "running" ? html`<button type="button" class="ghost" data-action="empty" disabled=${busy || off} onClick=${() => empty(c)}>Empty</button>` : null}
    </li>`)}</ul>` : null}
    ${held ? html`<div class="home-actions"><button type="button" class="ghost" id="history-empty" disabled=${busy || off} onClick=${() => empty(null)}>Empty everything set aside (${size(held)})</button></div>` : null}
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
    <p class="muted">What Convert… offers first for each platform. CD and DVD systems keep CHD, GameCube and Wii RVZ, and cartridges a ZIP for each game; the platforms below have a choice.</p>
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
    ${!isDesktop() ? html`<form class="pick-row" id="tools-form" onSubmit=${(e) => { e.preventDefault(); const f = e.target; set(f.tool.value, f.path.value); }}>
      <select name="tool" class="compact" aria-label="Tool">${r.tools.map((t) => html`<option key=${t.id} value=${t.id}>${t.id}</option>`)}</select>
      <input type="text" name="path" id="tools-path" placeholder="Its path" aria-label="Its path" />
      <button type="submit" class="ghost" id="tools-set">Set</button></form>` : null}` : null}
  </section>`;
}

export function Settings() {
  const s = useStore(ui, (st) => ({ library: st.library, recent: st.recent, skin: st.skin, theme: st.theme, index: st.index }));
  const lib = s.library;
  const others = (s.recent || []).filter((p) => p !== lib?.path);
  return html`<${CommandBar} id="settings-head" title="Settings" /><div class="pages"><div class="library-settings">
    <section class="ls-section" id="settings-library">
      <h2>Library</h2>
      ${lib ? html`
        <div class="field-block"><label class="field-label" for="library-name">Name</label>
          <${RenameForm} lib=${lib} key=${lib.path} /></div>
        <div class="field-block"><span class="field-label">Folder</span>
          <div class="pick-row"><span class="pick-path" id="library-path">${lib.path}</span>
            ${isDesktop() ? html`<button type="button" class="ghost" onClick=${showLibraryFolder}>Show in folder</button>
              <button type="button" class="ghost" id="open-library" title="Open a library, or make a new one in an empty folder" onClick=${() => openLibrary()}>Open another library…</button>` : null}</div>
          <small class="muted">The library's settings, each game's details, scraped media and your documents are kept in this folder, so you can move, copy or sync it and open it again here.</small>
        </div>
        <${IndexSettings} index=${s.index} />` : html`<p class="muted">No library is open.</p>`}
      ${!lib && isDesktop() ? html`<div><button type="button" class="ghost" id="open-library" title="Open a library, or make a new one in an empty folder" onClick=${() => openLibrary()}>Open another library…</button></div>` : null}
      ${others.length ? html`<h3>Libraries opened before</h3>
        <ul class="ls-list" id="recent-libraries">${others.map((p) => html`<li key=${p}><span>${p}</span>
          <button type="button" class="ghost" onClick=${() => openLibrary(p)}>Open</button></li>`)}</ul>` : null}
    </section>
    ${lib ? html`<${FoldersSection} lib=${lib} key=${`f-${lib.path}`} /><${DatsSection} lib=${lib} /><${SystemsSection} lib=${lib} /><${NamingSection} lib=${lib} key=${`n-${lib.path}`} /><${FormatsSection} lib=${lib} /><${PcSettings} lib=${lib} key=${`pc-${lib.path}`} /><${HistorySection} lib=${lib} />` : null}
    <${SourcesSettings} />
    <${SteamSettings} />
    <${ToolsSection} />
    <section class="ls-section">
      <h2>Appearance</h2>
      <label class="field-block"><span>Skin</span>
        <select id="skin-select" class="compact" value=${s.skin} onChange=${(e) => setSkin(e.target.value)}>
          ${SKINS.map(([v, label]) => html`<option value=${v} key=${v}>${label}</option>`)}
        </select>
        <small class="muted">The app's look: its colours, lettering, shapes and icons. Each one has light and dark colours, chosen by Theme below.</small></label>
      <label class="field-block"><span>Theme</span>
        <select id="theme-select" class="compact" value=${s.theme} onChange=${(e) => setTheme(e.target.value)}>
          ${THEMES.map(([v, label]) => html`<option value=${v} key=${v}>${label}</option>`)}
        </select></label>
    </section>
    <section class="ls-section" id="settings-about">
      <h2>About</h2>
      <p>Game Repo ${ctx.platform?.info?.version || ""}</p>
      <p><a href="#/about" id="settings-about-link">Versions, what's changed, what's coming and licences</a></p>
    </section>
  </div></div>`;
}
