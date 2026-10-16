// Settings: the open library (name, folder, other libraries), its index on this
// computer, the folders with games and how they're scanned, the DAT files, the
// look, and the app's version with a link to About. Naming and storage formats,
// external tools and metadata sources come with the phases that use them
// (docs/PLAN.md, 3.7).
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, THEMES, SKINS, setTheme, setSkin } from "./state.js";
import { ctx } from "./context.js";
import { isDesktop, openLibrary, renameLibrary, showLibraryFolder, checkIndex, undoable, size, plural } from "./library.js";
import { addRoot, updateRoot, repointRoot, removeRoot, scanSettings, setScanSettings, importDats, starterDats, useDat, removeDat, refreshGames, reveal } from "./games.js";
import { confirmDialog } from "./dialogs.js";
import { CommandBar } from "./layout.js";
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
        ${r.found ? null : html`<span class="warn-text">Not found on this computer</span>`}</span>
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
        <small class="muted">More is faster on SSDs and network drives; 1 suits a single spinning disk.</small></label>` : null}
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
    ${lib ? html`<${FoldersSection} lib=${lib} key=${`f-${lib.path}`} /><${DatsSection} lib=${lib} />` : null}
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
    <section class="ls-section" id="settings-coming">
      <h2>Still to come</h2>
      <p class="muted">Each platform's naming convention and preferred storage format (ZIP, CHD, RVZ, CSO, 7z), what's done to your original files, folders for external tools such as chdman, and accounts for metadata sources each arrive with the phase that uses them.</p>
    </section>
    <section class="ls-section" id="settings-about">
      <h2>About</h2>
      <p>Game Repo ${ctx.platform?.info?.version || ""}</p>
      <p><a href="#/about" id="settings-about-link">Versions, what's changed, what's coming and licences</a></p>
    </section>
  </div></div>`;
}
