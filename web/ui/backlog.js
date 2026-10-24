// The backlog (docs/PLAN.md, D157–D159): the games you mean to play, in the order you'll
// play them, beside the ones you've completed. It's under Collections in the menu (and at
// the top of the collections' ribbon); one to a library, kept in `_library/backlog.json`,
// and it takes PC games too. A game is dragged to its place (or moved with its menu, or
// Alt+↑ and Alt+↓), marked In progress (highlighted, as many as you like) and Completed:
// to the list beside, the newest first, with the date. Dragged back, or *Back to the
// backlog*, it's put where it's dropped (else at the bottom). Every change has Undo, and a
// game opened from either list steps through it with Previous and Next.
import { html, useState, useEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { routeHash } from "./context.js";
import { api, toast, undoable, plural } from "./library.js";
import { CommandBar } from "./layout.js";
import { usePageKeys, selectAllKey, openMenu } from "./actions.js";
import { Cover, opened, gameHash } from "./browse.js";
import { useSidePanel, argHash } from "./panels.js";
import { Dialog, close } from "./dialogs.js";
import { askValue } from "./shelf.js";
import { sourceText } from "./collections.js";
import { Icon } from "./icons.js";

const err = (e) => e.message || String(e);
/** 22 October 2026 (the year left out when it's this one). */
export const dayText = (iso) => {
  const d = new Date(iso);
  if (isNaN(d)) return "";
  return d.toLocaleDateString(undefined, { day: "numeric", month: "long", ...(d.getFullYear() === new Date().getFullYear() ? {} : { year: "numeric" }) });
};
/** Further down than any list is long: the bottom. */
const BOTTOM = 1e9;
const changed = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
/** "Zelda", or "3 games". */
const named = (games) => (games.length === 1 ? games[0].title || "it" : plural(games.length, "game", "games"));

/** The backlog put back as it was before a change (`before`, from the change). */
async function restore(before) {
  const r = await api("backlog_restore", { backlog: before });
  changed();
  return r;
}

/** A change to the backlog (`cmd` with `args`), said with Undo; resolves with the backlog. */
async function change(cmd, args, said) {
  const r = await api(cmd, args);
  changed();
  if (said) undoable(said, () => restore(r.before));
  return r;
}

/** Games ({ key, title, platform }) added to the backlog's list to play, at its bottom (or
 *  `top`), with Undo. */
export async function addToBacklog(games, { top = false } = {}) {
  const r = await api("backlog_add", { games: games.map((g) => ({ key: g.key, title: g.title || "", platform: g.platform || "" })), top });
  changed();
  if (!r.added) {
    toast(games.length === 1 ? `${games[0].title || "It"} is in the backlog already.` : "They're in the backlog already.", 5000);
    return r;
  }
  const n = r.added === games.length ? named(games) : plural(r.added, "game", "games");
  undoable(`Added ${n} to the backlog.`, () => restore(r.before), 8000, [{ label: "Open", run: () => { location.hash = routeHash("backlog"); } }]);
  return r;
}

/** In progress or not, completed, back to the list to play, or out of the backlog: one
 *  game's (a game's page), with Undo. */
export const backlogDo = {
  playing: (g, on) => change("backlog_playing", { keys: [g.key], on }, on ? `Marked ${g.title} in progress.` : `Marked ${g.title} not in progress.`),
  complete: (g) => change("backlog_complete", { keys: [g.key] }, `Marked ${g.title} completed.`),
  again: (g) => change("backlog_move", { keys: [g.key], to: BOTTOM }, `Put ${g.title} back in the backlog.`),
  remove: (g) => change("backlog_remove", { keys: [g.key] }, `Removed ${g.title} from the backlog.`),
};

// ------------------------------------------------------------ adding games

/** Add games…: found by their titles (the games you have, or every game in the DATs), several at once. */
function AddDialog({ has = [] }) {
  const [q, setQ] = useState("");
  const [every, setEvery] = useState(false);
  const [found, setFound] = useState(null);
  const [picked, setPicked] = useState(() => new Map());
  const [top, setTop] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (q.trim().length < 2) { setFound(null); return; }
    let gone = false;
    const t = setTimeout(() => api("browse_query", { query: { search: q, all: every, one_per_title: true, limit: 60 } })
      .then((r) => { if (!gone) setFound(r.items || []); }).catch((e) => { if (!gone) setFound({ error: err(e) }); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [q, every]);
  const toggle = (g) => setPicked((m) => { const n = new Map(m); n.has(g.key) ? n.delete(g.key) : n.set(g.key, g); return n; });
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await addToBacklog([...picked.values()], { top });
      close();
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  const list = Array.isArray(found) ? found : [];
  return html`<${Dialog} title="Add games to the backlog" id="backlog-add-dialog" wide=${true} onSubmit=${submit} busy=${busy || !picked.size} error=${error} submitLabel=${picked.size ? `Add ${plural(picked.size, "game", "games")}` : "Add"}>
    <div class="pick-row">
      <input type="search" id="backlog-add-search" class="compact" placeholder="Search by title" aria-label="Search by title" value=${q} onInput=${(e) => setQ(e.target.value)} />
      <label class="check-row"><input type="checkbox" id="backlog-add-every" checked=${every} onChange=${(e) => setEvery(e.target.checked)} /> Every game in the DATs, not only those you have</label>
    </div>
    ${found?.error ? html`<p class="warn-note" role="alert">${found.error}</p>` : null}
    ${found === null ? html`<p class="muted">Type two letters or more of a title.</p>` : null}
    <ul class="ls-list organise-list backlog-add-list" id="backlog-add-list">${list.map((g) => html`<li key=${g.key} data-key=${g.key}>
      <label class="check-row"><input type="checkbox" checked=${picked.has(g.key)} disabled=${has.includes(g.key)} onChange=${() => toggle(g)} /> ${g.title}</label>
      <span class="muted">${[g.platform_name, g.year, has.includes(g.key) ? "in the backlog" : null].filter(Boolean).join(" · ")}</span></li>`)}</ul>
    ${Array.isArray(found) && !found.length ? html`<p class="muted">No game matches.</p>` : null}
    ${picked.size ? html`<p class="muted" id="backlog-add-picked">${[...picked.values()].map((g) => g.title).join(", ")}</p>` : null}
    <label class="check-row"><input type="checkbox" id="backlog-add-top" checked=${top} onChange=${(e) => setTop(e.target.checked)} /> At the top of the list (they're added at the bottom otherwise)</label>
  <//>`;
}

export function BacklogDialogs({ dialog }) {
  if (dialog.type === "backlog-add") return html`<${AddDialog} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ the page

export function Backlog() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const [data, setData] = useState(null);
  const [colls, setColls] = useState(null);
  // what's picked: in one list at a time
  const [sel, setSel] = useState({ list: "todo", keys: [], anchor: -1 });
  const [drag, setDrag] = useState(null);
  const todoRef = useRef(null);
  const doneRef = useRef(null);
  const dragging = useRef(null);
  const noClick = useRef(false);
  const cameBack = useRef(false);

  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("backlog").then((d) => { if (!gone) setData(d); }).catch((e) => { if (!gone) setData({ error: err(e) }); });
    api("collections").then((cs) => { if (!gone) setColls(cs); }).catch(() => { if (!gone) setColls([]); });
    return () => { gone = true; };
  }, [s.library?.path, s.changes]);

  const lists = { todo: data?.games || [], done: data?.done || [] };
  // (the picked that are still in their list, in its order)
  const keysIn = (k, keys) => lists[k].filter((g) => keys.includes(g.key)).map((g) => g.key);
  const picked = keysIn(sel.list, sel.keys);
  const clear = () => setSel({ list: "todo", keys: [], anchor: -1 });

  // back from a game's page: the game last shown is picked and in view
  useEffect(() => {
    if (cameBack.current || !data?.games || !opened.keys || !opened.key) return;
    cameBack.current = true;
    const k = lists.todo.some((g) => g.key === opened.key) ? "todo" : lists.done.some((g) => g.key === opened.key) ? "done" : null;
    if (!k) return;
    setSel({ list: k, keys: [opened.key], anchor: lists[k].findIndex((g) => g.key === opened.key) });
    requestAnimationFrame(() => document.querySelector(`.bl-row[data-key="${CSS.escape(opened.key)}"]`)?.scrollIntoView({ block: "nearest" }));
  }, [data]);

  const apply = (p) => p.then((r) => { setData(r); return r; }).catch((e) => toast(err(e), 8000));
  const gamesOf = (k, keys) => lists[k].filter((g) => keys.includes(g.key));
  const move = (keys, to, from = "todo") => {
    const gs = gamesOf(from, keys);
    return apply(change("backlog_move", { keys, to }, from === "done" ? `Put ${named(gs)} back in the backlog.`
      : to >= BOTTOM ? `Moved ${named(gs)} to the bottom.` : `Moved ${named(gs)} to place ${to + 1}.`));
  };
  const setPlaying = (keys, on) => apply(change("backlog_playing", { keys, on }, `Marked ${named(gamesOf("todo", keys))} ${on ? "in progress" : "not in progress"}.`));
  const complete = (keys) => { clear(); return apply(change("backlog_complete", { keys }, `Marked ${named(gamesOf("todo", keys))} completed.`)); };
  const remove = (k, keys) => { clear(); return apply(change("backlog_remove", { keys }, `Removed ${named(gamesOf(k, keys))} from the backlog.`)); };
  /** Up or down one place, the picked together. */
  const step = (keys, by) => {
    const first = lists.todo.findIndex((g) => keys.includes(g.key));
    if (first < 0) return;
    const to = Math.max(0, first + by);
    if (to === first && keys.length === 1) return;
    return apply(change("backlog_move", { keys, to }, `Moved ${named(gamesOf("todo", keys))} to place ${to + 1}.`));
  };
  const openGame = (k, i) => {
    const l = lists[k];
    if (!l[i]) return;
    Object.assign(opened, { query: null, keys: l.map((g) => g.key), index: i, total: l.length, key: l[i].key, name: k === "done" ? "Completed" : "the backlog" });
    location.hash = gameHash(l[i].key);
  };

  /** What can be done to the games picked in a list (the bar, the right-click menu). */
  const actionsFor = (k, keys) => {
    const gs = gamesOf(k, keys);
    const at = lists[k].findIndex((g) => g.key === gs[0]?.key);
    const last = lists[k].findIndex((g) => g.key === gs[gs.length - 1]?.key);
    const one = gs.length === 1;
    if (k === "done") return [
      { id: "bl-open", label: "Open", icon: "external", one: true, run: () => openGame(k, at) },
      { id: "bl-again", label: "Back to the backlog", icon: "refresh", disabled: off, run: () => move(keys, BOTTOM, "done") },
      { sep: true },
      { id: "bl-remove", label: "Remove from the backlog", icon: "trash", disabled: off, run: () => remove(k, keys) },
    ];
    const playing = gs.length > 0 && gs.every((g) => g.started);
    const n = lists.todo.length;
    return [
      { id: "bl-open", label: "Open", icon: "external", one: true, run: () => openGame(k, at) },
      { id: "bl-playing", label: playing ? "Not in progress" : "In progress", icon: playing ? "close" : "gamepad", disabled: off, run: () => setPlaying(keys, !playing) },
      { id: "bl-complete", label: "Completed", icon: "check", disabled: off, run: () => complete(keys) },
      { sep: true },
      { id: "bl-top", label: "Move to the top", disabled: off || (one && at === 0), run: () => move(keys, 0) },
      { id: "bl-up", label: "Move up", disabled: off || (one && at === 0), run: () => step(keys, -1) },
      { id: "bl-down", label: "Move down", disabled: off || (one && last === n - 1), run: () => step(keys, 1) },
      { id: "bl-bottom", label: "Move to the bottom", disabled: off || (one && last === n - 1), run: () => move(keys, BOTTOM) },
      { id: "bl-place", label: "Move to place…", disabled: off, run: () => askValue({
        title: `Move ${named(gs)} to a place`, label: `The place, 1 (the top) to ${n}`, value: String(at + 1), button: "Move",
        run: async (v) => {
          const p = parseInt(v, 10);
          if (!(p >= 1)) throw new Error("Give a number: 1 is the top.");
          await move(keys, p - 1);
        },
      }) },
      { sep: true },
      { id: "bl-remove", label: "Remove from the backlog", icon: "trash", disabled: off, run: () => remove(k, keys) },
    ];
  };

  const pick = (e, k, g, i) => {
    if (e.shiftKey && sel.list === k && sel.anchor >= 0) {
      const [a, b] = sel.anchor < i ? [sel.anchor, i] : [i, sel.anchor];
      const range = lists[k].slice(a, b + 1).map((x) => x.key);
      setSel({ list: k, keys: e.ctrlKey || e.metaKey ? [...new Set([...sel.keys, ...range])] : range, anchor: sel.anchor });
      return;
    }
    if ((e.ctrlKey || e.metaKey) && sel.list === k) {
      setSel({ list: k, keys: sel.keys.includes(g.key) ? sel.keys.filter((x) => x !== g.key) : [...sel.keys, g.key], anchor: i });
      return;
    }
    setSel({ list: k, keys: [g.key], anchor: i });
  };
  const onClick = (e, k, g, i) => {
    if (noClick.current) return;
    pick(e, k, g, i);
  };
  const onMenu = (e, k, g, i) => {
    e.preventDefault();
    let keys = picked;
    if (sel.list !== k || !sel.keys.includes(g.key)) { keys = [g.key]; setSel({ list: k, keys, anchor: i }); }
    const items = actionsFor(k, keys).filter((a) => keys.length === 1 || !a.one);
    openMenu(e, items);
  };

  // dragging: a game (or the games picked with it) to its place in the list to play, from
  // there to Completed, or from Completed back to the list where it's dropped
  const target = (x, y, keys) => {
    for (const [k, ref] of [["todo", todoRef], ["done", doneRef]]) {
      const el = ref.current;
      if (!el) continue;
      const r = el.getBoundingClientRect();
      if (x < r.left || x > r.right || y < r.top - 24 || y > r.bottom + 24) continue;
      // near its edges, the list scrolls
      if (y < r.top + 36) el.scrollTop -= 14;
      else if (y > r.bottom - 36) el.scrollTop += 14;
      if (k === "done") return { list: "done", index: 0 };
      const rows = [...el.querySelectorAll(".bl-row")].filter((row) => !keys.includes(row.dataset.key));
      let index = rows.length;
      for (let j = 0; j < rows.length; j++) {
        const b = rows[j].getBoundingClientRect();
        if (y < b.top + b.height / 2) { index = j; break; }
      }
      return { list: "todo", index };
    }
    return null;
  };
  const onDown = (e, k, g, i) => {
    if (off || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.target.closest("button")) return;
    const start = { x: e.clientX, y: e.clientY };
    const keys = sel.list === k && sel.keys.includes(g.key) ? keysIn(k, sel.keys) : [g.key];
    const d = { from: k, keys, active: false, to: null, title: keys.length === 1 ? g.title : `${g.title} and ${plural(keys.length - 1, "other", "others")}` };
    dragging.current = d;
    const moveTo = (ev) => {
      if (!d.active) {
        if (Math.abs(ev.clientX - start.x) + Math.abs(ev.clientY - start.y) < 6) return;
        d.active = true;
        if (!(sel.list === k && sel.keys.includes(g.key))) setSel({ list: k, keys: [g.key], anchor: i });
      }
      ev.preventDefault();
      d.to = target(ev.clientX, ev.clientY, d.keys);
      setDrag({ ...d, x: ev.clientX, y: ev.clientY });
    };
    const stop = (ev) => {
      removeEventListener("pointermove", moveTo);
      removeEventListener("pointerup", stop);
      removeEventListener("pointercancel", stop);
      dragging.current = null;
      if (!d.active) return;
      setDrag(null);
      // (the click the release makes isn't a pick)
      noClick.current = true;
      setTimeout(() => { noClick.current = false; }, 0);
      const t = ev.type === "pointerup" ? d.to : null;
      if (!t) return;
      if (t.list === "done" && d.from === "todo") complete(d.keys);
      else if (t.list === "todo") {
        // (dropped where it was: nothing to do)
        const now = lists.todo.filter((x) => !d.keys.includes(x.key));
        const was = lists.todo.findIndex((x) => x.key === d.keys[0]);
        if (d.from === "todo" && was === t.index && d.keys.every((key, j) => lists.todo[was + j]?.key === key)) return;
        move(d.keys, Math.min(t.index, now.length), d.from);
      }
    };
    addEventListener("pointermove", moveTo);
    addEventListener("pointerup", stop);
    addEventListener("pointercancel", stop);
  };

  usePageKeys((e) => {
    const k = sel.list;
    if (e.key === "Escape" && picked.length) { clear(); return; }
    if (selectAllKey(e)) { e.preventDefault(); setSel({ list: k, keys: lists[k].map((g) => g.key), anchor: 0 }); return; }
    if (!picked.length) return;
    const at = lists[k].findIndex((g) => g.key === picked[0]);
    if (e.key === "Enter" && picked.length === 1) { openGame(k, at); return; }
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown") && k === "todo" && !off) { e.preventDefault(); step(picked, e.key === "ArrowUp" ? -1 : 1); return; }
    if (!e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      const i = Math.max(0, Math.min(lists[k].length - 1, at + (e.key === "ArrowUp" ? -1 : 1)));
      setSel({ list: k, keys: [lists[k][i].key], anchor: i });
      document.querySelector(`.bl-row[data-key="${CSS.escape(lists[k][i].key)}"]`)?.scrollIntoView({ block: "nearest" });
      return;
    }
    if (e.key === "Delete" && !off) { remove(k, picked); return; }
    if (e.key === "ContextMenu") {
      const el = document.querySelector(".bl-row.selected");
      if (el) { const r = el.getBoundingClientRect(); openMenu({ clientX: r.left + 40, clientY: r.top + 20 }, actionsFor(k, picked).filter((a) => picked.length === 1 || !a.one)); }
    }
  });

  // the collections' ribbon, the backlog at its top
  useSidePanel(Array.isArray(colls) ? {
    id: "collection-panel", route: "backlog", title: "Collections", icon: "collection", count: colls.length, current: "backlog",
    top: [{ id: "backlog", label: "Backlog", icon: "list", href: routeHash("backlog"), count: data?.games?.length }],
    items: colls.map((x) => ({ id: x.id, label: x.name, sub: sourceText(x.made_from, x.rules), href: argHash("collections", x.id) })),
    actions: [{ id: "collection-panel-new", label: "New collection…", icon: "plus", disabled: off, run: () => ui.set({ dialog: { type: "collection-new", way: "rules", onMade: (made) => { location.hash = argHash("collections", made.id); } } }) }],
  } : null);

  const n = lists.todo.length, done = lists.done.length, playing = data?.playing || 0;
  const count = data && !data.error ? [`${n} to play`, playing ? `${playing} in progress` : null, `${done} completed`].filter(Boolean).join(" · ") : null;
  const add = () => ui.set({ dialog: { type: "backlog-add", has: lists.todo.map((g) => g.key) } });
  const head = html`<${CommandBar} id="backlog-head" title=${html`<h1 id="backlog-title">Backlog</h1>`} count=${count}
    primary=${html`<button type="button" class="primary compact" id="backlog-add" disabled=${off} onClick=${add}>${Icon.plus(15)} Add games…</button>`}
    selection=${picked.length ? { count: `${picked.length} selected`, onClear: clear, items: () => actionsFor(sel.list, picked).filter((a) => picked.length === 1 || !a.one), buttons: 3 } : null} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (!data) return html`${head}<div class="pages"><p class="muted">Reading the backlog…</p></div>`;
  if (data.error) return html`${head}<div class="pages"><p class="warn-note" role="alert" id="backlog-error">${data.error}</p></div>`;

  const row = (k, g, i) => {
    const on = sel.list === k && sel.keys.includes(g.key);
    const sub = k === "todo"
      ? [g.platform_name, g.year, g.missing ? "not in the library any more" : g.kind === "dat" && !g.have ? "you don't have it" : null]
      : [g.platform_name, g.completed ? `completed ${dayText(g.completed)}` : null];
    const moving = drag && drag.keys.includes(g.key);
    return html`<li class=${`bl-row${on ? " selected" : ""}${moving ? " moving" : ""}`} key=${g.key} data-key=${g.key} data-playing=${k === "todo" && g.started ? "1" : null} data-missing=${g.missing ? "1" : null}
      role="option" aria-selected=${on ? "true" : "false"} onPointerDown=${(e) => onDown(e, k, g, i)} onClick=${(e) => onClick(e, k, g, i)} onDblClick=${() => openGame(k, i)} onContextMenu=${(e) => onMenu(e, k, g, i)}>
      ${k === "todo" ? html`<span class="bl-grip" aria-hidden="true" title="Drag to its place">⠿</span><span class="bl-rank">${i + 1}</span>` : null}
      <${Cover} g=${g} px=${128} cls="bl-cover" />
      <span class="bl-text"><span class="bl-title" title=${g.title}>${g.title}</span><span class="bl-sub">${sub.filter(Boolean).join(" · ")}</span></span>
      ${k === "todo" && g.started ? html`<span class="bl-badge" title=${`In progress since ${dayText(g.started)}`}>In progress</span>` : null}
      <span class="bl-buttons">${k === "todo" ? html`
        <button type="button" class="ghost compact" data-action="playing" aria-pressed=${g.started ? "true" : "false"} disabled=${off} title=${g.started ? "Not in progress" : "In progress"} aria-label=${g.started ? `${g.title}: not in progress` : `${g.title}: in progress`} onClick=${(e) => { e.stopPropagation(); setPlaying([g.key], !g.started); }}>${Icon.gamepad(15)}</button>
        <button type="button" class="ghost compact" data-action="complete" disabled=${off} title="Completed" aria-label=${`${g.title}: completed`} onClick=${(e) => { e.stopPropagation(); complete([g.key]); }}>${Icon.check(15)}</button>`
        : html`<button type="button" class="ghost compact" data-action="again" disabled=${off} title="Back to the backlog" aria-label=${`${g.title}: back to the backlog`} onClick=${(e) => { e.stopPropagation(); move([g.key], BOTTOM, "done"); }}>${Icon.refresh(15)}</button>`}</span>
    </li>`;
  };
  // the list to play, with where a dragged game would go
  const todoRows = [];
  let seen = 0;
  const dropAt = drag?.to?.list === "todo" ? drag.to.index : -1;
  lists.todo.forEach((g, i) => {
    const moving = drag && drag.keys.includes(g.key);
    if (!moving && seen === dropAt) todoRows.push(html`<li class="bl-drop" key="drop" aria-hidden="true"></li>`);
    if (!moving) seen++;
    todoRows.push(row("todo", g, i));
  });
  if (dropAt >= seen && dropAt >= 0) todoRows.push(html`<li class="bl-drop" key="drop" aria-hidden="true"></li>`);
  const background = (e) => { if (e.target === e.currentTarget) clear(); };
  return html`${head}<div class=${`backlog-page${drag ? " dragging" : ""}`} id="backlog-page">
    <section class="bl-col" id="backlog-todo" aria-label="To play">
      <header class="bl-head"><h2>To play</h2><span class="nav-count" id="backlog-todo-count">${n}</span>
        <span class="muted bl-note">The first is the one to play next. Drag a game to its place, or to Completed.</span></header>
      <ol class="bl-list" id="backlog-todo-list" ref=${todoRef} role="listbox" aria-multiselectable="true" aria-label="To play" onClick=${background}>
        ${todoRows}
        ${!n ? html`<li class="bl-empty" id="backlog-empty">
          <p>No games to play yet. Add them with <b>Add games…</b>, or pick games in the Library (or open a game's page) and choose <b>Add to the backlog</b>.</p>
          <div class="home-actions"><a class="button ghost" href=${routeHash("library")} id="backlog-to-library">Open the Library</a></div></li>` : null}
      </ol>
    </section>
    <section class=${`bl-col bl-done${drag?.to?.list === "done" && drag.from === "todo" ? " drop-on" : ""}`} id="backlog-done" aria-label="Completed" data-empty=${done ? "no" : "yes"}>
      <header class="bl-head"><h2>Completed</h2><span class="nav-count" id="backlog-done-count">${done}</span></header>
      <ol class="bl-list" id="backlog-done-list" ref=${doneRef} role="listbox" aria-multiselectable="true" aria-label="Completed" onClick=${background}>
        ${lists.done.map((g, i) => row("done", g, i))}
        ${!done ? html`<li class="bl-empty" id="backlog-done-empty"><p class="muted">The games you complete go here, the newest first.</p></li>` : null}
      </ol>
    </section>
    ${drag ? html`<div class="bl-ghost" style=${`left:${drag.x + 14}px;top:${drag.y + 10}px`} aria-hidden="true">${drag.title}</div>` : null}
  </div>`;
}
