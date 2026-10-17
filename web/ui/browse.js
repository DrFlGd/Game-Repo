// The Library (docs/PLAN.md, "Phase 6 design", D42–D44): every game the library holds,
// across its systems, browsed by its pictures and found by its details. The core keeps a
// row per game in the index (browse.rs), so a search, the filters' counts and a page of
// 200,000 games each come back quickly; the page draws only the rows in view (a
// virtualised grid, list or table) and asks for the rest as it scrolls. The filters are
// kept in the address, so Back works and a view can be bookmarked. Each game opens on a
// page of its own (gamepage.js).
import { html, useState, useEffect, useRef, useLayoutEffect, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx, routeHash } from "./context.js";
import { api, toast, isDesktop, plural, size, libraryUrl } from "./library.js";
import { reveal } from "./games.js";
import { CommandBar, ViewSwitch, SortMenu, MenuButton } from "./layout.js";
import { Dialog, close } from "./dialogs.js";
import { usePageKeys, selectAllKey, openMenu } from "./actions.js";
import { scrapeDialog } from "./metadata.js";
import { attachDocs } from "./documents.js";
import { convertDialog } from "./organise.js";
import { Icon } from "./icons.js";

const CHUNK = 120;
const err = (e) => e.message || String(e);
const prefs = () => ctx.platform?.store?.prefs;

/** The filters with several values, as the address and the query name them. */
const LISTS = ["platform", "genre", "decade", "developer", "publisher", "region", "language", "has"];
export const SORTS = [["title", "Title"], ["system", "System"], ["year", "Year"], ["rating", "Rating"], ["size", "Size on disk"], ["date", "File date"]];
const GROUPS = [["", "None"], ["system", "System"], ["genre", "Genre"], ["decade", "Decade"], ["developer", "Developer"], ["publisher", "Publisher"]];
const VIEWS = [["grid", "Grid", "grid"], ["list", "List", "list"], ["table", "Table", "table"]];
const SIZES = { small: 120, medium: 160, large: 220 };
const HAS = [["pictures", "Pictures"], ["description", "A description"], ["manual", "A manual"], ["documents", "Documents"], ["video", "A video"]];
const FILTER_NAMES = { platform: "System", genre: "Genre", decade: "Year", developer: "Developer", publisher: "Publisher", region: "Region", language: "Language", has: "Has" };
const RATINGS = [[0.4, "★★ and up"], [0.6, "★★★ and up"], [0.8, "★★★★ and up"]];

// ------------------------------------------------------------ the query, in the address

const blank = () => ({ search: "", all: false, one_per_title: false, look: false, players: 0, rating: 0, collection: "", sort: "title", desc: false, group: "", ...Object.fromEntries(LISTS.map((k) => [k, []])) });

/** The query an address holds (`#/library?genre=Puzzle&sort=year`). */
export function queryOf(hash = location.hash) {
  const p = new URLSearchParams(hash.split("?")[1] || "");
  const q = blank();
  q.search = p.get("q") || "";
  q.all = p.get("all") === "1";
  q.one_per_title = p.get("one") === "1";
  q.look = p.get("look") === "1";
  q.players = Number(p.get("players")) || 0;
  q.rating = Number(p.get("rating")) || 0;
  q.collection = p.get("collection") || "";
  q.sort = SORTS.some(([k]) => k === p.get("sort")) ? p.get("sort") : "title";
  q.desc = p.get("desc") === "1";
  q.group = GROUPS.some(([k]) => k && k === p.get("group")) ? p.get("group") : "";
  for (const k of LISTS) q[k] = p.getAll(k).map((v) => (k === "decade" ? Number(v) : v)).filter((v) => v !== "" && !Number.isNaN(v));
  return q;
}

/** The address of a query. */
export function hashOf(q) {
  const p = new URLSearchParams();
  if (q.search) p.set("q", q.search);
  for (const k of LISTS) for (const v of q[k]) p.append(k, String(v));
  if (q.all) p.set("all", "1");
  if (q.one_per_title) p.set("one", "1");
  if (q.look) p.set("look", "1");
  if (q.players) p.set("players", String(q.players));
  if (q.rating) p.set("rating", String(q.rating));
  if (q.collection) p.set("collection", q.collection);
  if (q.sort !== "title") p.set("sort", q.sort);
  if (q.desc) p.set("desc", "1");
  if (q.group) p.set("group", q.group);
  const s = p.toString();
  return `#/library${s ? `?${s}` : ""}`;
}

const filtered = (q) => LISTS.some((k) => q[k].length) || q.look || q.players || q.rating || q.collection || q.search;

/** The list a game page was opened from, so Previous and Next go through it. */
export const opened = { query: null, index: -1, total: 0 };

/** A game's page. */
export const gameHash = (key) => `#/game/${key.split("/").map(encodeURIComponent).join("/")}`;

// ------------------------------------------------------------ pictures

/** A colour of a system's own, for games with no picture. */
export const systemColour = (id) => {
  let h = 0;
  for (const c of id || "") h = (h * 31 + c.charCodeAt(0)) % 360;
  return `hsl(${h} 38% 46%)`;
};

/** A game's picture as a thumbnail (made by the core the first time, D43), else its
 *  title's first letter on its system's colour. */
export function Cover({ g, px = 256, cls = "lib-cover" }) {
  const [broken, setBroken] = useState(false);
  useEffect(() => setBroken(false), [g.picture]);
  if (g.picture && !broken) return html`<img class=${cls} src=${libraryUrl(`~thumb/${px}/${g.picture}`)} alt="" loading="lazy" decoding="async" draggable="false" onError=${() => setBroken(true)} />`;
  return html`<span class=${`${cls} lib-cover-none`} style=${`background:${systemColour(g.platform)}`} aria-hidden="true">${(g.title || "?").slice(0, 1)}</span>`;
}

const stars = (r) => (r == null ? "" : "★".repeat(Math.round(r * 5)) + "☆".repeat(5 - Math.round(r * 5)));

// ------------------------------------------------------------ the filters

function Values({ id, name, values, chosen, onToggle, label = (v) => v.name || String(v.value), long = false }) {
  const [open, setOpen] = useState(false);
  const [find, setFind] = useState("");
  const all = values || [];
  const list = all.filter((v) => !find || label(v).toLowerCase().includes(find.toLowerCase()));
  // the chosen first, then the most games
  const shown = open || find ? list : [...list.filter((v) => chosen.includes(v.value)), ...list.filter((v) => !chosen.includes(v.value))].slice(0, Math.max(8, chosen.length));
  if (!all.length && !chosen.length) return null;
  return html`<details class="lib-filter" id=${`filter-${id}`} open>
    <summary>${name}${chosen.length ? html` <span class="lib-filter-on">${chosen.length}</span>` : null}</summary>
    ${long && all.length > 10 ? html`<input type="search" class="compact lib-filter-find" placeholder=${`Find a ${name.toLowerCase()}`} aria-label=${`Find a ${name.toLowerCase()}`} value=${find} onInput=${(e) => setFind(e.target.value)} />` : null}
    <ul class="lib-values">${shown.map((v) => html`<li key=${String(v.value)}>
      <label class="check-row" data-value=${String(v.value)}><input type="checkbox" checked=${chosen.includes(v.value)} onChange=${() => onToggle(v.value)} />
        <span class="lib-value">${label(v)}</span><span class="lib-count">${v.count.toLocaleString()}</span></label></li>`)}
      ${chosen.filter((c) => !all.some((v) => v.value === c)).map((c) => html`<li key=${`gone-${c}`}><label class="check-row"><input type="checkbox" checked=${true} onChange=${() => onToggle(c)} /><span class="lib-value">${c}</span><span class="lib-count">0</span></label></li>`)}</ul>
    ${!find && list.length > shown.length ? html`<button type="button" class="linkish lib-more" onClick=${() => setOpen(true)}>Show all ${list.length}${all.length >= 500 ? "+" : ""}</button>` : null}
    ${open && !find && list.length > 8 ? html`<button type="button" class="linkish lib-more" onClick=${() => setOpen(false)}>Show fewer</button>` : null}
  </details>`;
}

function AtLeast({ id, name, values, value, onChange, label }) {
  if (!values?.some((v) => v.count) && !value) return null;
  return html`<details class="lib-filter" id=${`filter-${id}`} open>
    <summary>${name}${value ? html` <span class="lib-filter-on">1</span>` : null}</summary>
    <ul class="lib-values">
      <li><label class="check-row"><input type="radio" name=${`f-${id}`} checked=${!value} onChange=${() => onChange(0)} /><span class="lib-value">Any</span></label></li>
      ${values.map((v) => html`<li key=${v.value}><label class="check-row" data-value=${v.value}><input type="radio" name=${`f-${id}`} checked=${value === v.value} onChange=${() => onChange(v.value)} />
        <span class="lib-value">${label(v.value)}</span><span class="lib-count">${v.count.toLocaleString()}</span></label></li>`)}
    </ul></details>`;
}

function Filters({ q, facets, set, collections }) {
  const f = facets || {};
  const toggle = (k) => (v) => set({ [k]: q[k].includes(v) ? q[k].filter((x) => x !== v) : [...q[k], v] });
  const has = HAS.map(([k, l]) => ({ value: k, name: l, count: f.has?.[k] || 0 }));
  return html`<aside class="lib-filters" id="lib-filters" aria-label="Filters">
    <div class="lib-switches">
      <label class="check-row" title="Every game in the systems' DATs, so the games you've scraped but don't have can be browsed too"><input type="checkbox" id="lib-all" checked=${q.all} onChange=${(e) => set({ all: e.target.checked })} /> Every game in the DATs</label>
      <label class="check-row" title="One game of each title, in the collection rules' region order (USA, World, Europe, Japan)"><input type="checkbox" id="lib-one" checked=${q.one_per_title} onChange=${(e) => set({ one_per_title: e.target.checked })} /> One per title</label>
      ${f.look || q.look ? html`<label class="check-row"><input type="checkbox" id="lib-look" checked=${q.look} onChange=${(e) => set({ look: e.target.checked })} /> Needs a look <span class="lib-count">${(f.look || 0).toLocaleString()}</span></label>` : null}
    </div>
    <${Values} id="platform" name="System" values=${f.platform} chosen=${q.platform} onToggle=${toggle("platform")} />
    <${Values} id="genre" name="Genre" values=${f.genre} chosen=${q.genre} onToggle=${toggle("genre")} long=${true} />
    <${Values} id="decade" name="Year" values=${f.decade} chosen=${q.decade} onToggle=${toggle("decade")} label=${(v) => `${v.value}s`} />
    <${Values} id="developer" name="Developer" values=${f.developer} chosen=${q.developer} onToggle=${toggle("developer")} long=${true} />
    <${Values} id="publisher" name="Publisher" values=${f.publisher} chosen=${q.publisher} onToggle=${toggle("publisher")} long=${true} />
    <${AtLeast} id="players" name="Players" values=${f.players} value=${q.players} onChange=${(players) => set({ players })} label=${(n) => (n === 1 ? "1 or more" : `${n} or more`)} />
    <${AtLeast} id="rating" name="Rating" values=${f.rating} value=${q.rating} onChange=${(rating) => set({ rating })} label=${(n) => RATINGS.find(([v]) => v === n)?.[1] || n} />
    <${Values} id="region" name="Region" values=${f.region} chosen=${q.region} onToggle=${toggle("region")} />
    <${Values} id="language" name="Language" values=${f.language} chosen=${q.language} onToggle=${toggle("language")} />
    <${Values} id="has" name="Has" values=${has.filter((v) => v.count || q.has.includes(v.value))} chosen=${q.has} onToggle=${toggle("has")} />
    ${collections?.length ? html`<details class="lib-filter" id="filter-collection" open><summary>In a collection${q.collection ? html` <span class="lib-filter-on">1</span>` : null}</summary>
      <select id="lib-collection" class="compact" value=${q.collection} onChange=${(e) => set({ collection: e.target.value })}>
        <option value="">Any game</option>
        ${collections.map((c) => html`<option key=${c.id} value=${c.id}>${c.name}</option>`)}
      </select></details>` : null}
  </aside>`;
}

/** The filters in use, each with ×. */
function Chips({ q, set, facets, collections }) {
  const chips = [];
  const name = (k, v) => (k === "platform" ? facets?.platform?.find((x) => x.value === v)?.name || v : k === "decade" ? `${v}s` : k === "has" ? HAS.find(([x]) => x === v)?.[1] || v : v);
  for (const k of LISTS) for (const v of q[k]) chips.push([`${k}-${v}`, `${FILTER_NAMES[k]}: ${name(k, v)}`, () => set({ [k]: q[k].filter((x) => x !== v) })]);
  if (q.players) chips.push(["players", `Players: ${q.players}+`, () => set({ players: 0 })]);
  if (q.rating) chips.push(["rating", `Rating: ${RATINGS.find(([v]) => v === q.rating)?.[1] || q.rating}`, () => set({ rating: 0 })]);
  if (q.look) chips.push(["look", "Needs a look", () => set({ look: false })]);
  if (q.collection) chips.push(["collection", `In ${collections?.find((c) => c.id === q.collection)?.name || "a collection"}`, () => set({ collection: "" })]);
  if (q.search) chips.push(["search", `Search: ${q.search}`, () => set({ search: "" })]);
  if (!chips.length) return null;
  return html`<div class="lib-chips" id="lib-chips">${chips.map(([k, label, off]) => html`<span class="lib-chip" key=${k} data-chip=${k}>${label}<button type="button" class="ghost" aria-label=${`Remove ${label}`} title="Remove" onClick=${off}>${Icon.close(11)}</button></span>`)}
    <button type="button" class="linkish" id="lib-clear" onClick=${() => set({ ...blank(), sort: q.sort, desc: q.desc, group: q.group, all: q.all, one_per_title: q.one_per_title })}>Clear</button></div>`;
}

// ------------------------------------------------------------ the games, a page at a time

/** The list's games, fetched a chunk at a time as they come into view. */
function useGames(q, version, ready) {
  const [state, setState] = useState({ total: 0, loading: true, facets: null, groups: null, rev: 0 });
  const chunks = useRef(new Map());
  const pending = useRef(new Set());
  const gen = useRef(0);
  useEffect(() => {
    if (!ready) return;
    const mine = ++gen.current;
    setState((s) => ({ ...s, loading: true }));
    const t = setTimeout(async () => {
      try {
        const r = await api("browse_query", { query: { ...q, offset: 0, limit: CHUNK, facets: true } });
        if (mine !== gen.current) return;
        chunks.current = new Map([[0, r.items]]);
        pending.current = new Set();
        setState({ total: r.total, loading: false, facets: r.facets, groups: r.groups || null, rev: Date.now(), error: null });
      } catch (e) {
        if (mine === gen.current) setState({ total: 0, loading: false, facets: null, groups: null, rev: Date.now(), error: err(e) });
      }
    }, q.search ? 180 : 0);
    return () => clearTimeout(t);
  }, [JSON.stringify(q), version, ready]);
  const need = (n) => {
    if (chunks.current.has(n) || pending.current.has(n)) return;
    pending.current.add(n);
    const mine = gen.current;
    api("browse_query", { query: { ...q, offset: n * CHUNK, limit: CHUNK } }).then((r) => {
      if (mine !== gen.current) return;
      chunks.current.set(n, r.items);
      pending.current.delete(n);
      setState((s) => ({ ...s, rev: s.rev + 1 }));
    }).catch(() => pending.current.delete(n));
  };
  const at = (i) => chunks.current.get(Math.floor(i / CHUNK))?.[i % CHUNK];
  /** Games i..j, fetched first where they aren't yet. */
  const range = async (i, j) => {
    const out = [];
    for (let c = Math.floor(i / CHUNK); c <= Math.floor(j / CHUNK); c++) {
      if (!chunks.current.has(c)) chunks.current.set(c, (await api("browse_query", { query: { ...q, offset: c * CHUNK, limit: CHUNK } })).items);
    }
    for (let k = i; k <= j; k++) if (at(k)) out.push(at(k));
    return out;
  };
  return { ...state, at, need, range };
}

/** The list laid out: a segment per group (its heading, then rows of `cols` games). */
function layout(total, groups, cols, rowH, headH) {
  const segs = [];
  let top = 0, start = 0;
  for (const g of groups || [{ value: null, count: total, none: true }]) {
    const rows = Math.ceil(g.count / cols);
    const h = (g.none ? 0 : headH) + rows * rowH;
    segs.push({ g, top, start, rows, head: g.none ? 0 : headH, h });
    top += h;
    start += g.count;
  }
  return { segs, height: top };
}

function GroupHead({ g, group, q }) {
  const name = g.name || (g.value == null ? "Unknown" : group === "decade" ? `${g.value}s` : String(g.value));
  return html`<div class="lib-group" data-group=${String(g.value)}><h2>${name}</h2><span class="muted">${plural(g.count, "game", "games")}</span></div>`;
}

function Tile({ g, i, on, onPick, onOpen, onMenu, px }) {
  if (!g) return html`<div class="lib-tile lib-wait" aria-hidden="true"><span class="lib-cover lib-cover-wait"></span></div>`;
  return html`<button type="button" class=${`lib-tile${on ? " selected" : ""}${g.have ? "" : " lib-missing"}`} data-key=${g.key} data-index=${i} aria-pressed=${on ? "true" : "false"} title=${`${g.title}${g.year ? ` (${g.year})` : ""} · ${g.platform_name}`}
    onClick=${(e) => onPick(e, g, i)} onDblClick=${() => onOpen(g, i)} onContextMenu=${(e) => onMenu(e, g, i)}>
    <span class="lib-tile-art"><${Cover} g=${g} px=${px} /><span class="lib-badge" style=${`border-color:${systemColour(g.platform)}`}>${g.platform_name}</span></span>
    <span class="lib-tile-title">${g.title}</span>
    <span class="lib-tile-meta muted">${[g.year, g.developer].filter(Boolean).join(" · ") || (g.have ? "" : "Not in the library")}</span>
  </button>`;
}

function Line({ g, i, on, onPick, onOpen, onMenu }) {
  if (!g) return html`<div class="lib-line lib-wait" aria-hidden="true"></div>`;
  return html`<button type="button" class=${`lib-line${on ? " selected" : ""}${g.have ? "" : " lib-missing"}`} data-key=${g.key} data-index=${i} aria-pressed=${on ? "true" : "false"}
    onClick=${(e) => onPick(e, g, i)} onDblClick=${() => onOpen(g, i)} onContextMenu=${(e) => onMenu(e, g, i)}>
    <${Cover} g=${g} px=${128} cls="lib-cover small" />
    <span class="lib-line-main"><span class="lib-line-title">${g.title}${g.year ? html` <span class="muted">(${g.year})</span>` : null}</span>
      <span class="muted">${[g.platform_name, g.developer, g.genres?.slice(0, 2).join(", "), g.have ? null : "not in the library"].filter(Boolean).join(" · ")}</span></span>
    ${g.rating != null ? html`<span class="lib-stars" title=${`${Math.round(g.rating * 100)}%`}>${stars(g.rating)}</span>` : null}
  </button>`;
}

const COLUMNS = [
  ["title", "Title", (g) => g.title, "title"],
  ["system", "System", (g) => g.platform_name, "system"],
  ["year", "Year", (g) => g.year || "", "year"],
  ["developer", "Developer", (g) => g.developer || "", null],
  ["genre", "Genre", (g) => (g.genres || []).join(", "), null],
  ["players", "Players", (g) => g.players || "", null],
  ["rating", "Rating", (g) => stars(g.rating), "rating"],
  ["regions", "Region", (g) => g.regions, null],
  ["size", "Size", (g) => (g.size ? size(g.size) : ""), "size"],
  ["status", "Status", (g) => (g.have ? (g.review ? "Needs a look" : "In the library") : "Not in the library"), null],
];

function Row({ g, i, on, onPick, onOpen, onMenu }) {
  if (!g) return html`<div class="lib-row lib-wait" aria-hidden="true"></div>`;
  return html`<div role="row" tabindex="-1" class=${`lib-row${on ? " selected" : ""}${g.have ? "" : " lib-missing"}`} data-key=${g.key} data-index=${i} aria-selected=${on ? "true" : "false"}
    onClick=${(e) => onPick(e, g, i)} onDblClick=${() => onOpen(g, i)} onContextMenu=${(e) => onMenu(e, g, i)}>
    ${COLUMNS.map(([k, , v]) => html`<span role="cell" key=${k} class=${`lib-cell lib-col-${k}`}>${v(g)}</span>`)}</div>`;
}

function TableHead({ q, set }) {
  return html`<div class="lib-row lib-thead" role="row">${COLUMNS.map(([k, label, , sort]) => html`<span role="columnheader" key=${k} class=${`lib-cell lib-col-${k}`}
    aria-sort=${sort && q.sort === sort ? (q.desc ? "descending" : "ascending") : null}>${sort ? html`<button type="button" class="linkish" data-sort=${sort} onClick=${() => set({ sort, desc: q.sort === sort ? !q.desc : false })}>${label}${q.sort === sort ? (q.desc ? " ▾" : " ▴") : ""}</button>` : label}</span>`)}</div>`;
}

/** The grid, list or table, drawing only what's in view. */
function Games({ games, q, set, view, px, selected, onPick, onOpen, onMenu, scrollRef }) {
  const [box, setBox] = useState({ w: 800, h: 600, top: 0 });
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => setBox((b) => ({ ...b, w: el.clientWidth, h: el.clientHeight }));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    let raf = 0;
    const onScroll = () => { cancelAnimationFrame(raf); raf = requestAnimationFrame(() => setBox((b) => ({ ...b, top: el.scrollTop }))); };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => { ro.disconnect(); el.removeEventListener("scroll", onScroll); cancelAnimationFrame(raf); };
  }, [view]);
  const gap = 14;
  const tileW = SIZES[px] || SIZES.medium;
  const inner = Math.max(200, box.w - 32);
  const cols = view === "grid" ? Math.max(1, Math.floor((inner + gap) / (tileW + gap))) : 1;
  const colW = view === "grid" ? (inner - gap * (cols - 1)) / cols : inner;
  const rowH = view === "grid" ? Math.round(colW * 1.38) + 52 + gap : view === "list" ? 58 : 34;
  const { segs, height } = useMemo(() => layout(games.total, q.group ? games.groups : null, cols, rowH, 44), [games.total, games.groups, cols, rowH, q.group]);
  const from = box.top - box.h, to = box.top + box.h * 2;
  const drawn = [];
  const wanted = new Set();
  for (const s of segs) {
    if (s.top + s.h < from || s.top > to) continue;
    if (s.head && s.top + s.head >= from) drawn.push(html`<div class="lib-abs" key=${`g-${s.g.value}`} style=${`transform:translateY(${s.top}px);height:${s.head}px`}><${GroupHead} g=${s.g} group=${q.group} q=${q} /></div>`);
    const r0 = Math.max(0, Math.floor((from - s.top - s.head) / rowH));
    const r1 = Math.min(s.rows - 1, Math.floor((to - s.top - s.head) / rowH));
    for (let r = r0; r <= r1; r++) {
      const cells = [];
      for (let c = 0; c < cols; c++) {
        const k = r * cols + c;
        if (k >= s.g.count) break;
        const i = s.start + k;
        const g = games.at(i);
        if (!g) wanted.add(Math.floor(i / CHUNK));
        const on = !!g && selected(g.key);
        const props = { g, i, on, onPick, onOpen, onMenu, px: px === "large" ? 512 : 256 };
        cells.push(view === "grid" ? html`<${Tile} key=${i} ...${props} />` : view === "list" ? html`<${Line} key=${i} ...${props} />` : html`<${Row} key=${i} ...${props} />`);
      }
      drawn.push(html`<div class=${`lib-abs lib-r lib-r-${view}`} key=${`${s.start}-${r}`} style=${`transform:translateY(${s.top + s.head + r * rowH}px);height:${rowH - (view === "grid" ? gap : 0)}px;${view === "grid" ? `grid-template-columns:repeat(${cols}, minmax(0, 1fr));gap:${gap}px` : ""}`}>${cells}</div>`);
    }
  }
  useEffect(() => { for (const n of wanted) games.need(n); });
  return html`${view === "table" ? html`<${TableHead} q=${q} set=${set} />` : null}
    <div class=${`lib-canvas lib-${view}`} id="lib-games" role=${view === "table" ? "table" : "list"} data-total=${games.total} data-cols=${cols} style=${`height:${height}px`}>${drawn}</div>`;
}

// ------------------------------------------------------------ the panel

const DETAIL_LABELS = [["released", "Released"], ["developer", "Developer"], ["publisher", "Publisher"], ["genres", "Genres"], ["players", "Players"], ["rating", "Rating"]];

function Inspector({ count, keyOf, actions, overview, off }) {
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  const changes = useStore(ui, (s) => s.changes);
  useEffect(() => {
    if (!keyOf) { setG(null); return; }
    let gone = false;
    setError("");
    api("browse_game", { key: keyOf }).then((x) => { if (!gone) setG(x); }).catch((e) => { if (!gone) setError(err(e)); });
    return () => { gone = true; };
  }, [keyOf, changes]);
  if (count > 1) {
    return html`<aside class="lib-panel" id="lib-panel" data-count=${count}><h2>${plural(count, "game", "games")} selected</h2>
      <div class="lib-panel-actions">${actions.filter((a) => !a.one).map((a) => html`<button type="button" class="ghost" key=${a.id} id=${`lib-panel-${a.id}`} disabled=${!!a.disabled} title=${typeof a.disabled === "string" ? a.disabled : a.label} onClick=${a.run}>${Icon[a.icon]?.(14)} ${a.label}</button>`)}</div>
      <p class="muted">Ctrl or Shift and a click adds to the selection; Esc clears it.</p></aside>`;
  }
  if (!keyOf) {
    return html`<aside class="lib-panel lib-panel-empty" id="lib-panel">
      ${overview ? html`<p><b>${overview.have.toLocaleString()}</b> games in the library, across ${plural(overview.systems, "system", "systems")}${overview.all > overview.have ? html`; <span class="muted">${overview.all.toLocaleString()} in the DATs</span>` : null}.</p>` : null}
      <p class="muted">Pick a game to see it here; double-click (or Enter) opens its page. Ctrl+A selects every game shown, for a collection, a scrape or a conversion.</p></aside>`;
  }
  if (error) return html`<aside class="lib-panel" id="lib-panel"><p class="warn-note" role="alert">${error}</p></aside>`;
  if (!g || g.key !== keyOf) return html`<aside class="lib-panel" id="lib-panel"><p class="muted">Reading…</p></aside>`;
  const d = g.details || {};
  const facts = DETAIL_LABELS.map(([k, l]) => [l, k === "genres" && Array.isArray(d[k]) ? d[k].join(", ") : k === "rating" && d[k] != null ? `${stars(d[k])} ${Math.round(d[k] * 100)}%` : d[k]]).filter(([, v]) => v);
  const picture = g.media?.cover || g.media?.box3d || g.media?.screenshot || g.media?.titlescreen || g.media?.fanart;
  return html`<aside class="lib-panel" id="lib-panel" data-key=${g.key}>
    ${picture ? html`<img class="lib-panel-cover" src=${libraryUrl(`~thumb/512/${picture}`)} alt="" />` : html`<span class="lib-panel-cover lib-cover-none" style=${`background:${systemColour(g.platform)}`} aria-hidden="true">${(g.title || "?").slice(0, 1)}</span>`}
    <h2 id="lib-panel-title">${g.title}${g.year ? html` <span class="muted">(${g.year})</span>` : null}</h2>
    <p class="muted">${[g.platform_name, g.have === false ? "not in the library" : null, g.name !== g.title ? g.name : null].filter(Boolean).join(" · ")}</p>
    <div class="lib-panel-actions">
      <a class="primary compact lib-open" id="lib-panel-open" href=${gameHash(g.key)}>Open</a>
      ${actions.filter((a) => a.one !== false).slice(1).map((a) => html`<button type="button" class="ghost compact" key=${a.id} id=${`lib-panel-${a.id}`} disabled=${!!a.disabled || off} title=${typeof a.disabled === "string" ? a.disabled : a.label} onClick=${a.run}>${Icon[a.icon]?.(14)} ${a.label}</button>`)}
    </div>
    ${d.description ? html`<p class="lib-panel-text">${d.description}</p>` : null}
    ${facts.length ? html`<dl class="pc-facts">${facts.map(([k, v]) => html`<dt key=${`k${k}`}>${k}</dt><dd key=${`v${k}`}>${v}</dd>`)}</dl>` : html`<p class="muted">No details yet: Scrape… fills them in.</p>`}
    ${g.docs?.length ? html`<p class="muted">${plural(g.docs.length, "document", "documents")}</p>` : null}
    ${g.collections?.length ? html`<p class="muted">In ${g.collections.map((c) => c.name).join(", ")}</p>` : null}
  </aside>`;
}

// ------------------------------------------------------------ dialogs

/** Add games to a collection (a new one, or one there is), by hand. */
function AddToCollection({ games }) {
  const [list, setList] = useState(null);
  const [to, setTo] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { api("collections").then((r) => { const cs = r.collections || r || []; setList(cs); setTo(cs[0]?.id || "new"); }).catch((e) => setError(err(e))); }, []);
  const keys = games.filter((g) => g.kind === "dat").map((g) => g.key);
  const run = async () => {
    setBusy(true);
    setError("");
    try {
      let c;
      if (to === "new") {
        const platforms = [...new Set(games.filter((g) => g.kind === "dat").map((g) => g.platform))];
        // only these games, added by hand
        c = { name: name.trim() || "New collection", rules: { platforms, include: "none" }, added: [], removed: [] };
      } else {
        c = (await api("collection_get", { id: to })).collection;
      }
      const added = new Set(c.added || []);
      for (const k of keys) added.add(k);
      c.added = [...added];
      c.removed = (c.removed || []).filter((k) => !added.has(k));
      const r = await api("collection_save", { collection: c });
      ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
      close();
      toast(`Added ${plural(keys.length, "game", "games")} to ${r.collection?.name || c.name}.`, { ms: 8000, actions: [{ label: "Open", run: () => { location.hash = routeHash("collections"); } }] });
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  const skipped = games.length - keys.length;
  return html`<${Dialog} title=${`Add ${plural(keys.length, "game", "games")} to a collection`} id="lib-add-dialog" onSubmit=${run} busy=${busy || !list || !keys.length} error=${error} submitLabel="Add">
    ${list ? html`<label class="field-block"><span class="field-label">Collection</span>
      <select id="lib-add-to" class="compact" value=${to} onChange=${(e) => setTo(e.target.value)}>
        ${list.map((c) => html`<option key=${c.id} value=${c.id}>${c.name}</option>`)}
        <option value="new">A new collection…</option>
      </select></label>` : html`<p class="muted">Reading the collections…</p>`}
    ${to === "new" ? html`<label class="field-block"><span class="field-label">Its name</span><input type="text" id="lib-add-name" value=${name} placeholder="New collection" onInput=${(e) => setName(e.target.value)} /></label>` : null}
    <p class="muted">The games are added by hand, so they stay in it whatever its rules say.${skipped ? ` ${plural(skipped, "PC game isn't", "PC games aren't")} added: collections hold the systems' games.` : ""}</p>
  <//>`;
}

/** Save the filters as a collection: its rules keep up as games are scraped. */
function SaveAsCollection({ q }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const run = async () => {
    setBusy(true);
    setError("");
    try {
      const rules = {
        platforms: q.platform.filter((p) => p !== "pc"),
        include: q.all ? "all" : "have",
        one_per_title: q.one_per_title,
        regions: ["USA", "World", "Europe", "Japan", ...q.region.filter((r) => !["USA", "World", "Europe", "Japan"].includes(r))],
        only_regions: q.region.length > 0,
        languages: q.language,
        exclude: q.one_per_title ? undefined : [],
        details: { search: q.search, genre: q.genre, decade: q.decade, developer: q.developer, publisher: q.publisher, players: q.players, rating: q.rating, has: q.has, look: q.look },
      };
      if (q.region.length) rules.regions = q.region;
      if (rules.exclude === undefined) delete rules.exclude;
      const r = await api("collection_save", { collection: { name: name.trim() || "From the Library", rules, added: [], removed: [] } });
      ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
      close();
      toast(`Saved ${r.collection?.name || name} as a collection.`, { ms: 8000, actions: [{ label: "Open", run: () => { location.hash = routeHash("collections"); } }] });
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title="Save as a collection" id="lib-save-dialog" onSubmit=${run} busy=${busy} error=${error} submitLabel="Save">
    <label class="field-block"><span class="field-label">Its name</span><input type="text" id="lib-save-name" value=${name} placeholder="From the Library" onInput=${(e) => setName(e.target.value)} /></label>
    <p class="muted">The filters become its rules, so it keeps up as games are added and scraped. ${q.collection ? "(In a collection isn't carried over.) " : ""}${q.platform.includes("pc") ? "PC games aren't in collections. " : ""}It leaves out betas, demos and the like, as a new collection does.</p>
  <//>`;
}

export function BrowseDialogs({ dialog }) {
  if (dialog.type === "lib-add") return html`<${AddToCollection} ...${dialog} />`;
  if (dialog.type === "lib-save") return html`<${SaveAsCollection} ...${dialog} />`;
  return null;
}

// ------------------------------------------------------------ the page

export function Browse() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes, home: st.home }));
  const off = !!s.library?.read_only;
  const [q, setQState] = useState(() => queryOf());
  const [view, setView] = useState(() => prefs()?.get("gr-library-view", "grid") || "grid");
  const [px, setPx] = useState(() => prefs()?.get("gr-library-size", "medium") || "medium");
  const [overview, setOverview] = useState(null);
  const [collections, setCollections] = useState([]);
  const [sel, setSel] = useState(() => new Map());
  const [all, setAll] = useState(false);
  const [anchor, setAnchor] = useState(-1);
  const [box, setBox] = useState(q.search);
  const scrollRef = useRef(null);
  useEffect(() => {
    if (!s.library) return;
    let gone = false;
    api("browse").then((o) => { if (!gone) setOverview(o); }).catch((e) => { if (!gone) setOverview({ error: err(e) }); });
    api("collections").then((r) => { if (!gone) setCollections(r.collections || r || []); }).catch(() => {});
    return () => { gone = true; };
  }, [s.library?.path, s.changes, s.home]);
  const games = useGames(q, `${s.changes}`, !!overview && !overview.error);
  // the address is the query: Back and a bookmark come back to it
  useEffect(() => {
    const on = () => { if (location.hash.startsWith("#/library")) { const next = queryOf(); setQState(next); setBox(next.search); } };
    addEventListener("hashchange", on);
    return () => removeEventListener("hashchange", on);
  }, []);
  const set = (patch, replace = false) => {
    const next = { ...q, ...patch };
    setQState(next);
    clearSel();
    if (replace) history.replaceState(null, "", hashOf(next));
    else if (location.hash !== hashOf(next)) location.hash = hashOf(next);
    if (!("search" in patch)) scrollRef.current?.scrollTo({ top: 0 });
  };
  useEffect(() => {
    if (box === q.search) return;
    const t = setTimeout(() => set({ search: box }, true), 250);
    return () => clearTimeout(t);
  }, [box]);
  const clearSel = () => { setSel(new Map()); setAll(false); setAnchor(-1); };
  const pickView = (v) => { setView(v); prefs()?.set("gr-library-view", v); };
  const pickSize = (v) => { setPx(v); prefs()?.set("gr-library-size", v); };
  const count = all ? games.total : sel.size;
  const selected = (k) => all || sel.has(k);
  const open = (g, i) => {
    Object.assign(opened, { query: { ...q }, index: i, total: games.total });
    location.hash = gameHash(g.key);
  };
  const pick = async (e, g, i) => {
    if (e.shiftKey && anchor >= 0) {
      const [a, b] = anchor < i ? [anchor, i] : [i, anchor];
      const got = await games.range(a, b);
      const next = e.ctrlKey || e.metaKey ? new Map(sel) : new Map();
      for (const x of got) next.set(x.key, x);
      setAll(false);
      setSel(next);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      const next = new Map(all ? [] : sel);
      next.has(g.key) ? next.delete(g.key) : next.set(g.key, g);
      setAll(false);
      setSel(next);
      setAnchor(i);
      return;
    }
    setAll(false);
    setSel(new Map([[g.key, g]]));
    setAnchor(i);
  };
  /** The games selected, every one (asked of the core when everything is). */
  const chosen = async () => (all ? await api("browse_keys", { query: q }) : [...sel.values()]);
  const one = !all && sel.size === 1 ? [...sel.values()][0] : null;
  const scrape = async () => {
    const list = (await chosen()).filter((g) => g.kind === "dat" && g.dat_game);
    if (!list.length) { toast("Scrape… is for the systems' games; a PC game is identified on its page.", 8000); return; }
    const first = list[0];
    scrapeDialog({ platform: first.platform, platformName: first.platform_name, games: list.map((g) => g.dat_game), gameName: list.length === 1 ? first.title || first.name : plural(list.length, "game", "games") });
  };
  const convert = async () => {
    const list = (await chosen()).filter((g) => g.kind === "dat");
    const systems = [...new Set(list.map((g) => g.platform))];
    if (systems.length !== 1) { toast("Convert… works on one system's games at a time: pick games of one system.", 8000); return; }
    const dat = overview?.dats?.[systems[0]];
    convertDialog({ dat, platform: systems[0], platformName: list[0].platform_name || systems[0], games: list.map((g) => g.name) });
  };
  const showIn = async (g) => {
    const x = await api("browse_game", { key: g.key });
    const path = x.files?.[0]?.path || x.folder_path || x.dir;
    if (path) reveal(path); else toast("It has no files in the library.", 6000);
  };
  const oneSystem = all ? q.platform.length === 1 : new Set([...sel.values()].map((g) => g.platform)).size === 1;
  const formats = (p) => !!ui.get().platforms?.find((x) => x.id === p)?.formats?.length;
  const actions = [
    { id: "open", label: "Open", icon: "external", one: true, run: () => one && open(one, anchor) },
    { id: "add", label: "Add to a collection…", icon: "collection", disabled: off, run: async () => ui.set({ dialog: { type: "lib-add", games: await chosen() } }) },
    { id: "scrape", label: "Scrape…", icon: "scrape", disabled: off, run: scrape },
    { id: "convert", label: "Convert…", icon: "convert", disabled: off ? true : !oneSystem ? "Pick games of one system to convert them" : false, run: convert, hide: !(one ? formats(one.platform) : true) },
    { id: "attach", label: "Attach documents…", icon: "file", one: true, disabled: off || !isDesktop() || one?.kind !== "dat", run: () => one && attachDocs({ game: one.dat_game, gameName: one.title, platform: one.platform }) },
    { id: "reveal", label: "Show in folder", icon: "folder", one: true, disabled: !isDesktop(), run: () => one && showIn(one) },
  ].filter((a) => !a.hide);
  const barItems = () => actions.filter((a) => count === 1 || !a.one).map((a) => ({ ...a, run: () => a.run() }));
  const onMenu = (e, g, i) => {
    e.preventDefault();
    if (!selected(g.key)) { setAll(false); setSel(new Map([[g.key, g]])); setAnchor(i); }
    const single = !selected(g.key) || count <= 1;
    const items = actions.filter((a) => single || !a.one).map((a) => ({ ...a, run: a.id === "open" ? () => open(g, i) : a.one ? () => (a.id === "reveal" ? showIn(g) : a.id === "attach" ? attachDocs({ game: g.dat_game, gameName: g.title, platform: g.platform }) : a.run()) : a.run }));
    openMenu(e, items);
  };
  usePageKeys((e) => {
    if (e.key === "Escape" && count) { clearSel(); return; }
    if (selectAllKey(e)) { e.preventDefault(); setAll(true); return; }
    if (e.key === "Enter" && one) { open(one, anchor); return; }
    if (e.key === "ContextMenu" && count) { const el = document.querySelector(".lib-canvas .selected"); if (el) { const r = el.getBoundingClientRect(); openMenu({ clientX: r.left + 20, clientY: r.top + 20 }, barItems()); } }
  });
  const more = () => [
    { head: "Cover size" },
    ...Object.keys(SIZES).map((k) => ({ id: `size-${k}`, label: k[0].toUpperCase() + k.slice(1), icon: px === k ? "check" : null, run: () => pickSize(k), disabled: view !== "grid" ? "In the grid" : false })),
    { sep: true },
    { id: "lib-save", label: "Save as a collection…", icon: "collection", disabled: off || !filtered(q) ? (off ? true : "Choose some filters first") : false, run: () => ui.set({ dialog: { type: "lib-save", q } }) },
    { id: "lib-scrape-shown", label: `Scrape the ${plural(games.total, "game", "games")} shown…`, icon: "scrape", disabled: off || !games.total, run: async () => { const list = (await api("browse_keys", { query: q })).filter((g) => g.dat_game); if (list.length) scrapeDialog({ platform: list[0].platform, platformName: "", games: list.map((g) => g.dat_game), gameName: plural(list.length, "game", "games") }); } },
    { id: "lib-rename-folders", label: "Rename game folders to their DAT names…", icon: "edit", disabled: off, run: renameFolders },
  ];
  const search = html`<input type="search" id="lib-search" data-search="1" class="compact" placeholder="Search titles, developers, descriptions" aria-label="Search the library" value=${box} onInput=${(e) => setBox(e.target.value)} />`;
  const tools = html`<${ViewSwitch} id="lib-view" views=${VIEWS} value=${view} onChange=${pickView} />
    <${SortMenu} id="lib-sort" options=${SORTS} value=${q.sort} onChange=${(sort) => set({ sort, desc: false })} />
    <button type="button" class="ghost compact" id="lib-desc" title=${q.desc ? "Reversed: back to the usual order" : "Reverse the order"} aria-pressed=${q.desc ? "true" : "false"} onClick=${() => set({ desc: !q.desc })}>${q.desc ? "↑" : "↓"}</button>
    <${MenuButton} id="lib-group" label=${`Group: ${GROUPS.find(([k]) => k === q.group)?.[1]}`} icon="grouped" items=${() => GROUPS.map(([k, l]) => ({ id: `group-${k || "none"}`, label: l, icon: q.group === k ? "check" : null, run: () => set({ group: k }) }))} />`;
  const head = html`<${CommandBar} id="lib-head" title="Library" count=${games.loading ? null : plural(games.total, "game", "games")} search=${search} tools=${tools}
    more=${{ id: "lib-menu", title: "Cover size, save as a collection, scrape, rename folders", items: more }}
    selection=${count ? { count: `${count.toLocaleString()} selected`, onClear: clearSel, items: barItems, buttons: 2 } : null} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (overview?.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${overview.error}</p></div>`;
  if (overview && !overview.all) {
    return html`${head}<div class="pages"><div class="library-settings" id="lib-empty">
      <p class="page-lead">${Icon.grid(18)} Every game the library holds, across its systems, by its pictures and details.</p>
      <section class="ls-section"><h2>To start</h2>
        <p>Add a folder with games and import a DAT for its system (<a href=${routeHash("audit")}>Audit</a>), or import PC games (<a href=${routeHash("pc")}>PC games</a>). Scraping (<a href=${routeHash("metadata")}>Metadata</a>) brings their pictures and details.</p></section>
    </div></div>`;
  }
  const keyOf = one ? one.key : null;
  return html`${head}<div class="lib-page" id="lib-page" data-view=${view}>
    <${Filters} q=${q} facets=${games.facets} set=${set} collections=${collections} />
    <div class="lib-main">
      <${Chips} q=${q} set=${set} facets=${games.facets} collections=${collections} />
      ${games.error ? html`<p class="warn-note" role="alert">${games.error}</p>` : null}
      <div class="lib-scroll" ref=${scrollRef} aria-busy=${games.loading ? "true" : "false"} onClick=${(e) => { if (e.target === e.currentTarget || e.target.classList.contains("lib-canvas")) clearSel(); }}>
        ${!games.loading && !games.total ? html`<p class="muted lib-none" id="lib-none">${filtered(q) ? "No game matches these filters." : q.all ? "No games in the DATs." : "No games in the library yet. Every game in the DATs shows those you don't have."}</p>` : null}
        ${overview ? html`<${Games} games=${games} q=${q} set=${set} view=${view} px=${px} selected=${selected} onPick=${pick} onOpen=${open} onMenu=${onMenu} scrollRef=${scrollRef} />` : html`<p class="muted lib-none">Reading the library…</p>`}
      </div>
    </div>
    <${Inspector} count=${count} keyOf=${keyOf} actions=${actions} overview=${overview} off=${off} />
  </div>`;
}

/** Rename the game folders a DAT's new version renamed the games of (D45): the preview,
 *  then Run. */
export async function renameFolders() {
  try {
    const plan = await api("organise_rename_folders");
    if (!plan.ops) { toast(plan.left ? `Nothing to rename: ${plural(plan.left, "folder is", "folders are")} left as they are (see why in the preview).` : "Every game folder has its game's name already.", 8000); if (!plan.left) return; }
    ui.set({ dialog: { type: "organise", plan, platformName: "" } });
  } catch (e) {
    toast(`Couldn't plan it: ${err(e)}`, 8000);
  }
}
