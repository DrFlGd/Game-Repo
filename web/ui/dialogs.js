// Dialogs. Which one is open is `ui.dialog`: { type: "ask", … } (a question with
// a button that does it), { type: "confirm", … } (before something that can't
// be undone), "dat-platform" (which platform a DAT is for), "make-dat" (a DAT
// from a scanned set) or "organise" (organise.js). The pages that need forms of their own add their types here.
import { html, useState, useLayoutEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { Icon } from "./icons.js";
import { importDats, makeDat } from "./games.js";
import { OrganiseDialog, ConvertDialog } from "./organise.js";

export const close = () => ui.set({ dialog: null });

/** A dialog: Esc or Cancel closes it; `danger` makes the button red (it deletes).
 *  Esc and the first box work as soon as it shows (a layout effect, not after a paint). */
export function Dialog({ title, id, onSubmit, busy, error, submitLabel, danger = false, cancelLabel = "Cancel", onCancel = close, wide = false, children }) {
  const form = useRef(null);
  useLayoutEffect(() => {
    const key = (e) => { if (e.key === "Escape") close(); };
    addEventListener("keydown", key);
    const f = form.current;
    if (f && !f.contains(document.activeElement)) (f.querySelector("input:not([type=hidden]):not([disabled]), select, textarea") || f).focus();
    return () => removeEventListener("keydown", key);
  }, []);
  return html`<div class="dialog-backdrop" onMouseDown=${(e) => { if (e.target === e.currentTarget) close(); }}>
    <form class=${wide ? "dialog wide" : "dialog"} id=${id} ref=${form} tabindex="-1" role="dialog" aria-modal="true" aria-label=${title} onSubmit=${(e) => { e.preventDefault(); onSubmit(); }}>
      <div class="dialog-head"><h2>${title}</h2>
        <button type="button" class="ghost" aria-label="Close" title="Close (Esc)" onClick=${close}>${Icon.close(15)}</button></div>
      ${children}
      <div class="dialog-foot">
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
        <div class="dialog-actions">
          <button type="button" class="ghost" id=${id ? `${id}-cancel` : null} onClick=${onCancel}>${cancelLabel}</button>
          <button type="submit" class=${danger ? "primary danger" : "primary"} disabled=${busy}>${submitLabel}</button>
        </div>
      </div>
    </form>
  </div>`;
}

/** A question with a button that does it: `{ title, text (one or more paragraphs), button, cancel, run }`. */
function AskDialog({ title, text, button, cancel = "Not now", run }) {
  return html`<${Dialog} title=${title} id="ask-dialog" onSubmit=${() => { close(); run(); }} submitLabel=${button} cancelLabel=${cancel}>
    ${[].concat(text).map((t, i) => html`<p key=${i} class=${i ? "muted" : null}>${t}</p>`)}
  <//>`;
}

/** Ask before something that can't be undone:
 *  one red button that names what it does. Open it with `confirmDialog`. */
function ConfirmDialog({ title, text, button, run }) {
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
    <p class="muted">This can't be undone.</p>
  <//>`;
}

/** `{ title, text, button, run }`: `run()` is what happens after the red button. */
export const confirmDialog = (opts) => ui.set({ dialog: { type: "confirm", ...opts } });

/** A platform picker: the platforms the app knows, or another (its id typed). */
function PlatformSelect({ id, value, onChange, none }) {
  const list = useStore(ui, (s) => s.platforms) || [];
  const known = !value || list.some((p) => p.id === value);
  const [other, setOther] = useState(!known);
  return html`<span class="platform-pick">
    <select id=${id} class="compact" value=${other ? "__other" : value || ""} onChange=${(e) => {
      if (e.target.value === "__other") { setOther(true); onChange(""); } else { setOther(false); onChange(e.target.value); }
    }}>
      <option value="">${none}</option>
      ${list.map((p) => html`<option key=${p.id} value=${p.id}>${p.name}</option>`)}
      <option value="__other">Another platform…</option>
    </select>
    ${other ? html`<input type="text" id=${`${id}-other`} class="compact" placeholder="its id, e.g. satellaview" value=${value}
      onInput=${(e) => onChange(e.target.value.trim().toLowerCase().replace(/[^a-z0-9._-]+/g, "-"))} />` : null}
  </span>`;
}

/** Which platform is each of these DATs for? Imported again with the answers, which the library remembers. */
function DatPlatformDialog({ asks, answers }) {
  const [picked, setPicked] = useState(() => Object.fromEntries(asks.map((a) => [a.name, ""])));
  const [busy, setBusy] = useState(false);
  const ready = asks.every((a) => picked[a.name]);
  const submit = async () => {
    setBusy(true);
    close();
    await importDats([...new Set(asks.map((a) => a.path))], { ...answers, ...picked });
  };
  return html`<${Dialog} title="Which platform are these DATs for?" id="dat-platform-dialog" onSubmit=${submit} busy=${busy || !ready} submitLabel="Import">
    <p>The app couldn't tell from ${asks.length === 1 ? "this DAT's name" : "these DATs' names"}. Your answer is kept in the library, so newer versions are recognised.</p>
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
      await makeDat({ root, folder: folder.trim().replace(/\\/g, "/"), name: name.trim(), platform: platform || null });
      close();
    } catch (e) {
      setError(e.message || String(e));
    } finally {
      setBusy(false);
    }
  };
  if (!roots.length) {
    return html`<${Dialog} title="Make a DAT from a set" id="make-dat-dialog" onSubmit=${close} submitLabel="OK">
      <p>Add the folder with the set (Settings › Folders with games) and scan it first: the DAT is made from what the scan read.</p><//>`;
  }
  return html`<${Dialog} title="Make a DAT from a set" id="make-dat-dialog" onSubmit=${submit} busy=${busy || !name.trim() || !root} error=${error} submitLabel="Make the DAT">
    <p class="muted">Each file (or ZIP) the last scan found becomes a game in a Logiqx DAT, saved in the library's <code>_library/made-dats</code> folder. ZIPs are read in full so every ROM has its SHA1.</p>
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
  return null;
}
