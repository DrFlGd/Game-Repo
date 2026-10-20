// Game Repo Go's dialogs: Game Repo's look (styles.css's .dialog), Esc or Cancel closes,
// the first control (or the one marked data-first) is focused, and the arrow keys stay
// inside it (keys.js). In the big-screen view each is a panel from the right (client.css).
import { html, useEffect, useLayoutEffect, useRef } from "../lib/html.js";
import { Icon } from "../ui/icons.js";
import { closeDialog } from "./state.js";

export function Dialog({ title, id, onSubmit, busy, disabled = false, error, submitLabel, danger = false, cancelLabel = "Cancel", onCancel = closeDialog, wide = false, children }) {
  const form = useRef(null);
  useLayoutEffect(() => {
    const key = (e) => { if (e.key === "Escape" && !e.defaultPrevented) { e.preventDefault(); onCancel(); } };
    addEventListener("keydown", key);
    const f = form.current;
    if (f && !f.contains(document.activeElement)) (f.querySelector("[data-first]") || f.querySelector("input:not([type=hidden]):not([disabled]), select, button.primary") || f).focus();
    return () => removeEventListener("keydown", key);
  }, []);
  // a dialog whose main button waits for an answer (Start downloads…'s sizes): focused once
  // it can be pressed, unless something else in the dialog has been chosen meanwhile
  useEffect(() => {
    const f = form.current;
    const main = f?.querySelector("button.primary:not([disabled])");
    if (main && (document.activeElement === f || !f.contains(document.activeElement))) main.focus();
  }, [busy || disabled]);
  return html`<div class="dialog-backdrop" onMouseDown=${(e) => { if (e.target === e.currentTarget && !busy) onCancel(); }}>
    <form class=${wide ? "dialog wide" : "dialog"} id=${id} ref=${form} tabindex="-1" role="dialog" aria-modal="true" aria-label=${title}
      onSubmit=${(e) => { e.preventDefault(); if (!busy && !disabled) onSubmit(); }}>
      <div class="dialog-head"><h2>${title}</h2>
        <button type="button" class="ghost" aria-label="Close" title="Close (Esc)" onClick=${onCancel}>${Icon.close(15)}</button></div>
      ${children}
      <div class="dialog-foot">
        ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
        <div class="dialog-actions">
          ${cancelLabel ? html`<button type="button" class="ghost" onClick=${onCancel}>${cancelLabel}</button>` : null}
          ${submitLabel ? html`<button type="submit" class=${danger ? "primary danger" : "primary"} disabled=${busy || disabled}>${submitLabel}</button>` : null}
        </div>
      </div>
    </form>
  </div>`;
}
