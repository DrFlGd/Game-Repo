// The Library (D98, D103): the library's games from the device's own copy, so browsing and
// search are instant and work away from home. Systems and collections down the side on a
// PC, a list at the top on a phone; the games as box fronts in a list that only puts what's
// in view in the page (100,000 games scroll as a hundred); a tick on each, and a bar with
// what's ticked, its size against the free space and Add to device….
import { html, useState, useEffect, useRef, useLayoutEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call, pictureUrl } from "./backend.js";
import { go, open, showDialog } from "./state.js";
import { bytes, count, message, plural } from "./format.js";

const PAGE = 200;   // games read at a time from the device's copy
const GAP = 12;
const PAD = 16;

/** Tick or untick games (`games` from a page of the list). */
export function tick(games, on) {
  go.set((s) => {
    const picked = { ...s.picked };
    for (const g of games) {
      if (on ?? !picked[g.key]) picked[g.key] = { title: g.title, size: g.size, system: g.system };
      else delete picked[g.key];
    }
    return { picked };
  });
}

function placeName(where, systems, collections) {
  if (where.system) return systems.find((s) => s.id === where.system)?.name || where.system;
  if (where.collection) {
    const c = collections.find((x) => x.id === where.collection);
    const g = where.group != null ? c?.groups.find((x) => x.id === where.group) : null;
    return g ? `${c.name} › ${g.name}` : c?.name || "Collection";
  }
  return "Every game";
}

/** The systems and collections: a list down the side (wide) or a picker (narrow). */
function Places({ systems, collections, where }) {
  const pick = (w) => go.set({ where: { system: "", collection: "", group: null, ...w }, q: "" });
  const is = (w) => where.system === (w.system || "") && where.collection === (w.collection || "") && (where.group ?? null) === (w.group ?? null);
  const value = where.system ? `s:${where.system}` : where.collection ? `c:${where.collection}:${where.group ?? ""}` : "";
  const chose = (v) => {
    if (!v) return pick({});
    if (v.startsWith("s:")) return pick({ system: v.slice(2) });
    const [, c, g] = v.split(":");
    pick({ collection: c, group: g || null });
  };
  const item = (w, label, n) => html`<button type="button" class="go-place" aria-current=${is(w) ? "true" : null} onClick=${() => pick(w)}>
    <span>${label}</span>${n != null ? html`<span class="go-place-n">${count(n)}</span>` : null}</button>`;
  return html`
    <label class="go-places-pick"><span class="visually-hidden">Show</span>
      <select id="go-place-pick" value=${value} onChange=${(e) => chose(e.currentTarget.value)}>
        <option value="">Every game</option>
        <optgroup label="Systems">${systems.map((s) => html`<option value=${`s:${s.id}`}>${s.name} (${count(s.games)})</option>`)}</optgroup>
        ${collections.length ? html`<optgroup label="Collections">${collections.flatMap((c) => [
          html`<option value=${`c:${c.id}:`}>${c.name} (${count(c.games)})</option>`,
          ...c.groups.filter((g) => g.id).map((g) => html`<option value=${`c:${c.id}:${g.id}`}>${c.name} › ${g.name} (${count(g.games)})</option>`),
        ])}</optgroup>` : null}
      </select>
    </label>
    <nav class="go-places" aria-label="Systems and collections" id="go-places">
      ${item({}, "Every game")}
      <h3>Systems</h3>
      ${systems.map((s) => html`<div key=${s.id}>${item({ system: s.id }, s.name, s.games)}</div>`)}
      ${collections.length ? html`<h3>Collections</h3>` : null}
      ${collections.map((c) => html`<div key=${c.id}>
        ${item({ collection: c.id }, c.name, c.games)}
        ${c.groups.filter((g) => g.id).map((g) => html`<div class="go-place-group" key=${g.id}>${item({ collection: c.id, group: g.id }, g.name, g.games)}</div>`)}
      </div>`)}
    </nav>`;
}

/** One game's card: its box front (its title when there's no picture, or none kept away
 *  from home), title, year and size, a tick, and whether it's on the device. */
function Card({ g, i, current, style, ticked, size, onKey }) {
  const [bad, setBad] = useState(false);
  useEffect(() => setBad(false), [g?.key]);
  if (!g) return html`<div class="card go-card is-loading" style=${style} aria-hidden="true"><span class="go-cover"></span></div>`;
  const sub = [g.year, bytes(g.size), g.discs > 1 ? `${g.discs} discs` : null].filter(Boolean).join(" · ");
  return html`<div class=${`card go-card${ticked ? " is-ticked" : ""}`} style=${style} data-key=${g.key}>
    <button type="button" class="go-card-open" data-index=${i} tabindex=${current ? 0 : -1} aria-label=${`${g.title}, ${sub}${g.on_device ? ", on this device" : ""}${ticked ? ", ticked" : ""}`}
      onClick=${() => open({ page: "game", key: g.key })} onKeyDown=${(e) => onKey(e, i, g)}>
      <span class="go-cover">
        <span class="go-cover-title">${g.title}</span>
        ${g.picture && !bad ? html`<img src=${pictureUrl(g.key, g.picture, size)} alt="" decoding="async" draggable="false" onError=${() => setBad(true)} />` : null}
      </span>
      <span class="go-card-title">${g.title}</span>
      <span class="go-card-sub">${sub}</span>
    </button>
    <button type="button" class="go-tick" tabindex="-1" aria-pressed=${ticked ? "true" : "false"} aria-label=${`Tick ${g.title}`} title=${ticked ? "Untick" : "Tick"} onClick=${() => tick([g])}>${Icon.check(16)}</button>
    ${g.on_device ? html`<span class="go-on-device" title="On this device">On this device</span>` : null}
  </div>`;
}

/** The games, read a page at a time and only those in view drawn. Arrows move by card and
 *  row, Page Up and Down by a screen, X or Space ticks. */
function Games({ ask, rev, picked }) {
  const scroller = useRef(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  const [top, setTop] = useState(0);
  const [total, setTotal] = useState(null);
  const [pages, setPages] = useState({});
  const [error, setError] = useState("");
  const [current, setCurrent] = useState(0);
  const asking = useRef(new Set());
  const focusTo = useRef(null);
  const askKey = JSON.stringify(ask) + "|" + rev;
  const now = useRef(askKey);
  now.current = askKey;
  const pagesRef = useRef(pages);
  pagesRef.current = pages;

  const read = (n) => {
    if (asking.current.has(n)) return;
    asking.current.add(n);
    const k = askKey;
    call("games", { ...ask, offset: n * PAGE, limit: PAGE })
      .then((r) => {
        if (k !== now.current) return;
        setError("");
        setTotal(r.total);
        setPages((p) => ({ ...p, [n]: r.items }));
      })
      .catch((e) => { if (k === now.current) { setError(message(e)); asking.current.delete(n); } });
  };

  // another place, search or order (or the catalogue changed): from the top, keeping what
  // was there until the new list comes, so a refresh doesn't flash
  useEffect(() => {
    asking.current = new Set();
    const fresh = !rev || JSON.stringify(ask) !== scroller.current?.dataset.ask;
    if (fresh) {
      setPages({});
      setTotal(null);
      setCurrent(0);
      if (scroller.current) scroller.current.scrollTop = 0;
      setTop(0);
    }
    if (scroller.current) scroller.current.dataset.ask = JSON.stringify(ask);
    read(0);
    if (!fresh) for (const n of Object.keys(pagesRef.current)) if (+n) read(+n);
  }, [askKey]);

  useLayoutEffect(() => {
    const el = scroller.current;
    const measure = () => setBox({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const narrow = box.w < 560;
  const min = narrow ? 132 : 164;
  const cols = Math.max(2, Math.floor((box.w - 2 * PAD + GAP) / (min + GAP)));
  const cw = Math.max(60, (box.w - 2 * PAD - GAP * (cols - 1)) / cols);
  const ch = Math.round(cw * 4 / 3) + 58;
  const rh = ch + GAP;
  const n = total || 0;
  const rows = Math.ceil(n / cols);
  const r0 = Math.max(0, Math.floor((top - PAD) / rh) - 2);
  const r1 = Math.min(rows, Math.ceil((top + box.h) / rh) + 2);
  const size = cw * (devicePixelRatio || 1) > 260 ? 512 : 256;

  useEffect(() => {
    if (!total) return;
    for (let i = r0 * cols; i < Math.min(n, r1 * cols); i += PAGE) read(Math.floor(i / PAGE));
    const last = Math.min(n, r1 * cols) - 1;
    if (last >= 0) read(Math.floor(last / PAGE));
  }, [r0, r1, cols, total, askKey]);

  // focus follows the arrows once the card is drawn
  useEffect(() => {
    if (focusTo.current == null) return;
    const el = scroller.current?.querySelector(`[data-index="${focusTo.current}"]`);
    if (el) { el.focus({ preventScroll: true }); focusTo.current = null; }
  });

  const goTo = (j) => {
    j = Math.max(0, Math.min(n - 1, j));
    setCurrent(j);
    const el = scroller.current;
    const y = PAD + Math.floor(j / cols) * rh;
    if (y < el.scrollTop) el.scrollTop = y - PAD;
    else if (y + ch > el.scrollTop + el.clientHeight) el.scrollTop = y + ch - el.clientHeight + PAD;
    setTop(el.scrollTop);
    // focus moves with the key when the card is drawn already (so a key pressed straight
    // after acts on it), else once it is
    const card = el.querySelector(`[data-index="${j}"]`);
    if (card) card.focus({ preventScroll: true });
    else focusTo.current = j;
  };

  const onKey = (e, i, g) => {
    const page = Math.max(1, Math.floor(box.h / rh)) * cols;
    const to = {
      ArrowRight: i + 1 < n ? i + 1 : null,
      ArrowLeft: i > 0 ? i - 1 : null,
      ArrowDown: i + cols < n ? i + cols : Math.floor(i / cols) < rows - 1 ? n - 1 : null,
      ArrowUp: i - cols >= 0 ? i - cols : null,
      PageDown: Math.min(n - 1, i + page),
      PageUp: Math.max(0, i - page),
      Home: 0,
      End: n - 1,
    }[e.key];
    if (to !== undefined) {
      // at an edge the key goes on to the controls before or after (keys.js)
      if (to === null || to === i) return;
      e.preventDefault();
      goTo(to);
    } else if (e.key === " " || e.key === "x" || e.key === "X") {
      e.preventDefault();
      tick([g]);
    }
  };

  const items = [];
  for (let i = r0 * cols; i < Math.min(n, r1 * cols); i++) {
    const g = pages[Math.floor(i / PAGE)]?.[i % PAGE];
    const style = `left:${PAD + (i % cols) * (cw + GAP)}px;top:${PAD + Math.floor(i / cols) * rh}px;width:${cw}px;height:${ch}px`;
    items.push(html`<${Card} key=${g?.key || `i${i}`} g=${g} i=${i} current=${i === Math.min(current, n - 1)} style=${style} ticked=${!!(g && picked[g.key])} size=${size} onKey=${onKey} />`);
  }
  return html`<div class="go-games" ref=${scroller} data-keys="own" id="go-games" aria-label="Games" onScroll=${(e) => setTop(e.currentTarget.scrollTop)}>
    ${error ? html`<p class="form-error go-games-note" role="alert">${error}</p>` : null}
    ${total === 0 ? html`<p class="empty go-games-note" id="go-no-games">${ask.q ? `No games match “${ask.q}” here.` : "No games here."}</p>` : null}
    <div class="go-games-inner" style=${`height:${rows ? PAD * 2 + rows * rh - GAP : 0}px`}>${items}</div>
    ${total != null ? html`<span class="visually-hidden" id="go-games-total" data-total=${total}>${plural(total, "game")}</span>` : null}
  </div>`;
}

/** What's ticked: how many, their size against the free space, Add to device…, and Make
 *  room… when there isn't room (C9, C20). */
function Picked({ picked, free }) {
  const keys = Object.keys(picked);
  if (!keys.length) return null;
  const size = keys.reduce((a, k) => a + (picked[k].size || 0), 0);
  const short = free != null && size > free;
  return html`<div class="go-picked" id="go-picked" role="region" aria-label="Ticked games">
    <span class="go-picked-what"><strong>${plural(keys.length, "game")}</strong> · ${bytes(size)}${free != null ? html` · <span class=${short ? "danger-text" : "muted"}>${bytes(free)} free</span>` : null}</span>
    <span class="go-picked-do">
      <button type="button" class="ghost" id="go-picked-clear" onClick=${() => go.set({ picked: {} })}>Clear</button>
      ${short ? html`<button type="button" class="ghost" id="go-make-room" onClick=${() => { go.set({ roomFor: size - free + (64 << 20) }); open({ page: "device" }); }}>Make room…</button>` : null}
      <button type="button" class="primary" id="go-add" onClick=${() => showDialog({ type: "add", keys })}>Add to device…</button>
    </span>
  </div>`;
}

export function LibraryPage() {
  const s = useStore(go, (x) => ({ systems: x.systems, collections: x.collections, where: x.where, q: x.q, sort: x.sort, desc: x.desc, picked: x.picked, rev: x.libraryRev, status: x.status }));
  const [text, setText] = useState(s.q);
  useEffect(() => {
    const t = setTimeout(() => go.set({ q: text.trim() }), 200);
    return () => clearTimeout(t);
  }, [text]);
  // a place picked ends the search
  useEffect(() => { if (!s.q && text.trim()) setText(""); }, [s.where]);
  const st = s.status;
  const sync = st?.sync || {};
  // a search is across every system (D103); otherwise the place picked
  const ask = s.q ? { q: s.q, sort: s.sort, desc: s.desc } : { system: s.where.system, collection: s.where.collection, group: s.where.group ?? undefined, sort: s.sort, desc: s.desc };
  const empty = (st?.catalogue?.games || 0) === 0;
  return html`<div class="go-library">
    <${Places} systems=${s.systems} collections=${s.collections} where=${s.where} />
    <div class="go-library-main">
      <div class="cmdbar go-tools">
        <h1 class="go-place-name" id="go-place-name">${s.q ? "Search" : placeName(s.where, s.systems, s.collections)}</h1>
        <label class="search small go-search">${Icon.search(16)}<span class="visually-hidden">Search every game</span>
          <input type="search" id="go-search" placeholder="Search every game" value=${text} onInput=${(e) => setText(e.currentTarget.value)}
            onKeyDown=${(e) => { if (e.key === "Escape" && text) { e.preventDefault(); setText(""); } }} />
        </label>
        <label class="go-sort"><span class="visually-hidden">Order</span>
          <select id="go-sort" value=${`${s.sort}${s.desc ? "-desc" : ""}`} onChange=${(e) => { const [sort, d] = e.currentTarget.value.split("-"); go.set({ sort, desc: !!d }); }}>
            <option value="title">Title, A to Z</option>
            <option value="title-desc">Title, Z to A</option>
            <option value="year">Year, oldest first</option>
            <option value="year-desc">Year, newest first</option>
            <option value="size">Size, smallest first</option>
            <option value="size-desc">Size, largest first</option>
          </select>
        </label>
      </div>
      ${sync.running ? html`<div class="go-note" id="go-reading" role="status">
          <span>Reading the library: ${count(sync.done)}${sync.of ? ` of ${count(sync.of)}` : ""} games</span>
          ${sync.of ? html`<progress max=${sync.of} value=${sync.done}></progress>` : html`<progress></progress>`}
        </div>`
        : empty && sync.error ? html`<div class="go-note warn-note" role="alert" id="go-sync-error">
          <span>${sync.error} Nothing of the library is kept on this device yet.</span>
          <button type="button" class="ghost" onClick=${() => call("sync").catch(() => {})}>Try again</button>
        </div>` : null}
      <${Games} ask=${ask} rev=${s.rev} picked=${s.picked} />
      <${Picked} picked=${s.picked} free=${st?.device?.free ?? null} />
    </div>
  </div>`;
}
