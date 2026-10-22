// Dialogs. Which one is open is `ui.dialog`: { type: "ask", … } (a question with
// a button that does it), { type: "confirm", … } (before something that can't
// be undone), "dat-platform" (which system a DAT is for), "make-dat" (a DAT
// from a scanned set), "organise" and "convert" (organise.js), "system"
// (systems.js), a collection's (collections.js), the PC games' and Identify's (pcgames.js), Scrape… (metadata.js), the documents' (documents.js), Gather the BIOS files (bios.js), Save as a new layout (layouts.js), or the Library's Add to a collection… and Save as a collection… (browse.js), or a variant's (variants.js), or a system's DATs (audit.js). The pages that need forms of their own add their types here.
import { html, useState, useLayoutEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { Icon } from "./icons.js";
import { importDats, makeDat } from "./games.js";
import { OrganiseDialog, ConvertDialog } from "./organise.js";
import { SystemDialogs, resolveSystem, idFromName } from "./systems.js";
import { CollectionDialogs } from "./collections.js";
import { PcDialogs } from "./pcgames.js";
import { MetaDialogs } from "./metadata.js";
import { DocDialogs } from "./documents.js";
import { MagazineDialogs } from "./magazines.js";
import { VideoDialogs } from "./videos.js";
import { BiosDialogs } from "./bios.js";
import { ImportDialogs } from "./importpage.js";
import { LayoutDialogs } from "./layouts.js";
import { BrowseDialogs } from "./browse.js";
import { VariantDialogs } from "./variants.js";
import { AuditDialogs } from "./audit.js";
import { ShelfDialogs } from "./shelf.js";

export const close = () => ui.set({ dialog: null });

/** A dialog: Esc or Cancel closes it (`onDismiss` and `onCancel` say otherwise); `danger` makes the button red (it deletes).
 *  A `sticky` one (an import with what it read shown, or work running) stays when the page
 *  behind it is clicked, and Esc leaves it too unless `onDismiss` puts it away with nothing
 *  lost (review M35); its ✕ and Cancel still close it.
 *  Esc and the first box work as soon as it shows (a layout effect, not after a paint). */
export function Dialog({ title, id, onSubmit, busy, error, submitLabel, danger = false, cancelLabel = "Cancel", onCancel = close, onDismiss, wide = false, sticky = false, children }) {
  const form = useRef(null);
  const dismiss = onDismiss || close;
  const esc = useRef(null);
  esc.current = sticky && !onDismiss ? null : dismiss;
  useLayoutEffect(() => {
    const key = (e) => { if (e.key === "Escape") esc.current?.(); };
    addEventListener("keydown", key);
    const f = form.current;
    if (f && !f.contains(document.activeElement)) (f.querySelector("input:not([type=hidden]):not([disabled]), select, textarea") || f).focus();
    return () => removeEventListener("keydown", key);
  }, []);
  // a sticky dialog stays when the page behind it is clicked (an import with work running or kept)
  return html`<div class="dialog-backdrop" onMouseDown=${(e) => { if (e.target === e.currentTarget && !sticky) dismiss(); }}>
    <form class=${wide ? "dialog wide" : "dialog"} id=${id} ref=${form} tabindex="-1" role="dialog" aria-modal="true" aria-label=${title} onSubmit=${(e) => { e.preventDefault(); onSubmit(); }}>
      <div class="dialog-head"><h2>${title}</h2>
        <button type="button" class="ghost" aria-label="Close" title=${esc.current ? "Close (Esc)" : "Close"} onClick=${dismiss}>${Icon.close(15)}</button></div>
      ${children}
      <div class="dialog-foot">
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
        <div class="dialog-actions">
          ${cancelLabel ? html`<button type="button" class="ghost" id=${id ? `${id}-cancel` : null} onClick=${onCancel}>${cancelLabel}</button>` : null}
          <button type="submit" class=${danger ? "primary danger" : "primary"} disabled=${busy}>${submitLabel}</button>
        </div>
      </div>
    </form>
  </div>`;
}

/** What a dialog needn't say at once, folded under *More about this* (D147): a dialog says
 *  what it does in one line, and the rest is here or in Help. */
export function More({ id, children }) {
  return html`<details class="more-about" id=${id}><summary>More about this</summary>${children}</details>`;
}

/** A question with a button that does it: `{ title, text (one or more paragraphs), button, cancel, run }`.
 *  The first paragraph says it; the rest are under More about this (D147). */
function AskDialog({ title, text, button, cancel = "Not now", run }) {
  const [first, ...rest] = [].concat(text);
  return html`<${Dialog} title=${title} id="ask-dialog" onSubmit=${() => { close(); run(); }} submitLabel=${button} cancelLabel=${cancel}>
    <p>${first}</p>
    ${rest.length ? html`<${More} id="ask-more">${rest.map((t, i) => html`<p key=${i}>${t}</p>`)}<//>` : null}
  <//>`;
}

/** Ask before something that can't be undone:
 *  one red button that names what it does. Open it with `confirmDialog`. */
function ConfirmDialog({ title, text, button, run, undoable = false }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await run();
      close();
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${title} id="confirm-dialog" onSubmit=${submit} busy=${busy} error=${error} submitLabel=${button} danger=${true}>
    <p>${text}</p>
    ${undoable ? null : html`<p class="muted">This can't be undone.</p>`}
  <//>`;
}

/** `{ title, text, button, run, undoable }`: `run()` is what happens after the red button;
 *  `undoable`, History can undo it (so it isn't said that it can't be). */
export const confirmDialog = (opts) => ui.set({ dialog: { type: "confirm", ...opts } });

/** A system picker: every system the app knows (built-in and added), or a new one,
 *  named here and added when the form is sent (its value is then `{ new, name, id }`,
 *  which `resolveSystem` turns into an id). */
function PlatformSelect({ id, value, onChange, none }) {
  const list = useStore(ui, (s) => s.platforms) || [];
  const isNew = value && typeof value === "object";
  const sorted = [...list].sort((a, b) => a.name.localeCompare(b.name));
  return html`<span class="platform-pick">
    <select id=${id} class="compact" value=${isNew ? "__new" : value || ""} onChange=${(e) => {
      if (e.target.value === "__new") onChange({ new: true, name: "", id: "" }); else onChange(e.target.value);
    }}>
      <option value="">${none}</option>
      ${sorted.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}${p.custom ? " (added)" : ""}</option>`)}
      <option value="__new">A new system…</option>
    </select>
    ${isNew ? html`<span class="new-system">
      <input type="text" id=${`${id}-name`} class="compact" maxlength="80" placeholder="The system's name, e.g. Satellaview" aria-label="The new system's name" value=${value.name}
        onInput=${(e) => onChange({ ...value, name: e.target.value, id: value.idTyped ? value.id : idFromName(e.target.value) })} />
      <input type="text" id=${`${id}-id`} class="compact system-id" maxlength="40" placeholder="Folder name" aria-label="The new system's folder name (for exports)" title="Its folder name: the folder exports use" value=${value.id}
        onInput=${(e) => onChange({ ...value, id: e.target.value.trim().toLowerCase(), idTyped: true })} />
    </span>` : null}
  </span>`;
}

/** Whether a PlatformSelect has an answer. */
const answered = (v) => (v && typeof v === "object" ? !!(v.name.trim() && v.id) : !!v);

/** Which system is each of these DATs for? Imported again with the answers, which the library
 *  remembers; a new system is added first, with the DAT's name, so its next version is found. */
function DatPlatformDialog({ asks, answers }) {
  const [picked, setPicked] = useState(() => Object.fromEntries(asks.map((a) => [a.name, ""])));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const ready = asks.every((a) => answered(picked[a.name]));
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const ids = {};
      for (const a of asks) ids[a.name] = await resolveSystem(typeof picked[a.name] === "object" ? { ...picked[a.name], name: picked[a.name].name.trim() } : picked[a.name], a.name);
      close();
      await importDats([...new Set(asks.map((a) => a.path))], { ...answers, ...ids });
    } catch (e) {
      setError(e.message || String(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title="Which system are these DATs for?" id="dat-platform-dialog" onSubmit=${submit} busy=${busy || !ready} error=${error} submitLabel="Import">
    <p>Game Repo couldn't tell from ${asks.length === 1 ? "this DAT's name" : "these DATs' names"}. Choose a system, or add a new one. Your answer is kept in the library, so newer versions are recognised.</p>
    ${asks.map((a, i) => html`<div class="field-block" key=${a.name}><span class="field-label">${a.name}</span>
      <small class="muted">${a.file}</small>
      <${PlatformSelect} id=${`dat-platform-${i}`} value=${picked[a.name]} none="Choose…" onChange=${(v) => setPicked((p) => ({ ...p, [a.name]: v }))} /></div>`)}
  <//>`;
}

/** A DAT made from a scanned set: which folder, its name, and whether to import it. */
function MakeDatDialog() {
  const roots = useStore(ui, (s) => s.roots) || [];
  const [root, setRoot] = useState(roots[0]?.id || "");
  const [folder, setFolder] = useState("");
  const [name, setName] = useState("");
  const [platform, setPlatform] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const system = await resolveSystem(platform, name.trim());
      await makeDat({ root, folder: folder.trim().replace(/\\/g, "/"), name: name.trim(), platform: system || null });
      close();
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  if (!roots.length) {
    return html`<${Dialog} title="Make a DAT from a folder" id="make-dat-dialog" onSubmit=${close} submitLabel="OK">
      <p>Add the folder (Import › Folders with games) and scan it first: the DAT is made from what the scan read.</p><//>`;
  }
  return html`<${Dialog} title="Make a DAT from a folder" id="make-dat-dialog" onSubmit=${submit} busy=${busy || !name.trim() || !root || (typeof platform === "object" && !answered(platform))} error=${error} submitLabel="Make the DAT">
    <p class="muted">Each file (or ZIP) the last scan found becomes a game in a standard (Logiqx) DAT.</p>
    <label class="field-block"><span class="field-label">Folder with games</span>
      <select id="make-dat-root" class="compact" value=${root} onChange=${(e) => setRoot(e.target.value)}>
        ${roots.map((r) => html`<option key=${r.id} value=${r.id}>${r.dir}</option>`)}
      </select></label>
    <label class="field-block"><span class="field-label">Only the folder inside it (optional)</span>
      <input type="text" id="make-dat-folder" placeholder="e.g. Game Boy/Translations" value=${folder} onInput=${(e) => setFolder(e.target.value)} /></label>
    <label class="field-block"><span class="field-label">The DAT's name</span>
      <input type="text" id="make-dat-name" maxlength="150" placeholder="e.g. My Game Boy set" value=${name} onInput=${(e) => setName(e.target.value)} /></label>
    <div class="field-block"><span class="field-label">Import it to audit against</span>
      <${PlatformSelect} id="make-dat-platform" value=${platform} none="Don't import it" onChange=${setPlatform} /></div>
    <${More} id="make-dat-more"><p>It's saved in the library's <code>_library/made-dats</code> folder. ZIPs are read in full, so every file in them has its full hash.</p><//>
  <//>`;
}

export function Dialogs() {
  const dialog = useStore(ui, (s) => s.dialog);
  if (!dialog) return null;
  if (dialog.type === "ask") return html`<${AskDialog} ...${dialog} />`;
  if (dialog.type === "confirm") return html`<${ConfirmDialog} ...${dialog} />`;
  if (dialog.type === "dat-platform") return html`<${DatPlatformDialog} ...${dialog} />`;
  if (dialog.type === "make-dat") return html`<${MakeDatDialog} ...${dialog} />`;
  if (dialog.type === "organise") return html`<${OrganiseDialog} key=${dialog.plan?.id || "new"} ...${dialog} />`;
  if (dialog.type === "convert") return html`<${ConvertDialog} ...${dialog} />`;
  return SystemDialogs({ dialog }) || CollectionDialogs({ dialog }) || PcDialogs({ dialog }) || MetaDialogs({ dialog }) || DocDialogs({ dialog }) || MagazineDialogs({ dialog }) || VideoDialogs({ dialog }) || BiosDialogs({ dialog }) || ImportDialogs({ dialog }) || LayoutDialogs({ dialog }) || BrowseDialogs({ dialog }) || VariantDialogs({ dialog }) || AuditDialogs({ dialog }) || ShelfDialogs({ dialog });
}
