// Settings: the open library (name, folder, other libraries), its index on this
// computer, the look, and the app's version with a link to About. Platforms and
// their naming and storage formats, external tools and metadata sources come
// with the phases that use them (docs/PLAN.md, 3.7).
import { html, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, THEMES, SKINS, setTheme, setSkin } from "./state.js";
import { ctx } from "./context.js";
import { isDesktop, openLibrary, renameLibrary, showLibraryFolder, checkIndex, undoable, size } from "./library.js";
import { CommandBar } from "./layout.js";

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
      <p class="muted">Platforms with their naming convention and preferred storage format (ZIP, CHD, RVZ, CSO, 7z), what's done to your original files, folders for external tools such as chdman, and accounts for metadata sources each arrive with the phase that uses them.</p>
    </section>
    <section class="ls-section" id="settings-about">
      <h2>About</h2>
      <p>Game Repo ${ctx.platform?.info?.version || ""}</p>
      <p><a href="#/about" id="settings-about-link">Versions, what's changed, what's coming and licences</a></p>
    </section>
  </div></div>`;
}
