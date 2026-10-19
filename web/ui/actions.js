// The menu and the keys (from Model Library): one menu drawn over everything
// (right-click, ⋯), one handler for the window's keys, and each page says what
// its keys do. Game actions, drawn as buttons and as menu items from one list,
// come with the pages that need them (Phase 1 on).
import { html, useState, useEffect, useRef, useLayoutEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { Icon } from "./icons.js";
import { undoLast } from "./library.js";

/** True, or why the library can't be changed. */
export const writable = () => (ui.get().library?.read_only ? "The library is read-only." : true);

// ---------------------------------------------------------------- the menu

/** Open a menu: at a mouse event's point (or { clientX, clientY }), or under an
 *  element (a More button). `items`: [{ id, label, sub (a second line), icon, key,
 *  run, disabled (false or the reason), danger }, { sep: true } or { head: text }]. */
export function openMenu(at, items) {
  if (!items.some((it) => !it.sep && !it.head)) return;
  let x, y, from = document.activeElement;
  if (at instanceof Element) {
    const r = at.getBoundingClientRect();
    x = r.left;
    y = r.bottom + 4;
    from = at;
  } else {
    at.preventDefault?.();
    x = at.clientX;
    y = at.clientY;
  }
  ui.set({ menu: { x, y, items, from } });
}

export const closeMenu = () => {
  const m = ui.get().menu;
  if (!m) return;
  ui.set({ menu: null });
  if (m.from?.isConnected) m.from.focus?.({ preventScroll: true });
};

/** The menu that's open, drawn over everything. Arrow keys move, Enter picks, Esc closes. */
export function ContextMenu() {
  const menu = useStore(ui, (s) => s.menu);
  const ref = useRef(null);
  const [pos, setPos] = useState(null);
  useLayoutEffect(() => {
    setPos(null);
    if (!menu || !ref.current) return;
    const r = ref.current.getBoundingClientRect();
    const x = Math.max(8, Math.min(menu.x, innerWidth - r.width - 8));
    const y = menu.y + r.height > innerHeight - 8 ? Math.max(8, menu.y - r.height - 8) : menu.y;
    setPos({ x, y });
  }, [menu]);
  // once it's placed (and visible), the first item takes the keys
  useLayoutEffect(() => {
    if (pos) ref.current?.querySelector(".menu-item:not(:disabled)")?.focus({ preventScroll: true });
  }, [pos]);
  useEffect(() => {
    if (!menu) return;
    const outside = (e) => { if (!ref.current?.contains(e.target)) closeMenu(); };
    const away = () => closeMenu();
    addEventListener("mousedown", outside, true);
    addEventListener("contextmenu", outside, true);
    addEventListener("resize", away);
    addEventListener("blur", away);
    addEventListener("wheel", away, { passive: true });
    return () => {
      removeEventListener("mousedown", outside, true);
      removeEventListener("contextmenu", outside, true);
      removeEventListener("resize", away);
      removeEventListener("blur", away);
      removeEventListener("wheel", away);
    };
  }, [menu]);
  if (!menu) return null;
  const onKey = (e) => {
    const all = [...ref.current.querySelectorAll(".menu-item:not(:disabled)")];
    const i = all.indexOf(document.activeElement);
    const go = (n) => { e.preventDefault(); all[(n + all.length) % all.length]?.focus(); };
    if (e.key === "ArrowDown" || (e.key === "ArrowRight" && document.activeElement?.classList.contains("menu-swatch"))) go(i + 1);
    else if (e.key === "ArrowUp" || (e.key === "ArrowLeft" && document.activeElement?.classList.contains("menu-swatch"))) go(i < 0 ? -1 : i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(-1);
    else if (e.key === "Escape" || e.key === "Tab") { e.preventDefault(); e.stopPropagation(); closeMenu(); }
  };
  return html`<div class="menu" role="menu" id="context-menu" ref=${ref} onKeyDown=${onKey}
      style=${pos ? `left:${pos.x}px;top:${pos.y}px` : `left:${menu.x}px;top:${menu.y}px;visibility:hidden`}>
    ${menu.items.map((it, i) => it.sep ? html`<div class="menu-sep" role="separator" key=${`sep${i}`}></div>`
      : it.head ? html`<div class="menu-head" key=${`head${i}`}>${it.head}</div>`
      : it.swatches ? html`<div class="menu-swatches" role="group" aria-label=${it.label} key=${`sw${i}`}>${it.swatches.map((w) => html`<button type="button" role="menuitemradio" key=${w.id}
          data-action=${w.id} class="menu-item menu-swatch" aria-checked=${w.on ? "true" : "false"} title=${w.label} aria-label=${w.label}
          onClick=${() => { closeMenu(); w.run(); }}><span class="swatch" style=${{ background: w.hex }}></span></button>`)}</div>`
      : html`<button type="button" role="menuitem" key=${it.id || i} data-action=${it.id} class=${`menu-item${it.danger ? " danger-text" : ""}`}
          disabled=${!!it.disabled} title=${typeof it.disabled === "string" ? it.disabled : it.title || null}
          onClick=${() => { closeMenu(); it.run(); }}>
          <span class="menu-icon">${it.icon && Icon[it.icon] ? Icon[it.icon](14, it.filled) : null}</span><span class="menu-label">${it.swatch ? html`<span class="swatch" style=${{ background: it.swatch }}></span>` : null}${it.label}${it.sub ? html`<small class="menu-sub">${it.sub}</small>` : null}</span>${it.key ? html`<kbd>${it.key}</kbd>` : null}</button>`)}
  </div>`;
}

// ---------------------------------------------------------------- keys

let pageKeys = null;

/** What a page's keys do: `fn(e)` sees each key press that isn't typing in a box
 *  (and no dialog or menu is open). Shift+F10 and the menu key come as "ContextMenu". */
export function usePageKeys(fn) {
  const ref = useRef(fn);
  ref.current = fn;
  useLayoutEffect(() => {
    const h = (e) => ref.current(e);
    pageKeys = h;
    return () => { if (pageKeys === h) pageKeys = null; };
  }, []);
}

/** Whether a key press is typing (in a box, or on a button where Enter and Space press it). */
export const typing = (e) => !!e.target.closest?.("input, textarea, select, [contenteditable='true']");
const onControl = (e) => !!e.target.closest?.("button, a[href], summary");

/** The window's keys: Ctrl+Z undoes, "/" goes to the page's search box, the rest
 *  goes to the page. */
export function onWindowKey(e) {
  const s = ui.get();
  if (s.menu && e.key === "Escape") { closeMenu(); return; }
  if (s.dialog || s.menu || e.defaultPrevented || typing(e)) return;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && key === "z") {
    e.preventDefault();
    undoLast();
    return;
  }
  if (key === "/" && !e.ctrlKey && !e.metaKey && !e.altKey) {
    const box = document.querySelector("[data-search]");
    if (box) { e.preventDefault(); box.focus(); box.select?.(); }
    return;
  }
  // Enter on a game's tile or line opens it, as a double click does (review M38)
  const tile = key === "Enter" && !e.ctrlKey && !e.metaKey && !e.altKey && e.target.closest?.(".lib-tile, .lib-line");
  if (tile) {
    e.preventDefault();
    tile.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    return;
  }
  if ((key === "Enter" || key === " ") && onControl(e)) return;
  if ((e.shiftKey && key === "F10") || key === "ContextMenu") {
    e.preventDefault();
    pageKeys?.({ key: "ContextMenu", target: e.target, shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, preventDefault() {} });
    return;
  }
  pageKeys?.(e);
}

/** The plain letter of a key press (no Ctrl, Alt or Cmd), or "". */
export const letter = (e) => (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1 ? e.key.toLowerCase() : "");
/** Ctrl+A (Cmd+A). */
export const selectAllKey = (e) => (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "a";
