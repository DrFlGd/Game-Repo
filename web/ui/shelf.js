// The shelf (docs/PLAN.md, D152): Videos and Magazines laid out as the Library is. The
// filters go down the left, each value with how many it has under the other filters, and
// the filters in use show as chips. The bar has a search, Grid, List or Table, Sort and its
// reverse, and Group by; the panel on the right is for what's picked. A click picks one,
// Ctrl and Shift add to it, Ctrl+A picks every one shown, Esc clears, Enter or a double
// click opens, and a right click has the menu. Every item is read at once (there are
// hundreds, not the Library's many thousands), so it's all done here. What an item shows,
// and what can be done with those picked, the page says. The view, sort, grouping,
// filters and search are kept for each page (`gr-shelf-<page>`), so coming back to it is
// where it was left (D150).
import { html, useState, useEffect, useMemo, useRef, useLayoutEffect } from "../lib/html.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { api, plural } from "./library.js";
import { CommandBar, ViewSwitch, SortMenu, MenuButton } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { usePageKeys, selectAllKey, openMenu } from "./actions.js";
import { Values, VIEWS, SIZES } from "./browse.js";
import { Icon } from "./icons.js";

const prefs = () => ctx.platform?.store?.prefs;
const plain = (s) => String(s ?? "").normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
export const byName = (a, b) => String(a ?? "").localeCompare(String(b ?? ""), undefined, { numeric: true, sensitivity: "base" });

/** A page's shelf settings, kept: `[state, set]`. `defaults`: its own view, sort and group. */
export function useShelfState(page, defaults = {}) {
  const key = `gr-shelf-${page}`;
  const [st, setSt] = useState(() => {
    let saved = {};
    try { saved = prefs()?.get(key, {}) || {}; } catch { /* none kept */ }
    return { search: "", view: "grid", size: "medium", sort: "", desc: false, group: "", f: {}, ...defaults, ...saved };
  });
  const set = (patch) => setSt((s) => {
    const next = { ...s, ...patch };
    try { prefs()?.set(key, next); } catch { /* not kept */ }
    return next;
  });
  return [st, set];
}

/** A picture, else the next, else `none` ({ text, colour, icon }) drawn instead. */
export function Pic({ srcs, none, cls = "lib-cover", px = 28 }) {
  const list = (srcs || []).filter(Boolean);
  const k = list.join("|");
  const [bad, setBad] = useState({ k, n: 0 });
  const n = bad.k === k ? bad.n : 0;
  if (n < list.length) return html`<img class=${cls} src=${list[n]} alt="" loading="lazy" decoding="async" draggable="false" onError=${() => setBad({ k, n: n + 1 })} />`;
  return html`<span class=${`${cls} lib-cover-none`} style=${`background:${none?.colour || "var(--graphite)"}`} aria-hidden="true">${none?.icon && Icon[none.icon] ? Icon[none.icon](px) : (none?.text || "?").slice(0, 3)}</span>`;
}

/** Values in the order a field wants: "count" (most first), "name", "newest" (the highest first), or a compare. */
function ordered(list, order = "count") {
  if (typeof order === "function") return list.sort(order);
  if (order === "name") return list.sort((a, b) => byName(a.name, b.name));
  if (order === "newest") return list.sort((a, b) => byName(b.value, a.value));
  return list.sort((a, b) => b.count - a.count || byName(a.name, b.name));
}

const values = (f, item) => [...new Set((f.of(item) || []).filter((v) => v != null && v !== "").map(String))];

/**
 * The shelf. `items`: every item ({ id, … }); null while reading. `fields`: the filters, [{ id,
 * name, of(item) → its values, label(value), long, order, groupOrder, none, hidden (only to group by) }]. `groups`:
 * [[field id or "", label]]. `sorts`: [[id, label, compare]]. `text(item)`: the words a search
 * looks in. `tile(item)` and `line(item)`: { pics, none, badge: { text, colour }, title, meta,
 * missing }. `columns`: [[id, label, value(item), sort id]] with `cols` their widths.
 * `actions(picked)`: menu items ({ one: true } for one only). `panel(item)`: the panel for one.
 * `onOpen(item)`; `onPicked(item)`, the one picked (or null). `groupMenu({ value, name, items })`: a
 * group heading's menu, or null.
 */
export function Shelf({ page, noun, title, items, error, st, set, fields, groups, sorts, views = VIEWS, text, tile, line = tile, columns = [], cols = "", ratio = "1",
  actions, panel, onOpen, onPicked, groupMenu, switches = null, top = null, primary = null, more = () => [], moreTitle = "More actions", empty = null, none = null }) {
  const scrollRef = useRef(null);
  const [sel, setSel] = useState(() => new Set());
  const [anchor, setAnchor] = useState(-1);
  const sort = sorts.some(([k]) => k === st.sort) ? st.sort : sorts[0][0];
  const group = groups.some(([k]) => k === st.group) ? st.group : groups[0][0];
  const view = views.some(([k]) => k === st.view) ? st.view : views[0][0];
  const all = items || [];
  const vals = useMemo(() => new Map(all.map((i) => [i.id, Object.fromEntries(fields.map((f) => [f.id, values(f, i)]))])), [items]);
  const hay = useMemo(() => new Map(all.map((i) => [i.id, plain(text(i))])), [items]);
  const f = st.f || {};
  const words = plain(st.search).split(/\s+/).filter(Boolean);
  const passes = (i, skip) => fields.every((x) => x.id === skip || !f[x.id]?.length || vals.get(i.id)[x.id].some((v) => f[x.id].includes(v)));
  const searched = words.length ? all.filter((i) => words.every((w) => hay.get(i.id).includes(w))) : all;
  const shown = searched.filter((i) => passes(i, null));
  const facets = Object.fromEntries(fields.map((x) => {
    const n = new Map();
    for (const i of searched) if (passes(i, x.id)) for (const v of vals.get(i.id)[x.id]) n.set(v, (n.get(v) || 0) + 1);
    return [x.id, ordered([...n].map(([value, count]) => ({ value, name: x.label ? x.label(value) : value, count })), x.order)];
  }));
  // the groups, each in the order asked; an item in several (its tags, its games) is in each
  const cmp = sorts.find(([k]) => k === sort)[2];
  const inOrder = [...shown].sort((a, b) => (st.desc ? -1 : 1) * cmp(a, b) || byName(a.id, b.id));
  const gField = fields.find((x) => x.id === group);
  let sections = [{ value: null, name: null, items: inOrder }];
  if (gField) {
    const m = new Map();
    const without = [];
    for (const i of inOrder) {
      const vs = vals.get(i.id)[gField.id];
      if (!vs.length) without.push(i);
      for (const v of vs) { if (!m.has(v)) m.set(v, []); m.get(v).push(i); }
    }
    sections = ordered([...m].map(([value, list]) => ({ value, name: gField.label ? gField.label(value) : value, count: list.length, items: list })), gField.groupOrder || "name");
    if (without.length) sections.push({ value: "", name: gField.none || `No ${gField.name.toLowerCase()}`, count: without.length, items: without });
  }
  const flat = sections.flatMap((s) => s.items);
  const picked = shown.filter((i) => sel.has(i.id));
  const one = picked.length === 1 ? picked[0] : null;
  useEffect(() => { onPicked?.(one); }, [one]);
  const clear = () => { setSel(new Set()); setAnchor(-1); };
  // a filter, the search or the order changed: what's picked goes, and the list is at its top
  const change = (patch) => {
    set(patch);
    clear();
    scrollRef.current?.scrollTo({ top: 0 });
  };
  const toggle = (id) => (v) => change({ f: { ...f, [id]: (f[id] || []).includes(v) ? f[id].filter((x) => x !== v) : [...(f[id] || []), v] } });
  const pick = (e, item, pos) => {
    if (e.shiftKey && anchor >= 0) {
      const [a, b] = anchor < pos ? [anchor, pos] : [pos, anchor];
      const next = e.ctrlKey || e.metaKey ? new Set(sel) : new Set();
      for (const x of flat.slice(a, b + 1)) next.add(x.id);
      setSel(next);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      const next = new Set(sel);
      next.has(item.id) ? next.delete(item.id) : next.add(item.id);
      setSel(next);
      setAnchor(pos);
      return;
    }
    setSel(new Set([item.id]));
    setAnchor(pos);
  };
  const barItems = (list = picked) => actions(list).filter((a) => list.length === 1 || !a.one);
  const onMenu = (e, item, pos) => {
    e.preventDefault();
    const list = sel.has(item.id) ? picked : [item];
    if (!sel.has(item.id)) { setSel(new Set([item.id])); setAnchor(pos); }
    const items = barItems(list);
    if (items.length) openMenu(e, items);
  };
  usePageKeys((e) => {
    if (e.key === "Escape" && picked.length) { clear(); return; }
    if (selectAllKey(e)) { e.preventDefault(); setSel(new Set(shown.map((i) => i.id))); return; }
    if (e.key === "Enter" && one) { onOpen?.(one); return; }
    if (e.key === "ContextMenu" && picked.length) {
      const el = scrollRef.current?.querySelector(".selected");
      const r = el?.getBoundingClientRect();
      const items = barItems();
      if (r && items.length) openMenu({ clientX: r.left + 20, clientY: r.top + 20 }, items);
    }
  });
  // where the list was scrolled to, for when the page is come back to
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const on = () => { scrolledTo.set(page, el.scrollTop); };
    el.addEventListener("scroll", on, { passive: true });
    return () => el.removeEventListener("scroll", on);
  }, [!!items?.length]);
  const restored = useRef(false);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || restored.current || !items?.length) return;
    restored.current = true;
    if (scrolledTo.get(page)) el.scrollTop = scrolledTo.get(page);
  });
  const p = page;
  const sizeItems = [{ head: "Tile size" }, ...Object.keys(SIZES).map((k) => ({ id: `${p}-size-${k}`, label: k[0].toUpperCase() + k.slice(1), icon: (st.size || "medium") === k ? "check" : null, run: () => set({ size: k }), disabled: view !== "grid" ? "Only in the grid" : false })), { sep: true }];
  const search = html`<input type="search" id=${`${p}-search`} data-search="1" class="compact" placeholder=${`Search the ${noun[1]}`} aria-label=${`Search the ${noun[1]}`} value=${st.search} onInput=${(e) => change({ search: e.target.value })} />`;
  const tools = html`<${ViewSwitch} id=${`${p}-view`} views=${views} value=${view} onChange=${(v) => set({ view: v })} />
    <${SortMenu} id=${`${p}-sort`} options=${sorts.map(([k, l]) => [k, l])} value=${sort} onChange=${(k) => change({ sort: k, desc: false })} />
    <button type="button" class="ghost compact" id=${`${p}-desc`} title=${st.desc ? "Reversed: back to the usual order" : "Reverse the order"} aria-pressed=${st.desc ? "true" : "false"} onClick=${() => change({ desc: !st.desc })}>${st.desc ? "↑" : "↓"}</button>
    <${MenuButton} id=${`${p}-group`} label=${`Group: ${groups.find(([k]) => k === group)?.[1]}`} icon="grouped" items=${() => groups.map(([k, l]) => ({ id: `${p}-group-${k || "none"}`, label: l, icon: group === k ? "check" : null, run: () => change({ group: k }) }))} />`;
  const head = html`<${CommandBar} id=${`${p}-head`} title=${title} count=${items ? plural(shown.length, noun[0], noun[1]) : null} search=${items?.length ? search : null} tools=${items?.length ? tools : null}
    primary=${primary} more=${{ id: `${p}-menu`, title: moreTitle, items: () => [...(items?.length ? sizeItems : []), ...more()] }}
    selection=${picked.length ? { count: `${picked.length.toLocaleString()} selected`, onClear: clear, items: () => barItems(), buttons: 2 } : null} />`;
  if (error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${error}</p></div>`;
  if (!items) return html`${head}<div class="pages"><p class="muted">Reading…</p></div>`;
  if (!items.length && empty) return html`${head}<div class="pages">${empty}</div>`;
  const chips = [];
  for (const x of fields) for (const v of f[x.id] || []) chips.push([`${x.id}-${v}`, `${x.name}: ${x.label ? x.label(v) : v}`, () => toggle(x.id)(v)]);
  if (st.search) chips.push(["search", `Search: ${st.search}`, () => change({ search: "" })]);
  let pos = 0;
  const draw = (item) => {
    const at = pos++;
    const on = sel.has(item.id);
    const ev = { onClick: (e) => pick(e, item, at), onDblClick: () => onOpen?.(item), onContextMenu: (e) => onMenu(e, item, at) };
    if (view === "table") {
      return html`<div role="row" tabindex="-1" key=${item.id} class=${`lib-row shelf-row${on ? " selected" : ""}${item.missing ? " lib-missing" : ""}`} data-id=${item.id} aria-selected=${on ? "true" : "false"} style=${`grid-template-columns:${cols}`} ...${ev}>
        ${columns.map(([k, , v]) => html`<span role="cell" key=${k} class=${`lib-cell ${p}-col-${k}`}>${v(item)}</span>`)}</div>`;
    }
    const t = (view === "grid" ? tile : line)(item);
    if (view === "list") {
      return html`<button type="button" key=${item.id} class=${`lib-line shelf-line${on ? " selected" : ""}${t.missing ? " lib-missing" : ""}`} data-id=${item.id} aria-pressed=${on ? "true" : "false"} ...${ev}>
        <${Pic} srcs=${t.pics} none=${t.none} cls="lib-cover small" px=${18} />
        <span class="lib-line-main"><span class="lib-line-title">${t.title}</span><span class="muted">${t.meta}</span></span>
        ${t.badge ? html`<span class="shelf-line-badge muted">${t.badge.text}</span>` : null}</button>`;
    }
    return html`<button type="button" key=${item.id} class=${`lib-tile shelf-tile${on ? " selected" : ""}${t.missing ? " lib-missing" : ""}`} data-id=${item.id} aria-pressed=${on ? "true" : "false"} title=${t.title} ...${ev}>
      <span class="lib-tile-art"><${Pic} srcs=${t.pics} none=${t.none} />${t.badge ? html`<span class="lib-badge" style=${`border-color:${t.badge.colour || "var(--steel-2)"}`}>${t.badge.text}</span>` : null}</span>
      <span class="lib-tile-title">${t.title}</span>
      <span class="lib-tile-meta muted">${t.meta}</span></button>`;
  };
  const sortable = (id) => sorts.some(([k]) => k === id);
  const thead = view === "table" ? html`<div class="lib-row lib-thead shelf-thead" role="row" style=${`grid-template-columns:${cols}`}>${columns.map(([k, label, , sid]) => html`<span role="columnheader" key=${k} class=${`lib-cell ${p}-col-${k}`}
    aria-sort=${sid && sort === sid ? (st.desc ? "descending" : "ascending") : null}>${sid && sortable(sid) ? html`<button type="button" class="lib-sort" data-sort=${sid} onClick=${() => change({ sort: sid, desc: sort === sid ? !st.desc : false })}>${label}${sort === sid ? (st.desc ? " ▾" : " ▴") : ""}</button>` : label}</span>`)}</div>` : null;
  const heading = (s) => {
    const gm = groupMenu?.(s);
    const menu = gm?.length ? (e) => { e.preventDefault(); openMenu(e.currentTarget.closest(".lib-group").querySelector(".shelf-group-more") || e, gm); } : null;
    return html`<div class="lib-group shelf-group-head" data-group=${s.value} onContextMenu=${menu}><h2>${s.name}</h2><span class="muted">${plural(s.count, noun[0], noun[1])}</span>
      ${gm?.length ? html`<button type="button" class="ghost compact shelf-group-more" aria-haspopup="menu" title=${`${s.name}: more`} aria-label=${`${s.name}: more`} onClick=${(e) => openMenu(e.currentTarget, gm)}>${Icon.more(14)}</button>` : null}</div>`;
  };
  const multi = picked.length > 1;
  const panelActions = multi ? actions(picked).filter((a) => !a.one) : [];
  return html`${head}<div class=${`lib-page shelf-page ${p}-page`} id=${`${p}-page`} data-view=${view}>
    <aside class="lib-filters" id=${`${p}-filters`} aria-label="Filters">
      ${switches ? html`<div class="lib-switches">${switches}</div>` : null}
      ${fields.filter((x) => !x.hidden).map((x) => html`<${Values} key=${x.id} id=${`${p}-${x.id}`} name=${x.name} values=${facets[x.id]} chosen=${f[x.id] || []} onToggle=${toggle(x.id)} long=${!!x.long} />`)}
    </aside>
    <div class="lib-main">
      ${top}
      ${chips.length ? html`<div class="lib-chips" id=${`${p}-chips`}>${chips.map(([k, label, off]) => html`<span class="lib-chip" key=${k} data-chip=${k}>${label}<button type="button" class="ghost" aria-label=${`Remove ${label}`} title="Remove" onClick=${off}>${Icon.close(11)}</button></span>`)}
        <button type="button" class="linkish" id=${`${p}-clear`} onClick=${() => change({ f: {}, search: "" })}>Clear</button></div>` : null}
      <div class="lib-scroll shelf-scroll" ref=${scrollRef} onClick=${(e) => { if (e.target === e.currentTarget || e.target.classList.contains("shelf-items")) clear(); }}>
        ${thead}
        ${!shown.length ? html`<p class="muted lib-none" id=${`${p}-none`}>${none || `No ${noun[0]} matches these filters.`}</p>` : null}
        <div class=${`shelf-items shelf-${view}s`} id=${`${p}-items`} data-count=${shown.length} role=${view === "table" ? "table" : "list"} style=${`--tile:${SIZES[st.size] || SIZES.medium}px;--ratio:${ratio}`}>
          ${sections.map((s) => html`<section class="shelf-section" key=${s.value ?? "all"} data-group=${s.value ?? ""}>
            ${s.name != null ? heading(s) : null}
            <div class=${`shelf-${view}`}>${s.items.map(draw)}</div></section>`)}
        </div>
      </div>
    </div>
    ${!picked.length ? html`<aside class="lib-panel lib-panel-empty" id=${`${p}-panel`}></aside>`
      : multi ? html`<aside class="lib-panel" id=${`${p}-panel`} data-count=${picked.length}><h2>${plural(picked.length, noun[0], noun[1])} selected</h2>
        <div class="lib-panel-actions">${panelActions.map((a) => html`<button type="button" class="ghost" key=${a.id} id=${`${p}-panel-${a.id}`} disabled=${!!a.disabled} title=${typeof a.disabled === "string" ? a.disabled : a.label} onClick=${a.run}>${Icon[a.icon]?.(14)} ${a.label}</button>`)}</div>
        <p class="muted">Ctrl or Shift and a click adds to the selection; Esc clears it.</p></aside>`
      : html`<aside class="lib-panel" id=${`${p}-panel`} data-id=${one.id} key=${one.id}>${panel(one)}</aside>`}
  </div>`;
}

const scrolledTo = new Map();

/** A panel's buttons: the actions for one. */
export function PanelActions({ page, actions }) {
  return html`<div class="lib-panel-actions">${actions.map((a, i) => html`<button type="button" class=${i === 0 ? "primary compact" : "ghost compact"} key=${a.id} id=${`${page}-panel-${a.id}`} disabled=${!!a.disabled} title=${typeof a.disabled === "string" ? a.disabled : a.label} onClick=${a.run}>${Icon[a.icon]?.(14)} ${a.label}</button>`)}</div>`;
}

/** A dialog that asks for a word or a name (`list`: those there are already), or one of
 *  `options` ([[value, label]]); `run(value)` does it, and may open another dialog. */
function AskDialog({ title, label, value = "", list = [], options = null, button, note, empty = false, run }) {
  const [v, setV] = useState(value ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await run(options ? v : v.trim());
      if (ui.get().dialog?.type === "shelf-ask") close();
    } catch (e) {
      setError(e.message || String(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${title} id="shelf-ask" onSubmit=${submit} busy=${busy || (!empty && !String(v).trim())} error=${error} submitLabel=${button}>
    <label class="field-block"><span class="field-label">${label}</span>
      ${options ? html`<select id="shelf-ask-value" value=${v} onChange=${(e) => setV(e.target.value)}>${options.map(([k, l]) => html`<option key=${k} value=${k}>${l}</option>`)}</select>`
        : html`<input type="text" id="shelf-ask-value" list=${list.length ? "shelf-ask-list" : null} value=${v} onInput=${(e) => setV(e.target.value)} />`}</label>
    ${list.length && !options ? html`<datalist id="shelf-ask-list">${list.map((x) => html`<option key=${x} value=${x} />`)}</datalist>` : null}
    ${note ? html`<p class="muted">${note}</p>` : null}
  <//>`;
}

/** A game picked by its title (from the Library, every game in the DATs): `run(game)`. */
function GameDialog({ title, button, note, run }) {
  const [q, setQ] = useState("");
  const [found, setFound] = useState([]);
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (q.trim().length < 2) { setFound([]); return; }
    let gone = false;
    const t = setTimeout(() => api("browse_query", { query: { search: q, all: true, one_per_title: true, limit: 30 } })
      .then((r) => { if (!gone) { setFound(r.items || []); setTo(r.items?.[0]?.key || ""); } }).catch(() => { if (!gone) setFound([]); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [q]);
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await run(found.find((g) => g.key === to));
      if (ui.get().dialog?.type === "shelf-game") close();
    } catch (e) {
      setError(e.message || String(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${title} id="shelf-game" onSubmit=${submit} busy=${busy || !to} error=${error} submitLabel=${button}>
    <label class="field-block"><span class="field-label">The game's title</span>
      <input type="search" id="shelf-game-search" value=${q} placeholder="Its title" onInput=${(e) => setQ(e.target.value)} /></label>
    <select id="shelf-game-pick" class="compact" size="8" aria-label="The game" value=${to} onChange=${(e) => setTo(e.target.value)}>
      ${found.map((g) => html`<option key=${g.key} value=${g.key}>${g.title} (${g.platform_name})</option>`)}</select>
    ${note ? html`<p class="muted">${note}</p>` : null}
  <//>`;
}

export const askValue = (opts) => ui.set({ dialog: { type: "shelf-ask", ...opts } });
export const askGame = (opts) => ui.set({ dialog: { type: "shelf-game", ...opts } });

export function ShelfDialogs({ dialog }) {
  if (dialog.type === "shelf-ask") return html`<${AskDialog} ...${dialog} />`;
  if (dialog.type === "shelf-game") return html`<${GameDialog} ...${dialog} />`;
  return null;
}

/** Tags, shown and changed in a panel: each with ×, and a box to add one. */
export function TagEdit({ id, tags, all = [], off, onAdd, onRemove }) {
  const [v, setV] = useState("");
  const add = () => { const t = v.trim(); if (t) { onAdd(t); setV(""); } };
  return html`<div class="shelf-tags" id=${id}>
    ${(tags || []).map((t) => html`<span class="lib-chip" key=${t} data-tag=${t}>${t}<button type="button" class="ghost" aria-label=${`Remove the tag ${t}`} title="Remove the tag" disabled=${off} onClick=${() => onRemove(t)}>${Icon.close(11)}</button></span>`)}
    <input type="text" class="compact shelf-tag-add" id=${`${id}-add`} list=${`${id}-list`} aria-label="Add a tag" placeholder="Add a tag" value=${v} disabled=${off}
      onInput=${(e) => setV(e.target.value)} onKeyDown=${(e) => { if (e.key === "Enter") { e.preventDefault(); add(); } }} />
    <datalist id=${`${id}-list`}>${all.filter((t) => !(tags || []).includes(t)).map((t) => html`<option key=${t} value=${t} />`)}</datalist>
  </div>`;
}
