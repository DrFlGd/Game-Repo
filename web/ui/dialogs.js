// Dialogs. Which one is open is `ui.dialog`: { type: "ask", … } (a question with
// a button that does it) or { type: "confirm", … } (before something that can't
// be undone). The pages that need forms of their own add their types here.
import { html, useState, useLayoutEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { Icon } from "./icons.js";

export const close = () => ui.set({ dialog: null });

/** A dialog: Esc or Cancel closes it; `danger` makes the button red (it deletes).
 *  Esc and the first box work as soon as it shows (a layout effect, not after a paint). */
export function Dialog({ title, id, onSubmit, busy, error, submitLabel, danger = false, cancelLabel = "Cancel", children }) {
  const form = useRef(null);
  useLayoutEffect(() => {
    const key = (e) => { if (e.key === "Escape") close(); };
    addEventListener("keydown", key);
    const f = form.current;
    if (f && !f.contains(document.activeElement)) (f.querySelector("input:not([type=hidden]):not([disabled]), select, textarea") || f).focus();
    return () => removeEventListener("keydown", key);
  }, []);
  return html`<div class="dialog-backdrop" onMouseDown=${(e) => { if (e.target === e.currentTarget) close(); }}>
    <form class="dialog" id=${id} ref=${form} tabindex="-1" role="dialog" aria-modal="true" aria-label=${title} onSubmit=${(e) => { e.preventDefault(); onSubmit(); }}>
      <div class="dialog-head"><h2>${title}</h2>
        <button type="button" class="ghost" aria-label="Close" title="Close (Esc)" onClick=${close}>${Icon.close(15)}</button></div>
      ${children}
      <div class="dialog-foot">
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
        <div class="dialog-actions">
          <button type="button" class="ghost" id=${id ? `${id}-cancel` : null} onClick=${close}>${cancelLabel}</button>
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

export function Dialogs() {
  const dialog = useStore(ui, (s) => s.dialog);
  if (!dialog) return null;
  if (dialog.type === "ask") return html`<${AskDialog} ...${dialog} />`;
  if (dialog.type === "confirm") return html`<${ConfirmDialog} ...${dialog} />`;
  return null;
}
