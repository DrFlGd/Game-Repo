// Every control reached with the arrow keys and Enter. In the classic view they go in a set
// order (D103), so a controller moves through the pages as a mouse would: Down and Right go
// to the next control, Up and Left to the one before (in a text box Left and Right move in
// the text, on a list they change it). In the big-screen view (D113) they go to the nearest
// control that way, as Steam's Big Picture does, the shoulder buttons change the page, Y
// searches, ☰ opens Options, and a list opens as a panel to choose from. Enter presses, and
// ticks a box; Esc or Backspace goes back.
// A part that moves itself (the Library's games, by rows and columns) says so with
// data-keys="own" and leaves the key alone at its edges. On Windows and Linux the page
// reads controllers itself (the Gamepad API; on the Steam Deck, the controller Steam Input
// gives it, D107); Android's plugin turns them into keys (part 3).

import { info, keyboard } from "./backend.js";

const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]';

const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";

/** The controls in their order: an open dialog's, else the page's (the menu, then the page).
 *  Within a part that moves itself, only its current control (tabindex 0) is a stop. */
export function controls() {
  const scope = document.querySelector(".dialog-backdrop .dialog") || document.getElementById("go-root") || document.body;
  return [...scope.querySelectorAll(FOCUSABLE)].filter((el) => el.tabIndex >= 0 && shown(el));
}

const typing = (t) => t?.matches?.('input:not([type="checkbox"]):not([type="radio"]):not([type="range"]), textarea');

const big = () => document.documentElement.dataset.view === "big";

/** Focus the nearest control in a direction (big screen): the one whose edge is closest
 *  that way, a control off to the side counting double, so Down from a game goes to the
 *  game below it, not the first of the next row. */
export function moveTo(dir) {
  const list = controls();
  const from = document.activeElement;
  if (!list.includes(from)) return focusFirst(list);
  const a = from.getBoundingClientRect();
  let best = null, score = Infinity;
  for (const el of list) {
    if (el === from || from.contains(el) || el.contains(from)) continue;
    const b = el.getBoundingClientRect();
    // how far ahead, and how far off to the side (0 when they overlap that way)
    const ahead = { right: b.left - a.right, left: a.left - b.right, down: b.top - a.bottom, up: a.top - b.bottom }[dir];
    const centre = { right: b.left + b.width / 2 - (a.left + a.width / 2), left: a.left + a.width / 2 - (b.left + b.width / 2),
      down: b.top + b.height / 2 - (a.top + a.height / 2), up: a.top + a.height / 2 - (b.top + b.height / 2) }[dir];
    if (centre <= 1 || ahead < -Math.min(a.width, a.height, b.width, b.height) / 2) continue;
    const across = dir === "left" || dir === "right"
      ? Math.max(0, b.top - a.bottom, a.top - b.bottom)
      : Math.max(0, b.left - a.right, a.left - b.right);
    const s = Math.max(0, ahead) + 2 * across;
    if (s < score) { score = s; best = el; }
  }
  if (!best) return false;
  show(best);
  return true;
}

/** Bring a control into view and focus it; a game on a shelf brings its shelf's name too. */
export function show(el) {
  el.focus({ preventScroll: true });
  el.closest("[data-shelf]")?.scrollIntoView?.({ block: "nearest" });
  el.scrollIntoView?.({ block: "nearest", inline: "nearest" });
}

/** With nothing focused, the page's first control (not a text box: Android would bring up
 *  its keyboard). */
export function focusFirst(list = controls()) {
  const el = list.find((e) => e.closest("#go-main, .dialog") && !typing(e)) || list.find((e) => !typing(e));
  if (!el) return false;
  show(el);
  return true;
}

/** A part that scrolls by itself (a game's description): Up and Down scroll it first. */
function scrolls(el, by) {
  if (!el?.matches?.(".go-scroll")) return false;
  const room = by > 0 ? el.scrollHeight - el.clientHeight - el.scrollTop : el.scrollTop;
  if (room <= 1) return false;
  el.scrollBy({ top: by * Math.min(room, el.clientHeight * 0.6) });
  return true;
}

/** Focus the next (`by` 1) or previous (-1) control from the one focused. */
export function move(by) {
  const list = controls();
  if (!list.length) return false;
  const i = list.indexOf(document.activeElement);
  const next = list[i < 0 ? (by > 0 ? 0 : list.length - 1) : Math.min(list.length - 1, Math.max(0, i + by))];
  if (!next || next === document.activeElement) return false;
  next.focus();
  next.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  return true;
}

/** `back()` for Esc; in the big-screen view `on` has `page(by)` for the shoulder buttons,
 *  `search()` for Y, `options()` for ☰ and `choose(select)` for a list. */
export function keys(back, on = {}) {
  // what was used last: after a touch or a click nothing is focused for it (big.js), and the
  // big screen's focus is only drawn for keys and controllers
  const root = document.documentElement;
  addEventListener("pointerdown", () => { root.dataset.input = "pointer"; }, true);
  addEventListener("keydown", () => { root.dataset.input = "keys"; }, true);
  addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const t = e.target;
    const arrow = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
    const dialog = !!document.querySelector(".dialog-backdrop");
    if (arrow && big()) {
      const sideways = e.key === "ArrowLeft" || e.key === "ArrowRight";
      if (sideways && (typing(t) || t?.matches?.('input[type="range"]'))) return;
      if ((!sideways && scrolls(t, arrow)) || moveTo(e.key.slice(5).toLowerCase())) e.preventDefault();
    } else if (arrow) {
      const sideways = e.key === "ArrowLeft" || e.key === "ArrowRight";
      if (sideways && (typing(t) || t?.matches?.('select, input[type="range"]'))) return;
      if (move(arrow)) e.preventDefault();
    } else if (big() && e.key === "Enter" && t?.matches?.("select") && on.choose) {
      e.preventDefault();
      on.choose(t);
    } else if (big() && (e.key === "PageUp" || e.key === "PageDown") && !dialog && on.page) {
      e.preventDefault();
      on.page(e.key === "PageUp" ? -1 : 1);
    } else if (big() && (e.key === "y" || e.key === "Y") && !typing(t) && !dialog && on.search) {
      e.preventDefault();
      on.search();
    } else if (big() && e.key === "ContextMenu" && !dialog && on.options) {
      e.preventDefault();
      on.options();
    } else if (e.key === "Enter" && t?.matches?.('input[type="checkbox"], input[type="radio"]')) {
      e.preventDefault();
      t.click();
    } else if ((e.key === "Escape" || (e.key === "Backspace" && !typing(t))) && !document.querySelector(".dialog-backdrop")) {
      e.preventDefault();
      back();
    }
  });
}

/** Controllers on Windows and Linux (the Gamepad API's standard layout): the D-pad and
 *  left stick as the arrows, A as Enter, B as Back, X selects a game (adds it to the queue in
 *  the big-screen view), Y searches, ☰ (Start) opens Options, the shoulder buttons a page up
 *  or down (the page before or after in the big-screen view). Held arrows repeat. In Game
 *  Mode, A on a text box brings up Steam's on-screen keyboard (D107). */
export function gamepads() {
  if (!navigator.getGamepads) return;
  let running = false;
  const was = {}, again = {};
  const press = (key) => {
    const t = document.activeElement && document.activeElement !== document.body ? document.activeElement : document.getElementById("go-root");
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
    t?.dispatchEvent(e);
    // a key made here presses nothing itself: Enter clicks what's focused
    if (key === "Enter" && !e.defaultPrevented && !typing(t)) t?.click?.();
    else if (key === "Enter" && typing(t) && info.game_mode) keyboard();
    if (key === "Escape" && !e.defaultPrevented) document.querySelector(".dialog-backdrop .dialog-head button")?.click();
  };
  const frame = (now) => {
    const pads = [...navigator.getGamepads()].filter(Boolean);
    if (!pads.length) { running = false; return; }
    const held = {};
    for (const p of pads) {
      const b = (i) => !!p.buttons[i]?.pressed;
      const [x = 0, y = 0] = p.axes;
      held.ArrowUp ||= b(12) || y < -0.6;
      held.ArrowDown ||= b(13) || y > 0.6;
      held.ArrowLeft ||= b(14) || x < -0.6;
      held.ArrowRight ||= b(15) || x > 0.6;
      held.Enter ||= b(0);
      held.Escape ||= b(1);
      held.x ||= b(2);
      held.y ||= b(3);
      held.ContextMenu ||= b(9);
      held.PageUp ||= b(4);
      held.PageDown ||= b(5);
    }
    for (const [key, on] of Object.entries(held)) {
      if (on && !was[key]) { press(key); again[key] = now + 400; }
      else if (on && key.startsWith("Arrow") && now >= again[key]) { press(key); again[key] = now + 90; }
      was[key] = on;
    }
    requestAnimationFrame(frame);
  };
  addEventListener("gamepadconnected", () => {
    if (!running) { running = true; requestAnimationFrame(frame); }
  });
}
