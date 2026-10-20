// Settings (D103): the library paired with, ES-DE's folders found (each can be changed, C3),
// the media ticked by default, downloads at once, the ES-DE custom collection games go in
// (C16), what the Library shows, the device's name and screen, the picture cache, the look
// (Game Repo's skins and themes) and the version. Each change is kept as it's made.
import { html, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { SKINS, THEMES } from "../ui/state.js";
import { call, info, pickFolder } from "./backend.js";
import { go, look, refreshLists, refreshSettings, refreshStatus, say, setLook, showDialog } from "./state.js";
import { ago, bytes, count, KINDS, message } from "./format.js";

function Folder({ id, label, value, fallback, setting, error }) {
  const [text, setText] = useState(null);
  const [busy, setBusy] = useState(false);
  const save = async (v) => {
    setBusy(true);
    try {
      await call("set_settings", { [setting]: v });
      setText(null);
      await refreshStatus();
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
    }
  };
  const choose = async () => {
    const p = await pickFolder(`Choose ${label}`).catch(() => null);
    if (p) save(p);
  };
  return html`<div class="field-block go-folder" id=${id}>
    <span>${label}</span>
    <div class="go-folder-row">
      <input value=${text ?? (value || "")} placeholder=${fallback || "Not found"} spellcheck="false" aria-label=${label}
        onInput=${(e) => setText(e.currentTarget.value)} onKeyDown=${(e) => { if (e.key === "Enter" && text != null) { e.preventDefault(); save(text.trim()); } }} />
      ${text != null ? html`<button type="button" class="ghost" disabled=${busy} onClick=${() => save(text.trim())}>Save</button>` : null}
      <button type="button" class="ghost" disabled=${busy} onClick=${choose}>Choose…</button>
    </div>
    ${error ? html`<small class="danger-text">${error}</small>` : null}
  </div>`;
}

export function SettingsPage() {
  const s = useStore(go, (x) => ({ settings: x.settings, status: x.status }));
  const [error, setError] = useState("");
  const [lk, setLk] = useState(look());
  const st = s.status;
  const v = s.settings;
  if (!v || !st) return html`<section class="go-page go-settings"><p class="muted">Opening Settings…</p></section>`;
  const set = async (patch) => {
    setError("");
    try {
      go.set({ settings: await call("set_settings", patch) });
      if ("show_every_system" in patch || "show_on_device" in patch) refreshLists();
    } catch (e) {
      setError(message(e));
      refreshSettings();
    }
  };
  const kinds = new Set(v.media);
  const flip = (k) => set({ media: KINDS.map(([x]) => x).filter((x) => x !== "manual" && (x === k ? !kinds.has(k) : kinds.has(x))) });
  const dev = st.device || {};
  const synced = st.catalogue?.synced;
  const changeLook = (patch) => { setLook(patch); setLk(look()); };
  return html`<section class="go-page go-settings" id="go-settings">
    <h1>Settings</h1>
    ${error ? html`<p class="form-error" role="alert" id="settings-error">${error}</p>` : null}

    <section class="go-set" id="set-library">
      <h2>Library</h2>
      ${st.paired ? html`<p><strong id="set-library-name">${v.library || "Library"}</strong> <span class="muted">at ${st.library?.address}</span></p>
        <p class="muted" id="set-library-kept">${count(st.catalogue?.games)} games kept on this device${synced ? `, read ${ago(synced)}` : ""}.
          ${st.sync?.running ? " Reading it now…" : st.sync?.error ? ` ${st.sync.error}` : ""}</p>
        <div class="go-set-do">
          <button type="button" class="ghost" id="set-sync" disabled=${st.sync?.running} onClick=${() => call("sync").then(refreshStatus).catch((e) => say(message(e)))}>Read the library now</button>
          <button type="button" class="ghost" id="set-pair-again" onClick=${() => showDialog({ type: "pair" })}>Pair with another library…</button>
          <button type="button" class="ghost danger-text" id="set-unpair" onClick=${() => showDialog({ type: "unpair", library: v.library })}>Unpair…</button>
        </div>` : html`<p class="muted">Not paired with a library.</p>`}
    </section>

    <section class="go-set" id="set-esde">
      <h2>ES-DE on this device</h2>
      ${dev.error ? html`<p class="warn-note" id="set-no-esde">${dev.error}</p>` : null}
      <${Folder} id="set-esde-folder" label="ES-DE's folder" value=${v.esde || dev.esde} setting="esde" fallback="ES-DE's folder, with settings/es_settings.xml in it" />
      <${Folder} id="set-roms-folder" label="ROMs folder" value=${v.roms || dev.roms} setting="roms" fallback="As ES-DE's settings say" />
      ${dev.media ? html`<p class="muted">Pictures and manuals go in ${dev.media}, where ES-DE looks for them.</p>` : null}
      ${v.esde || v.roms ? html`<div><button type="button" class="ghost" id="set-folders-found" onClick=${() => set({ esde: "", roms: "" }).then(refreshStatus)}>Use the folders ES-DE says</button></div>` : null}
    </section>

    <section class="go-set" id="set-downloads">
      <h2>Downloads</h2>
      <div class="field-block" id="set-kinds"><span>Pictures and manuals ticked when adding games</span>
        <div class="go-kinds">${KINDS.filter(([k]) => k !== "manual").map(([k, name]) => html`<label class="check-row" key=${k}>
          <input type="checkbox" name=${k} checked=${kinds.has(k)} onChange=${() => flip(k)} /><span>${name}</span></label>`)}
          <label class="check-row"><input type="checkbox" name="manual" checked=${v.manuals} onChange=${() => set({ manuals: !v.manuals })} /><span>Manual</span></label>
        </div>
        <small class="muted">Add to device… starts with these ticked, and remembers what you chose last time.</small>
      </div>
      <label class="field-block"><span>Files downloaded at once</span>
        <select id="set-at-once" class="compact" value=${String(v.at_once)} onChange=${(e) => set({ at_once: +e.currentTarget.value })}>
          ${[1, 2, 3, 4].map((n) => html`<option value=${String(n)}>${n}</option>`)}
        </select>
      </label>
      <label class="field-block"><span>Put the games added in an ES-DE custom collection</span>
        <select id="set-collection" class="compact" value=${v.collection} onChange=${(e) => set({ collection: e.currentTarget.value })}>
          <option value="none">No</option>
          <option value="one">Yes, one for every game Game Repo Go adds</option>
          <option value="came-from">Yes, the collection they were added from</option>
        </select>
      </label>
      ${v.collection === "one" ? html`<label class="field-block"><span>Its name</span>
        <input id="set-collection-name" value=${v.collection_name} onChange=${(e) => set({ collection_name: e.currentTarget.value.trim() || "Game Repo Go" })} /></label>` : null}
    </section>

    <section class="go-set" id="set-showing">
      <h2>The Library</h2>
      <label class="check-row"><input type="checkbox" id="set-every-system" checked=${v.show_every_system} onChange=${() => set({ show_every_system: !v.show_every_system })} />
        <span>Show every system, not only those ES-DE has a folder for on this device</span></label>
      <label class="check-row"><input type="checkbox" id="set-on-device" checked=${v.show_on_device} onChange=${() => set({ show_on_device: !v.show_on_device })} />
        <span>Show games already on this device</span></label>
    </section>

    <section class="go-set" id="set-device">
      <h2>This device</h2>
      <label class="field-block"><span>Its name, as Game Repo shows it</span>
        <input id="set-name" value=${v.name} onChange=${(e) => set({ name: e.currentTarget.value.trim() })} />
        <small class="muted">Game Repo learns a new name when this device pairs again.</small></label>
      <label class="field-block"><span>Pictures made for a screen of</span>
        <select id="set-screen" class="compact" value=${String(v.screen)} onChange=${(e) => set({ screen: +e.currentTarget.value })}>
          ${[[640, "640 pixels (small handhelds)"], [960, "960 pixels"], [1280, "1280 pixels (most handhelds)"], [1920, "1920 pixels (full HD)"], [2560, "2560 pixels"], [3840, "3840 pixels (4K)"]].map(([n, t]) => html`<option value=${String(n)}>${t}</option>`)}
        </select></label>
      <label class="field-block"><span>Box fronts kept for browsing</span>
        <select id="set-pictures" class="compact" value=${String(v.pictures_mb)} onChange=${(e) => set({ pictures_mb: +e.currentTarget.value })}>
          ${[100, 250, 500, 1000, 2000, 5000].map((n) => html`<option value=${String(n)}>Up to ${bytes(n * 1024 * 1024)}</option>`)}
        </select>
        <small class="muted" id="set-pictures-kept">${count(st.pictures?.kept)} kept, ${bytes(st.pictures?.size)}. The least recently shown go first.</small></label>
    </section>

    <section class="go-set" id="set-look">
      <h2>Look</h2>
      <label class="field-block"><span>Skin</span>
        <select id="set-skin" class="compact" value=${lk.skin} onChange=${(e) => changeLook({ skin: e.currentTarget.value })}>
          ${SKINS.map(([k, name]) => html`<option value=${k}>${name}</option>`)}</select></label>
      <label class="field-block"><span>Theme</span>
        <select id="set-theme" class="compact" value=${lk.theme} onChange=${(e) => changeLook({ theme: e.currentTarget.value })}>
          ${THEMES.map(([k, name]) => html`<option value=${k}>${name}</option>`)}</select></label>
    </section>

    <section class="go-set" id="set-about">
      <h2>About</h2>
      <p>Game Repo Go <span id="set-version">${st.version}</span>${info.kind ? ` for ${info.kind[0].toUpperCase()}${info.kind.slice(1)}` : ""}. Free software under the MIT licence.</p>
    </section>
  </section>`;
}
