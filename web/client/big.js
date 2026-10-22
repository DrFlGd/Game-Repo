// The big-screen view (docs/PLAN.md, "Phase 9e design", D113), after Steam's Big Picture:
// the pages as tabs along the top, changed with the shoulder buttons (L1 and R1); the button
// hints along the bottom, saying what the focused thing does; the Library as shelves of box
// fronts, one per system, a system's every game a grid (the classic view's, larger); a game's
// page with its picture large behind it; Options (☰) and the lists (a <select>) as panels
// from the right, as every dialog is. Downloads, On this device and Settings are the classic
// view's pages, made larger (client.css). Touch works on all of it, for phones, where the
// tabs go along the bottom instead.
import { html, useState, useEffect, useRef } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call, info, keyboard, pictureUrl } from "./backend.js";
import { back, closeDialog, go, open, routeOf, say, setLook, showDialog } from "./state.js";
import { bytes, count, kindName, message, plural } from "./format.js";
import { Dialog } from "./dialog.js";
import { enqueue } from "./add.js";
import { LIBRARY_ORDERS } from "./library.js";
import { DEVICE_ORDERS } from "./device.js";
import { focusFirst, show } from "./keys.js";

export const TABS = [
  ["library", "Library", Icon.gamepad],
  ["queue", "Downloads", Icon.download],
  ["device", "On this device", Icon.monitor],
  ["settings", "Settings", Icon.cog],
];
const tabOf = (page) => (page === "game" || page === "games" ? "library" : page);
const SHELF = 15;   // games on a shelf before its All

// ---- the controller's buttons (keys.js calls these) ------------------------------------

/** L1 and R1: the page before or after (from the address, which a press changes at once:
 *  the route follows on "hashchange", after a second quick press). */
export function page(by) {
  const now = tabOf(routeOf(location.hash).page);
  const here = TABS.findIndex(([p]) => p === now);
  const next = TABS[Math.max(0, Math.min(TABS.length - 1, here + by))][0];
  if (next !== now) open({ page: next });
}

/** Once `sel` is in the page (or after 2 seconds), focus it and run `then`. */
function focusSoon(sel, then) {
  let n = 0;
  const step = () => {
    const el = document.querySelector(sel);
    if (el) { show(el); then?.(el); } else if (n++ < 120) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

/** Y: the search, over every game; in Game Mode with Steam's keyboard up. */
export function search() {
  if (go.get().route.page !== "games") {
    go.set({ where: { system: "", collection: "", group: null }, q: "" });
    open({ page: "games" });
  }
  focusSoon("#go-search", () => { if (info.game_mode) keyboard(); });
}

export const options = () => showDialog({ type: "options" });

/** A: a list opens as a panel of its choices (a controller can't work the system's own). */
export const choose = (select) => showDialog({ type: "choose", select });

// ---- the frame ---------------------------------------------------------------------------

export function BigBar({ route, status }) {
  const here = tabOf(route.page);
  const q = status?.queue;
  const n = q?.left || q?.queued || 0;
  const busy = q && q.left > 0;
  const pct = busy && q.size ? Math.floor((q.done * 100) / q.size) : 0;
  const free = status?.device?.free;
  const name = TABS.find(([p]) => p === here)?.[1] || "";
  return html`<header class="go-big-bar" id="go-big-bar">
    <a class="go-big-brand" href="#library" tabindex="-1">Game Repo Go</a>
    <span class="go-big-here">${name}</span>
    <nav class="go-tabs" aria-label="Pages">
      <button type="button" class="go-pad go-pad-shoulder" tabindex="-1" title="The page before" onClick=${() => page(-1)}>L1</button>
      ${TABS.map(([p, label, icon]) => html`<a class="go-tab" href=${`#${p}`} id=${`go-nav-${p}`} tabindex="-1" aria-current=${here === p ? "page" : null}>
        ${icon(22)}<span>${label}</span>${p === "queue" && n ? html`<span class="go-nav-n" aria-label=${q.left ? `${n} to come` : `${n} queued`}>${n}</span>` : null}</a>`)}
      <button type="button" class="go-pad go-pad-shoulder" tabindex="-1" title="The page after" onClick=${() => page(1)}>R1</button>
    </nav>
    ${busy ? html`<a class="go-big-free go-bar-queue" href="#queue" tabindex="-1" id="go-bar-queue">${q.offline ? "Waiting for the library" : `Downloading ${q.left} · ${pct}%`}</a>`
      : free != null ? html`<span class="go-big-free">${bytes(free)} free</span>` : null}
    ${here === "library" ? html`<button type="button" class="go-big-search" tabindex="-1" aria-label="Search every game" onClick=${search}>${Icon.search(20)}</button>` : null}
  </header>`;
}

const typing = (t) => t?.matches?.('input:not([type="checkbox"]):not([type="radio"]):not([type="range"]), textarea');

/** The button hints along the bottom: what A, X, Y, ☰ and B do for what's focused. Each is a
 *  button too, for a mouse or a touch. */
export function Hints({ route, dialog }) {
  const [, redraw] = useState(0);
  useEffect(() => {
    const again = () => redraw((n) => n + 1);
    const later = () => setTimeout(again, 0);
    document.addEventListener("focusin", again);
    document.addEventListener("focusout", later);
    document.addEventListener("keyup", later);
    document.addEventListener("click", later);
    return () => {
      document.removeEventListener("focusin", again);
      document.removeEventListener("focusout", later);
      document.removeEventListener("keyup", later);
      document.removeEventListener("click", later);
    };
  }, []);
  const el = document.activeElement && document.activeElement !== document.body ? document.activeElement : null;
  const at = (k) => el?.closest?.(`[data-${k}]`)?.dataset[k];
  let a = at("a");
  if (a == null && el) {
    a = el.matches("select") ? "Choose" : typing(el) ? "Type" : el.matches('input[type="checkbox"]') ? (el.checked ? "Untick" : "Tick") : el.matches("button, a[href]") ? "Select" : "";
  }
  const x = at("x");
  const say = at("say");
  const home = route.page === "library";
  const library = home || route.page === "games" || route.page === "game";
  const press = (key) => el?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  const hint = (pad, label, run, id) => html`<button type="button" class="go-hint" tabindex="-1" id=${id} onMouseDown=${(e) => e.preventDefault()} onClick=${run}>
    <span class=${`go-pad go-pad-${pad}`}>${pad === "menu" ? "☰" : pad.toUpperCase()}</span>${label}</button>`;
  return html`<footer class="go-hints" id="go-hints">
    <span class="go-hints-say">${dialog ? "" : say || ""}</span>
    ${a ? hint("a", a, () => (el?.matches("select") ? choose(el) : el?.click()), "go-hint-a") : null}
    ${x && !dialog ? hint("x", x, () => press("x"), "go-hint-x") : null}
    ${library && !dialog ? hint("y", "Search", search, "go-hint-y") : null}
    ${!dialog ? hint("menu", "Options", options, "go-hint-menu") : null}
    ${dialog ? hint("b", "Close", closeDialog, "go-hint-b") : !home || info.kind === "android" ? hint("b", "Back", back, "go-hint-b") : null}
  </footer>`;
}

// where the focus was on each page (a game's key or a control's id), to come back to it
const kept = {};
const FOCUSABLE = "button:not([disabled]), a[href], input:not([disabled]), select:not([disabled]), [tabindex]";

/** Keeps the focus: back on a page, where it was; on a new one, its first thing (a game,
 *  Download…), so a controller always has something to move from. Not after a touch or a
 *  click: nothing's focused for those. */
export function useFocus(route) {
  useEffect(() => {
    const note = (e) => {
      const el = e.target;
      if (!el?.closest?.("#go-main")) return;
      const key = el.closest("[data-key]")?.dataset.key;
      kept[location.hash || "#library"] = key ? { key } : el.id ? { id: el.id } : null;
    };
    document.addEventListener("focusin", note);
    return () => document.removeEventListener("focusin", note);
  }, []);
  useEffect(() => {
    if (document.documentElement.dataset.input === "pointer") return undefined;
    const was = kept[location.hash || "#library"];
    // where it was can come a moment after the page (a shelf loads its games once drawn), so
    // it's waited for a second before the page's first thing
    const wait = was ? 60 : 0;
    let n = 0, frame = 0;
    const step = () => {
      n++;
      const main = document.getElementById("go-main");
      const now = document.activeElement;
      if (!main || document.querySelector(".dialog-backdrop") || (now && now !== document.body && now !== main && now.isConnected)) return;
      let el = was?.key ? main.querySelector(`[data-key="${CSS.escape(was.key)}"]`) : was?.id ? document.getElementById(was.id) : null;
      if (el && !el.matches(FOCUSABLE)) el = el.querySelector(FOCUSABLE);
      if (!el && n > wait) el = main.querySelector("[data-first]");
      if (el) show(el);
      else if (n > wait + 30 && focusFirst()) return;
      else if (n < wait + 150) frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [route.page, route.key]);
}

// ---- the Library: shelves -------------------------------------------------------------------

function Tile({ g, place, first }) {
  const [bad, setBad] = useState(false);
  const sub = [g.year, bytes(g.size)].filter(Boolean).join(" · ");
  const x = (e) => {
    if (e.key !== "x" && e.key !== "X") return;
    e.preventDefault();
    if (g.on_device) say(`${g.title} is on this device already.`);
    else enqueue([g.key]);
  };
  return html`<button type="button" class="go-tile" data-key=${g.key} data-first=${first ? "" : null} data-a="Open" data-x=${g.on_device ? null : "Add to queue"}
      data-say=${`${place} › ${g.title} · ${bytes(g.size)}`} aria-label=${`${g.title}, ${sub}${g.on_device ? ", on this device" : ""}`}
      onClick=${() => open({ page: "game", key: g.key })} onKeyDown=${x}>
    <span class="go-cover go-tile-cover">
      <span class="go-cover-title">${g.title}</span>
      ${g.picture && !bad ? html`<img src=${pictureUrl(g.key, g.picture, 256)} alt="" decoding="async" draggable="false" onError=${() => setBad(true)} />` : null}
      ${g.on_device ? html`<span class="go-on-device">On this device</span>` : null}
    </span>
    <span class="go-tile-title">${g.title}</span>
    <span class="go-tile-sub">${sub}</span>
  </button>`;
}

/** A tile that opens a place's every game: a system's All, Every game, a collection. */
function PlaceTile({ where, name, n, label }) {
  const browse = () => { go.set({ where: { system: "", collection: "", group: null, ...where }, q: "" }); open({ page: "games" }); };
  return html`<button type="button" class="go-tile go-tile-place" data-a="Open" data-say=${`${name} · ${plural(n, "game")}`} onClick=${browse}>
    <span class="go-tile-cover"><strong>${label || name}</strong><span>${count(n)}</span></span>
  </button>`;
}

/** One system's shelf: its first games by title, read once it's near the screen. */
function Shelf({ system, first }) {
  const ref = useRef(null);
  const rev = useStore(go, (x) => `${x.libraryRev}/${x.deviceRev}`);
  const [near, setNear] = useState(false);
  const [games, setGames] = useState(null);
  useEffect(() => {
    const io = new IntersectionObserver(([e]) => { if (e.isIntersecting) { setNear(true); io.disconnect(); } },
      { root: document.getElementById("go-main"), rootMargin: "600px 0px" });
    io.observe(ref.current);
    return () => io.disconnect();
  }, []);
  useEffect(() => {
    if (!near) return;
    call("games", { system: system.id, sort: "title", offset: 0, limit: SHELF }).then((r) => setGames(r.items)).catch(() => setGames([]));
  }, [near, rev]);
  return html`<section class="go-shelf" data-shelf=${system.id} ref=${ref}>
    <h2>${system.name} <span class="muted">${plural(system.games, "game")}</span></h2>
    <div class="go-shelf-row">
      ${games ? games.map((g, i) => html`<${Tile} key=${g.key} g=${g} place=${system.name} first=${first && i === 0} />`)
        : [0, 1, 2, 3].map((i) => html`<span class="go-tile is-loading" key=${i} aria-hidden="true"><span class="go-cover go-tile-cover"></span></span>`)}
      <${PlaceTile} where=${{ system: system.id }} name=${system.name} n=${system.games} label="All" />
    </div>
  </section>`;
}

export function Shelves() {
  const s = useStore(go, (x) => ({ systems: x.systems, collections: x.collections, status: x.status }));
  const st = s.status;
  const sync = st?.sync || {};
  const systems = s.systems.filter((x) => x.games > 0);
  const all = st?.catalogue?.games || 0;
  return html`<div class="go-shelves" id="go-shelves">
    ${sync.running ? html`<div class="go-note" id="go-reading" role="status">
        <span>Reading the library: ${count(sync.done)}${sync.of ? ` of ${count(sync.of)}` : ""} games</span>
        ${sync.of ? html`<progress max=${sync.of} value=${sync.done}></progress>` : html`<progress></progress>`}
      </div>`
      : !all && sync.error ? html`<div class="go-note warn-note" role="alert" id="go-sync-error">
        <span>${sync.error} Nothing from the library is kept on this device yet.</span>
        <button type="button" class="ghost" onClick=${() => call("sync").catch(() => {})}>Try again</button>
      </div>` : null}
    ${!systems.length && !sync.running ? html`<p class="empty" id="go-no-games">No games here yet.</p>` : null}
    ${systems.map((x, i) => html`<${Shelf} key=${x.id} system=${x} first=${i === 0} />`)}
    ${all ? html`<section class="go-shelf" data-shelf="places">
      <h2>${s.collections.length ? "Every game and the collections" : "Every game"}</h2>
      <div class="go-shelf-row">
        <${PlaceTile} where=${{}} name="Every game" n=${all} />
        ${s.collections.map((c) => html`<${PlaceTile} key=${c.id} where=${{ collection: c.id }} name=${c.name} n=${c.games} />`)}
      </div>
    </section>` : null}
  </div>`;
}

// ---- a game's page -----------------------------------------------------------------------

function Picture({ k, kind, size, alt, cls }) {
  const [bad, setBad] = useState(false);
  useEffect(() => setBad(false), [k, kind]);
  if (bad || !kind) return null;
  return html`<img class=${cls} src=${pictureUrl(k, kind, size)} alt=${alt} onError=${() => setBad(true)} />`;
}

const list = (v) => (Array.isArray(v) ? v.join(", ") : v);

/** The row On this device has for a game, for Remove… and Add media… on its page. */
async function rowOf(key, then) {
  try {
    const r = await call("on_device");
    const row = r.games.find((x) => x.key === key);
    if (row) then(row);
    else say("This game isn't in ES-DE's folders on this device.");
  } catch (e) {
    say(message(e));
  }
}

export function BigGame({ gameKey }) {
  const s = useStore(go, (x) => ({ systems: x.systems, deviceRev: x.deviceRev }));
  const [g, setG] = useState(null);
  const [error, setError] = useState("");
  useEffect(() => {
    setError("");
    call("game", { key: gameKey }).then(setG).catch((e) => setError(message(e)));
  }, [gameKey, s.deviceRev]);
  const game = g?.game;
  const d = g?.details?.details || {};
  const media = g?.details?.media || [];
  const files = g?.details?.files || [];
  const has = (k) => media.some((m) => m.kind === k);
  const behind = has("fanart") ? "fanart" : has("screenshot") ? "screenshot" : game?.picture;
  const system = game && (s.systems.find((x) => x.id === game.system)?.name || game.system);
  const facts = [["Players", d.players], ["Genres", list(d.genres)], ["Developer", d.developer], ["Publisher", d.publisher], ["Released", d.released]].filter(([, v]) => v);
  return html`<article class="go-big-game" id="go-game">
    ${game ? html`<div class="go-hero-back" aria-hidden="true"><${Picture} k=${game.key} kind=${behind} size=${512} alt="" cls="go-hero-pic" /></div>` : null}
    <div class="go-hero">
      <button type="button" class="ghost go-big-back" id="go-game-back" tabindex="-1" onClick=${back}>${Icon.chevronsLeft(16)} Back</button>
      ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
      ${!g && !error ? html`<p class="muted">Opening…</p>` : null}
      ${g && !game ? html`<p class="empty">The library doesn't have this game any more.</p>` : null}
      ${game ? html`<div class="go-hero-body">
        <div class="go-cover go-hero-cover">
          <span class="go-cover-title">${game.title}</span>
          <${Picture} k=${game.key} kind=${game.picture} size=${512} alt=${`${game.title}'s box`} />
        </div>
        <div class="go-hero-info">
          <span class="go-hero-system">${system}</span>
          <h1 id="go-game-title">${game.title}</h1>
          <p class="go-hero-line">${[game.year, list(d.genres), bytes(game.size), game.discs > 1 ? `${game.discs} discs` : null].filter(Boolean).join(" · ")}</p>
          <div class="go-hero-do">
            ${g.on_device
              ? html`<span class="badge go-badge-on" id="go-game-on">On this device</span>
                  <button type="button" class="primary danger" id="go-game-remove" data-first="" onClick=${() => rowOf(game.key, (row) => showDialog({ type: "remove", asks: [{ id: row.id, key: row.key }], title: `Remove ${row.name}` }))}>Remove…</button>
                  <button type="button" class="ghost" id="go-game-addmedia" onClick=${() => rowOf(game.key, (row) => showDialog({ type: "add-media", row }))}>Add pictures and manuals…</button>`
              : html`<button type="button" class="primary" id="go-game-add" data-first="" data-say="Downloads straight away into ES-DE's folders" onClick=${() => showDialog({ type: "add", keys: [game.key] })}>
                    ${Icon.download(22)} Download… · ${bytes(game.size)}</button>
                  <button type="button" class="ghost" id="go-game-enqueue" data-say="Queued, it waits for Start downloads… in Downloads" onClick=${() => enqueue([game.key])}>Add to queue</button>`}
            ${media.length || files.length ? html`<button type="button" class="ghost" id="go-game-media" onClick=${() => showDialog({ type: "media", title: game.title, media, files })}>Pictures and files…</button>` : null}
          </div>
          ${d.description ? html`<div class="go-hero-text go-scroll" tabindex="0" data-a="" data-say="Up and Down scroll the description">${String(d.description).split(/\n\s*\n/).map((p) => html`<p>${p}</p>`)}</div>` : null}
          ${facts.length ? html`<dl class="go-hero-facts">${facts.map(([k, v]) => html`<div><dt>${k}</dt><dd>${v}</dd></div>`)}</dl>` : null}
          ${!g.details ? html`<p class="muted">The rest of its details show once the library can be reached.</p>` : null}
        </div>
      </div>` : null}
    </div>
  </article>`;
}

/** Media…: the pictures and manuals the library has for a game, and its files, with sizes. */
export function MediaDialog({ title, media, files }) {
  return html`<${Dialog} title=${title} id="media-dialog" onSubmit=${closeDialog} cancelLabel="Close">
    ${media.length ? html`<div class="field-block"><span>Pictures and manuals</span>
      <ul class="go-sizes">${media.map((m) => html`<li key=${m.kind}><span>${kindName(m.kind)}</span><span class="muted">${bytes(m.size)}</span></li>`)}</ul></div>` : null}
    ${files.length ? html`<div class="field-block"><span>Files</span>
      <ul class="go-sizes">${files.map((f) => html`<li key=${f.name}><span class="go-file">${f.name}</span><span class="muted">${bytes(f.size)}</span></li>`)}</ul></div>` : null}
    <p class="muted">Download… chooses which pictures and manuals come with the game.</p>
  <//>`;
}

// ---- Options (☰) and the lists -----------------------------------------------------------

function Option({ label, on = false, run, id }) {
  return html`<button type="button" class=${`go-option${on ? " is-on" : ""}`} id=${id} aria-pressed=${on ? "true" : null} data-first=${on ? "" : null} onClick=${run}>
    <span>${label}</span>${on ? Icon.check(20) : null}</button>`;
}

export function OptionsDialog() {
  const s = useStore(go, (x) => ({ route: x.route, sort: x.sort, desc: x.desc, deviceSort: x.deviceSort }));
  const here = tabOf(s.route.page);
  const set = (patch) => { go.set(patch); closeDialog(); };
  return html`<${Dialog} title="Options" id="options-dialog" onSubmit=${closeDialog} cancelLabel="Close">
    ${here === "library" ? html`<div class="field-block go-options"><span>Order of the games</span>
      ${LIBRARY_ORDERS.map(([v, label]) => { const [sort, d] = v.split("-"); return html`<${Option} key=${v} label=${label} on=${s.sort === sort && s.desc === !!d} run=${() => set({ sort, desc: !!d })} />`; })}
    </div>` : null}
    ${here === "device" ? html`<div class="field-block go-options"><span>Order</span>
      ${DEVICE_ORDERS.map(([v, label]) => html`<${Option} key=${v} label=${label} on=${s.deviceSort === v} run=${() => set({ deviceSort: v })} />`)}
      <${Option} label="Make room…" id="options-room" run=${() => showDialog({ type: "room", run: (need) => go.set({ roomFor: need }) })} />
    </div>` : null}
    <div class="field-block go-options"><span>View</span>
      <${Option} label="Switch to the classic view" id="options-classic" run=${() => { closeDialog(); setLook({ view: "classic" }); say("The classic view, for a mouse and keyboard. Settings › Look brings the big screen back."); }} />
    </div>
  <//>`;
}

/** A list's choices as a panel: chosen, the list changes as if picked from it. */
export function ChooseDialog({ select }) {
  const label = select.closest("label")?.querySelector("span")?.textContent?.trim() || select.getAttribute("aria-label") || "Choose";
  const pick = (o) => {
    select.value = o.value;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    closeDialog();
  };
  return html`<${Dialog} title=${label} id="choose-dialog" onSubmit=${closeDialog}>
    <div class="go-options">
      ${[...select.options].filter((o) => !o.disabled).map((o) => html`<${Option} key=${o.value}
        label=${o.parentElement?.tagName === "OPTGROUP" ? `${o.parentElement.label} · ${o.textContent}` : o.textContent}
        on=${o.value === select.value} run=${() => pick(o)} />`)}
    </div>
  <//>`;
}
