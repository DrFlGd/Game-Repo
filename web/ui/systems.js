// Systems (docs/PLAN.md, "Phase 3 design", D22): Settings › Systems lists the built-in
// ones and those added by hand, which can be added, changed and removed; the DAT
// question offers every system or a new one. Added systems are kept in the library.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { api, toast } from "./library.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";

export const MEDIA = [
  ["cartridge", "Cartridges (ZIP)"],
  ["cd", "CDs (CHD)"],
  ["nintendo", "GameCube or Wii discs (RVZ)"],
  ["umd", "PSP discs (CSO, ZSO, CHD)"],
  ["audit", "Audit only (no conversions)"],
  ["arcade", "Arcade (a ZIP per machine, as MAME or FBNeo)"],
];

/** An id from a name, as the app makes it: lower case letters and digits. */
export const idFromName = (name) => name.toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, "").slice(0, 40);

/** Read the systems again (after one is added, changed or removed). */
export async function refreshPlatforms() {
  ui.set({ platforms: await api("platforms") });
  return ui.get().platforms;
}

/** Add a system or change one (`old`: its id until now). Resolves with it as saved. */
export async function saveSystem(system, old = null) {
  const saved = await api("systems_set", { system, old });
  await refreshPlatforms();
  return saved;
}

/** A PlatformSelect's value as a system's id, adding the system first when it's a new
 *  one (`{ new: true, name, id }`), with `datName` as its DAT name. */
export async function resolveSystem(value, datName) {
  if (!value || typeof value === "string") return value;
  const saved = await saveSystem({ id: value.id || idFromName(value.name), name: value.name, dat_names: datName ? [datName] : [], media: value.media || "cartridge" });
  return saved.id;
}

function SystemDialog({ system }) {
  const editing = !!system;
  const [name, setName] = useState(system?.name || "");
  const [id, setId] = useState(system?.id || "");
  const [idTyped, setIdTyped] = useState(editing);
  const [datNames, setDatNames] = useState((system?.dat_names || []).join(", "));
  const [folders, setFolders] = useState((system?.folders || []).join(", "));
  const [exts, setExts] = useState((system?.exts || []).join(", "));
  const [media, setMedia] = useState(system?.media || "cartridge");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const shownId = idTyped ? id : idFromName(name);
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const s = await saveSystem({ id: shownId, name, dat_names: datNames, folders, exts, media }, system?.id || null);
      toast(editing ? `Saved ${s.name}.` : `Added ${s.name}: DATs named ${s.dat_names[0] || "as it"} are found for it.`, 6000);
      close();
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${editing ? `Change ${system.name}` : "Add a system"} id="system-dialog" onSubmit=${submit} busy=${busy || !name.trim() || !shownId} error=${error} submitLabel=${editing ? "Save" : "Add"}>
    <label class="field-block"><span class="field-label">Name</span>
      <input type="text" id="system-name" maxlength="80" placeholder="e.g. Satellaview" value=${name} onInput=${(e) => setName(e.target.value)} /></label>
    <label class="field-block"><span class="field-label">Folder name</span>
      <input type="text" id="system-id" maxlength="40" value=${shownId} onInput=${(e) => { setIdTyped(true); setId(e.target.value.trim().toLowerCase()); }} />
      <small class="muted">The folder exports use: ES-DE's name for the system, if it has one (its <code>es_systems.xml</code> lists them).${editing && system.dats ? " It can't change while the system has DATs." : ""}</small></label>
    <label class="field-block"><span class="field-label">DAT names</span>
      <input type="text" id="system-dats" placeholder="e.g. Nintendo - Satellaview" value=${datNames} onInput=${(e) => setDatNames(e.target.value)} />
      <small class="muted">The names in its DATs' headers, separated by commas: a DAT whose name starts with one of them is found for this system.</small></label>
    <label class="field-block"><span class="field-label">Folder names</span>
      <input type="text" id="system-folders" placeholder="e.g. Satellaview, BS-X" value=${folders} onInput=${(e) => setFolders(e.target.value)} />
      <small class="muted">Folders with these names hold its games.</small></label>
    <label class="field-block"><span class="field-label">File extensions</span>
      <input type="text" id="system-exts" placeholder="e.g. bs" value=${exts} onInput=${(e) => setExts(e.target.value)} /></label>
    <label class="field-block"><span class="field-label">Its games are on</span>
      <select id="system-media" class="compact" value=${media} onChange=${(e) => setMedia(e.target.value)}>
        ${MEDIA.map(([v, label]) => html`<option key=${v} value=${v}>${label}</option>`)}
      </select>
      <small class="muted">Which formats Convert… offers for it.</small></label>
  <//>`;
}

export function SystemsSection({ lib }) {
  const list = useStore(ui, (s) => s.platforms) || [];
  const dats = useStore(ui, (s) => s.dats) || [];
  const [showAll, setShowAll] = useState(false);
  const off = !!lib.read_only;
  const added = list.filter((p) => p.custom);
  const builtIn = list.filter((p) => !p.custom);
  const withDats = (id) => dats.filter((d) => d.platform === id).length;
  const remove = (p) => confirmDialog({
    title: `Remove ${p.name}?`, text: `${p.name} is taken out of the library's systems. Its folders and files aren't touched.`, button: "Remove",
    run: async () => { await api("systems_remove", { id: p.id }); await refreshPlatforms(); toast(`Removed ${p.name}.`); },
  });
  const open = (p) => ui.set({ dialog: { type: "system", system: p ? { ...p, dats: withDats(p.id) } : null } });
  // Phase 7 (D60): how each arcade system's ZIPs are made up
  const [sets, setSets] = useState(null);
  useEffect(() => { api("arcade_sets").then(setSets).catch(() => setSets(null)); }, [lib.path, list.length]);
  const arcade = list.filter((p) => p.media === "arcade");
  const chooseSet = async (p, t) => {
    try { setSets(await api("arcade_set", { platform: p.id, set_type: t })); toast(`${p.name} is kept as a ${t} set now.`); }
    catch (e) { toast(`Couldn't change the set type: ${e.message || e}`, 8000); }
  };
  return html`<section class="ls-section" id="settings-systems">
    <h2>Systems</h2>
    <p class="muted">The systems the app knows. Add one it doesn't (a DAT's system, a computer, a handheld) and its DATs, folders and files are found like any other's. Added systems are kept in the library.</p>
    ${added.length ? html`<ul class="ls-list change-list" id="systems-added">${added.map((p) => html`<li key=${p.id} data-system=${p.id}>
      <span class="change-label"><span><b>${p.name}</b> <span class="muted">${p.id}</span></span>
        <span class="muted">${[p.dat_names?.length ? `DATs: ${p.dat_names.join(", ")}` : "No DAT names yet", p.exts?.length ? `.${p.exts.join(", .")}` : null, withDats(p.id) ? `${withDats(p.id)} DAT${withDats(p.id) === 1 ? "" : "s"}` : null].filter(Boolean).join(" · ")}</span></span>
      <button type="button" class="ghost" data-action="edit" disabled=${off} onClick=${() => open(p)}>Change…</button>
      <button type="button" class="ghost" data-action="remove" disabled=${off || withDats(p.id) > 0} title=${withDats(p.id) ? "Remove its DATs first (DAT files, above)" : null} onClick=${() => remove(p)}>Remove</button>
    </li>`)}</ul>` : html`<p class="muted" id="systems-none">No systems added yet.</p>`}
    <div class="home-actions">
      <button type="button" class="ghost" id="system-add" disabled=${off} onClick=${() => open(null)}>Add a system…</button>
      <button type="button" class="ghost" id="systems-built-in" aria-expanded=${showAll ? "true" : "false"} onClick=${() => setShowAll(!showAll)}>${showAll ? "Hide" : "Show"} the ${builtIn.length} built-in systems</button>
    </div>
    ${arcade.length && sets ? html`<h3>Arcade set types</h3>
      <p class="muted">How each arcade system's ZIPs are made up. Non-merged: every game's ZIP holds all it needs but its BIOS's. Split: a clone's ZIP holds only what its parent's doesn't. Merged: a parent's ZIP holds its clones' too. Organise builds ZIPs this way; an export is always non-merged.</p>
      <ul class="ls-list" id="arcade-set-types">${arcade.map((p) => html`<li key=${p.id} data-system=${p.id}><span>${p.name}</span>
        <select class="compact" aria-label=${`${p.name}'s set type`} value=${sets.systems?.[p.id] || "non-merged"} disabled=${off} onChange=${(e) => chooseSet(p, e.target.value)}>
          ${(sets.types || []).map((t) => html`<option key=${t} value=${t}>${t[0].toUpperCase()}${t.slice(1)}</option>`)}
        </select></li>`)}</ul>` : null}
    ${showAll ? html`<ul class="ls-list" id="systems-list">${builtIn.map((p) => html`<li key=${p.id}><span>${p.name}</span><span class="muted">${p.id} · ${p.dat || ""}</span></li>`)}</ul>` : null}
  </section>`;
}

export function SystemDialogs({ dialog }) {
  if (dialog.type === "system") return html`<${SystemDialog} key=${dialog.system?.id || "new"} system=${dialog.system} />`;
  return null;
}
