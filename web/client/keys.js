// Every control reached with the arrow keys and Enter, in a set order (D103), so a
// controller moves through the pages as a mouse would: Down and Right go to the next
// control, Up and Left to the one before (in a text box Left and Right move in the text,
// on a list they change it); Enter presses, and ticks a box; Esc or Backspace goes back.
// A part that moves itself (the Library's games, by rows and columns) says so with
// data-keys="own" and leaves the key alone at its edges. On Windows the page reads
// controllers itself (the Gamepad API); Android's plugin turns them into keys (part 3).

const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]';

const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";

/** The controls in their order: an open dialog's, else the page's (the menu, then the page).
 *  Within a part that moves itself, only its current control (tabindex 0) is a stop. */
export function controls() {
  const scope = document.querySelector(".dialog-backdrop .dialog") || document.getElementById("go-root") || document.body;
  return [...scope.querySelectorAll(FOCUSABLE)].filter((el) => el.tabIndex >= 0 && shown(el));
}

const typing = (t) => t?.matches?.('input:not([type="checkbox"]):not([type="radio"]):not([type="range"]), textarea');

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

export function keys(back) {
  addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
    const t = e.target;
    const arrow = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[e.key];
    if (arrow) {
      const sideways = e.key === "ArrowLeft" || e.key === "ArrowRight";
      if (sideways && (typing(t) || t?.matches?.('select, input[type="range"]'))) return;
      if (move(arrow)) e.preventDefault();
    } else if (e.key === "Enter" && t?.matches?.('input[type="checkbox"], input[type="radio"]')) {
      e.preventDefault();
      t.click();
    } else if ((e.key === "Escape" || (e.key === "Backspace" && !typing(t))) && !document.querySelector(".dialog-backdrop")) {
      e.preventDefault();
      back();
    }
  });
}

/** Controllers on Windows (the Gamepad API's standard layout): the D-pad and left stick
 *  as the arrows, A as Enter, B as Back, X ticks a game, the shoulder buttons a page up
 *  or down. Held arrows repeat. */
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
