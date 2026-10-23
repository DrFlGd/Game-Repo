// Add a system (D166): a system the library has no DAT for yet, or a new one, and how its
// list of games comes: downloaded (libretro's copy of No-Intro's or Redump's DAT, or
// Redump's own for a disc system, D167), from a DAT file you have, or made from a folder
// of its games. Get Redump's DATs… (D167) is here too: Redump's own DATs for the disc
// systems, for the full check libretro's copies (one track of each disc) can't give.
import { html, useState, useRef, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { api, toast, isDesktop, plural } from "./library.js";
import { Dialog, More, close } from "./dialogs.js";
import { importDats, makeDat, runStarter, runRedump, runUpdateDats, scan, refreshGames } from "./games.js";
import { saveSystem, idFromName, MEDIA } from "./systems.js";
import { argHash } from "./panels.js";

/** Open Add a system, with `platform` (an id, or `NEW` for one the app doesn't know)
 *  chosen, else the first without a DAT. */
export const addSystemDialog = (platform = null) => ui.set({ dialog: { type: "add-system", platform } });

/** A system Game Repo doesn't know, as the dialog's choice. */
export const NEW = "__new";

/** Open Get Redump's DATs, with `platforms` ticked, else the disc systems that have a DAT. */
export const redumpDialog = (platforms = null) => ui.set({ dialog: { type: "redump", platforms } });

/** Open Update DATs (D172): the newest of every system's DATs. */
export const updateDatsDialog = () => ui.set({ dialog: { type: "dats-update" } });

const byName = (a, b) => a.name.localeCompare(b.name);

/** After its DAT came: the system's audit, when it has one now. */
function show(id) {
  if (!(ui.get().dats || []).some((d) => d.platform === id)) return;
  const prefs = ctx.platform.store.prefs;
  prefs.set("gr-audit", { ...(prefs.get("gr-audit", {}) || {}), platform: id });
  location.hash = argHash("audit", id);
}

function AddSystemDialog({ platform: preset }) {
  const list = (useStore(ui, (s) => s.platforms) || []).filter((p) => p.id !== "bios");
  const dats = useStore(ui, (s) => s.dats) || [];
  const roots = useStore(ui, (s) => s.roots) || [];
  const has = (id) => dats.some((d) => d.platform === id);
  const without = list.filter((p) => !has(p.id)).sort(byName);
  const withDat = list.filter((p) => has(p.id)).sort(byName);
  const [pick, setPick] = useState(preset || without[0]?.id || NEW);
  const [newName, setNewName] = useState("");
  const [newId, setNewId] = useState("");
  const [media, setMedia] = useState("cartridge");
  const [more, setMore] = useState({ dat_names: "", folders: "", exts: "" });
  const sys = list.find((p) => p.id === pick);
  const isNew = pick === NEW;
  const builtIn = !!sys && !sys.custom;
  const [way, setWay] = useState(isNew ? "later" : builtIn || !isDesktop() ? "download" : "file");
  const [source, setSource] = useState("libretro");
  const name = isNew ? newName.trim() : sys?.name || "";
  const [folder, setFolder] = useState(() => roots.find((r) => r.platform === pick)?.dir || "");
  const [datName, setDatName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  // a new system added already, when what came after it failed and it's tried again
  const added = useRef(null);
  const id = isNew ? newId || idFromName(newName) : pick;
  const choose = (v) => {
    setPick(v);
    setError("");
    const p = list.find((x) => x.id === v);
    if (way === "download" && (!p || p.custom)) setWay(isDesktop() ? "file" : "folder");
    if (way === "later" && v !== NEW) setWay(p && !p.custom ? "download" : isDesktop() ? "file" : "folder");
    if (source === "redump" && !p?.redump) setSource("libretro");
    const r = roots.find((x) => x.platform === v);
    if (r) setFolder(r.dir);
  };
  const shownDatName = datName || (name ? `My ${name} games` : "");
  const ways = [
    ["download", "Download its DAT", builtIn ? (sys.redump ? "libretro's copy of its DAT, as Get starter DATs gets, or Redump's own." : "libretro's copy of its DAT, as Get starter DATs gets.") : "Only for the systems Game Repo knows.", !builtIn],
    ["file", "From a DAT file", isDesktop() ? "A .dat, .xml or .zip you have: from No-Intro, Redump, TOSEC or anywhere else." : "In the Game Repo app.", !isDesktop()],
    ["folder", "From a folder of its games", "Each file in the folder becomes a game in a DAT made for it: what you have, as you have it.", false],
    // (a system the app doesn't know can wait for its DAT: Import DATs… then asks which system)
    ...(isNew ? [["later", "Nothing yet", "Add it now, and import or make its DAT later.", false]] : []),
  ];
  const ready = !!name && !!id && (way === "later" ? isNew : way === "download" ? builtIn : way === "file" ? isDesktop() : !!folder.trim() && !!shownDatName.trim());
  const pickFolder = async () => {
    const f = await ctx.platform.library.pickFolder(`The folder with ${name || "the system"}'s games`);
    if (f) setFolder(f);
  };
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      // a DAT file is chosen first: nothing's added if none is
      const files = way === "file" ? await ctx.platform.library.pickFiles(`DAT files for ${name} (.dat, .xml or .zip)`) : null;
      if (way === "file" && !files?.length) { setBusy(false); return; }
      let system = id;
      if (isNew && added.current !== id) {
        const saved = await saveSystem({ id, name, ...more, media });
        system = added.current = saved.id;
        if (way === "later") {
          close();
          toast(`Added ${saved.name}: DATs named ${saved.dat_names[0] || "after it"} are found for it.`, 6000);
          return;
        }
      }
      if (way === "download") {
        close();
        const r = source === "redump" ? await runRedump([system]) : await runStarter([system], true);
        if (r?.imported?.length) show(system);
        return;
      }
      if (way === "file") {
        close();
        // every DAT picked is for this system, whatever its name says
        const r = await importDats(files, { "*": system });
        if (r?.imported?.length) show(system);
        return;
      }
      // the folder with games it's in, or a new one for this system
      let at = await api("roots_locate", { path: folder.trim() });
      if (!at) {
        const root = await api("roots_add", { path: folder.trim(), platform: system });
        await refreshGames();
        at = { root, folder: "" };
      }
      close();
      const where = at.folder ? `${at.root.dir.replace(/[\\/]+$/, "")}/${at.folder}` : at.root.dir;
      const scanned = await scan(null, [where]);
      if (!scanned || scanned.stopped) return;
      await makeDat({ root: at.root.id, folder: at.folder, name: shownDatName.trim(), platform: system });
      show(system);
    } catch (e) {
      // (before it closed: what's after shows its own message)
      if (ui.get().dialog?.type === "add-system") { setError(e.message || String(e)); setBusy(false); }
      else toast(`Couldn't add ${name}: ${e.message || e}`, 8000);
    }
  };
  return html`<${Dialog} title="Add a system" id="add-system-dialog" onSubmit=${submit} busy=${busy || !ready} error=${error}
    submitLabel=${way === "download" ? "Download" : way === "file" ? "Choose the DAT…" : way === "folder" ? "Make its DAT" : "Add"}>
    <p class="muted">Choose the system, then where its list of games comes from: Audit checks your files against it.</p>
    <label class="field-block"><span class="field-label">System</span>
      <select id="add-system-pick" class="compact" value=${pick} onChange=${(e) => choose(e.target.value)}>
        ${without.length ? html`<optgroup label="No DAT yet">${without.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}${p.custom ? " (added)" : ""}</option>`)}</optgroup>` : null}
        ${withDat.length ? html`<optgroup label="With a DAT: add another">${withDat.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}${p.custom ? " (added)" : ""}</option>`)}</optgroup>` : null}
        <option value=${NEW}>A system Game Repo doesn't know…</option>
      </select></label>
    ${isNew ? html`<div class="field-block" id="add-system-new"><span class="field-label">The new system</span>
      <div class="pick-row">
        <input type="text" id="add-system-name" maxlength="80" placeholder="Its name, e.g. Xbox One" aria-label="The new system's name" value=${newName} onInput=${(e) => setNewName(e.target.value)} />
        <input type="text" id="add-system-id" class="system-id" maxlength="40" placeholder="Folder name" aria-label="The new system's folder name (for exports)" title="Its folder name: the folder exports use" value=${id} onInput=${(e) => setNewId(e.target.value.trim().toLowerCase())} />
      </div>
      <select id="add-system-media" class="compact" aria-label="Its games are on" value=${media} onChange=${(e) => setMedia(e.target.value)}>
        ${MEDIA.map(([v, label]) => html`<option key=${v} value=${v}>${label}</option>`)}
      </select>
      <details class="more-about" id="add-system-details"><summary>Its DAT names, folders and file extensions</summary>
        ${[["dat_names", "DAT names", "e.g. Microsoft - Xbox One", "The names in its DATs' headers, separated by commas: a DAT whose name starts with one of them is found for it."],
          ["folders", "Folder names", "e.g. Xbox One, XB1", "Folders with these names hold its games."],
          ["exts", "File extensions", "e.g. xvc", ""]].map(([k, label, eg, about]) => html`<label class="field-block" key=${k}><span class="field-label">${label}</span>
          <input type="text" id=${`add-system-${k.replace("_", "-")}`} placeholder=${eg} value=${more[k]} onInput=${(e) => setMore((m) => ({ ...m, [k]: e.target.value }))} />
          ${about ? html`<small class="muted">${about}</small>` : null}</label>`)}
      </details></div>` : null}
    <div class="field-block way-pick" id="add-system-way" role="radiogroup" aria-label="Where its DAT comes from">${ways.map(([k, label, about, off]) => html`<label key=${k} class="way" data-way=${k} aria-disabled=${off ? "true" : null}>
      <input type="radio" name="add-system-way" value=${k} checked=${way === k} disabled=${off} onChange=${() => { setWay(k); setError(""); }} />
      <span><b>${label}</b><small class="muted">${about}</small></span></label>`)}</div>
    ${way === "download" && builtIn ? html`<label class="field-block"><span class="field-label">From</span>
      <select id="add-system-source" class="compact" value=${source} onChange=${(e) => setSource(e.target.value)}>
        <option value="libretro">libretro's copy (GitHub)</option>
        ${sys.redump ? html`<option value="redump">Redump (redump.org), every track of each disc</option>` : null}
      </select>
      <small class="muted">${source === "redump" ? "Redump's own DAT, used before libretro's copy." : `If libretro has none for ${sys.name}${sys.redump ? ", try Redump, or" : ","} make one from a folder of its games.`}</small></label>` : null}
    ${way === "folder" ? html`<div class="field-block"><span class="field-label">Folder</span>
      <div class="pick-row">
        <input type="text" id="add-system-folder" placeholder="The folder with its games" value=${folder} onInput=${(e) => setFolder(e.target.value)} />
        ${isDesktop() ? html`<button type="button" class="ghost" id="add-system-choose" onClick=${pickFolder}>Choose…</button>` : null}
      </div>
      <small class="muted">A folder that isn't in your folders with games yet is added as one, for this system, and scanned.</small></div>
      <label class="field-block"><span class="field-label">The DAT's name</span>
        <input type="text" id="add-system-dat-name" maxlength="150" value=${shownDatName} onInput=${(e) => setDatName(e.target.value)} /></label>` : null}
    <${More} id="add-system-more">
      <p>PlayStation 3, 4 and 5, Xbox 360, Switch and the other systems Game Repo knows are only listed on Home and Audit once they have a DAT.</p>
      <p>libretro keeps no DAT for some (Switch, Wii U, PlayStation 4 and 5): Redump has the disc systems', and a DAT made from a folder lists what you have. A made DAT is saved in the library's <code>_library/made-dats</code> folder.</p>
    <//>
  <//>`;
}

/** Which disc systems to get Redump's own DATs for. */
function RedumpDialog({ platforms: preset }) {
  const list = (useStore(ui, (s) => s.platforms) || []).filter((p) => p.redump).sort(byName);
  const dats = useStore(ui, (s) => s.dats) || [];
  const mine = (id) => dats.filter((d) => d.platform === id);
  const [on, setOn] = useState(() => new Set(preset || list.filter((p) => mine(p.id).length).map((p) => p.id)));
  const now = (id) => (mine(id).some((d) => !d.libretro) ? "Has Redump's or your own" : mine(id).length ? "Has libretro's copy" : "No DAT yet");
  const tick = (id, v) => setOn((s) => { const t = new Set(s); v ? t.add(id) : t.delete(id); return t; });
  return html`<${Dialog} title="Get Redump's DATs" id="redump-dialog" wide=${true} onSubmit=${() => { close(); runRedump(list.filter((p) => on.has(p.id)).map((p) => p.id)); }}
    busy=${!on.size} submitLabel=${on.size ? `Download ${on.size === 1 ? "it" : `${on.size} DATs`}` : "Download"}>
    <p>Redump's own DATs for the systems you tick are downloaded from redump.org and imported. They list every track of each disc, so they're used before libretro's copies.</p>
    <p class="muted"><button type="button" class="linkish" id="redump-all" onClick=${() => setOn(new Set(list.map((p) => p.id)))}>Tick them all</button> · <button type="button" class="linkish" id="redump-none" onClick=${() => setOn(new Set())}>None</button></p>
    <div class="tick-grid redump-systems" id="redump-systems">${list.map((p) => html`<label class="check-row long" key=${p.id} data-platform=${p.id}>
      <input type="checkbox" checked=${on.has(p.id)} onChange=${(e) => tick(p.id, e.target.checked)} />
      <span>${p.name}<br /><small class="muted">${now(p.id)}</small></span></label>`)}</div>
    <${More} id="redump-more">
      <p>Redump's DATs change most days: get them again for the newest. A version you have already is kept as it is.</p>
      <p>Some may be given only to people signed in on redump.org: download those there and import them with Import DATs….</p>
    <//>
  <//>`;
}

/** When DATs were last updated, as a date. */
export const updatedOn = (at) => (at ? new Date(at).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }) : "");

/** Update DATs (D172): the systems with a DAT (or every one), from Redump and libretro. */
function UpdateDatsDialog() {
  const [info, setInfo] = useState(null);
  const [every, setEvery] = useState(false);
  const [redump, setRedump] = useState(true);
  const [libretro, setLibretro] = useState(true);
  useEffect(() => { api("dats_update_info").then(setInfo).catch((e) => setInfo({ error: e.message || String(e) })); }, []);
  const n = every ? info?.every : info?.have?.length;
  const none = !redump && !libretro;
  return html`<${Dialog} title="Update DATs" id="dats-update-dialog" wide=${true} onSubmit=${() => { close(); runUpdateDats({ every, redump, libretro }); }}
    busy=${!info || !!info.error || !n || none} error=${info?.error || ""} submitLabel=${n ? `Update ${n === 1 ? "1 system" : `${n} systems`}` : "Update"}>
    <p>The newest DAT of each system is downloaded and imported. Every version you have is kept: a new one is used unless you chose an older one, and <em>Use this version</em> in the DAT list goes back to any of them.${info?.updated ? html` <span class="muted" id="dats-updated">Last updated ${updatedOn(info.updated)}.</span>` : null}</p>
    <fieldset class="field-block dats-update-set" id="dats-update-which"><legend class="field-label">Which systems</legend>
      <label class="check-row"><input type="radio" name="dats-which" value="have" checked=${!every} onChange=${() => setEvery(false)} /> The ${info ? plural(info.have.length, "system", "systems") : "systems"} with a DAT</label>
      <label class="check-row"><input type="radio" name="dats-which" value="every" checked=${every} onChange=${() => setEvery(true)} /> Every system Game Repo knows${info ? ` (${info.every})` : ""}: those without a DAT get one too</label>
    </fieldset>
    <fieldset class="field-block dats-update-set" id="dats-update-from"><legend class="field-label">From</legend>
      <label class="check-row long"><input type="checkbox" id="dats-from-redump" checked=${redump} onChange=${(e) => setRedump(e.target.checked)} />
        <span>Redump (redump.org)<br /><small class="muted">Its own DATs for the disc systems, with every track of each disc.</small></span></label>
      <label class="check-row long"><input type="checkbox" id="dats-from-libretro" checked=${libretro} onChange=${(e) => setLibretro(e.target.checked)} />
        <span>libretro (RetroArch's database on GitHub)<br /><small class="muted">Its copies of No-Intro's DATs for the cartridge systems, and of Redump's for a disc system Redump gives none for.</small></span></label>
    </fieldset>
    ${none ? html`<p class="warn-note" role="alert">Tick Redump, libretro or both.</p>` : null}
    <${More} id="dats-update-more">
      <p>A version you have already is kept as it is, so updating often costs nothing but the downloads. Redump's DATs change most days; libretro's copies a few times a month.</p>
      <p>Older versions stay in the library's <code>_library/dats</code> folder until you remove them: the DAT list folds them under each DAT, with <em>Remove older versions…</em>.</p>
      ${info?.arcade?.length ? html`<p>Arcade systems' DATs come with MAME's and FBNeo's releases, so they aren't updated here: import a new one with Import DATs….</p>` : null}
      <p>Some of Redump's are given only to people signed in on redump.org: libretro's copy comes instead, or download Redump's there and import it.</p>
    <//>
  <//>`;
}

export function AddSystemDialogs({ dialog }) {
  if (dialog.type === "add-system") return html`<${AddSystemDialog} ...${dialog} />`;
  if (dialog.type === "redump") return html`<${RedumpDialog} ...${dialog} />`;
  if (dialog.type === "dats-update") return html`<${UpdateDatsDialog} />`;
  return null;
}
