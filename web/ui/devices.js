// The Devices page (docs/PLAN.md, "Save sync and the Devices page", D187–D188): every device
// Game Repo knows down the side, and the one chosen with what it held when it last connected,
// by system, and the changes waiting for it. Add games… and Remove queue changes, which the
// device takes the next time it connects (Game Repo Go when it reaches the library); a change
// not taken yet can be cancelled. A game's page says which devices hold it, with Send to a
// device… (`GameDevices`). A card, a drive or a share is a folder device (D189): Add a card or
// drive… marks it, Game Repo knows it again wherever it comes back, and while Game Repo runs
// its waiting changes are made as soon as it's connected, as a change with Undo.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { routeHash, ctx } from "./context.js";
import { api, toast, size, plural, followJob, isDesktop } from "./library.js";
import { undoChange } from "./organise.js";
import { CommandBar } from "./layout.js";
import { useSidePanel, useRouteArg, argHash } from "./panels.js";
import { Dialog, close } from "./dialogs.js";
import { gameHash } from "./browse.js";
import { Icon } from "./icons.js";

const err = (e) => e.message || String(e);
const changed = () => ui.set((s) => ({ changes: (s.changes || 0) + 1 }));
export const KIND = { android: "Android", windows: "Windows PC", linux: "Linux PC", steamdeck: "Steam Deck", card: "SD card", test: "Device" };
const kindName = (k) => KIND[k] || k || "Device";
const seenText = (secs) => (secs ? new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "never");
const STATE = { waiting: "Waiting for its next connect", taken: "In its downloads", done: "Done", failed: "Didn't work" };

/** Changes for games ({ key, title, platform, size }) queued for a device, said with what's next. */
export async function sendToDevice(device, games) {
  const r = await api("devices_queue", { id: device.id, changes: games.map((g) => ({ action: "add", key: g.key, title: g.title || g.key, system: g.platform || g.key.split("/")[0], size: g.size || 0 })) });
  changed();
  toast(r.added ? `${games.length === 1 ? games[0].title || "It" : plural(r.added, "game", "games")} will go to ${device.name} the next time it connects.` : `That's waiting for ${device.name} already.`, 8000);
  return r;
}

/** Add games…: found by title in the Library, ticked, their size against the device's free space. */
function AddDialog({ device, free, has = [], onDone }) {
  const [q, setQ] = useState("");
  const [found, setFound] = useState(null);
  const [picked, setPicked] = useState(() => new Map());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (q.trim().length < 2) { setFound(null); return; }
    let gone = false;
    // the Library brought up to date first, as its page does
    const t = setTimeout(() => api("browse").then(() => api("browse_query", { query: { search: q, one_per_title: false, limit: 60 } }))
      .then((r) => { if (!gone) setFound((r.items || []).filter((g) => !g.key.startsWith("pc/") || device.kind !== "android")); }).catch((e) => { if (!gone) setFound({ error: err(e) }); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [q]);
  const toggle = (g) => setPicked((m) => { const n = new Map(m); n.has(g.key) ? n.delete(g.key) : n.set(g.key, g); return n; });
  const total = [...picked.values()].reduce((n, g) => n + (g.size || 0), 0);
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      await sendToDevice(device, [...picked.values()]);
      close();
      onDone?.();
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  const list = Array.isArray(found) ? found : [];
  return html`<${Dialog} title=${`Add games to ${device.name}`} id="device-add-dialog" wide=${true} onSubmit=${submit} busy=${busy || !picked.size} error=${error} submitLabel=${picked.size ? `Add ${plural(picked.size, "game", "games")}` : "Add"}>
    <p class="muted">They go to ${device.name} the next time it connects${device.by === "go" ? " (when Game Repo Go there reaches the library)" : ""}.</p>
    <div class="pick-row"><input type="search" id="device-add-search" class="compact" placeholder="Search the Library by title" aria-label="Search the Library by title" value=${q} onInput=${(e) => setQ(e.target.value)} /></div>
    ${found?.error ? html`<p class="warn-note" role="alert">${found.error}</p>` : null}
    ${found === null ? html`<p class="muted">Type two letters or more of a title.</p>` : null}
    <ul class="ls-list organise-list" id="device-add-list">${list.map((g) => html`<li key=${g.key} data-key=${g.key}>
      <label class="check-row"><input type="checkbox" checked=${picked.has(g.key)} disabled=${has.includes(g.key)} onChange=${() => toggle(g)} /> ${g.title}</label>
      <span class="muted">${[g.platform_name, g.year, g.size ? size(g.size) : null, has.includes(g.key) ? "on the device" : null].filter(Boolean).join(" · ")}</span></li>`)}</ul>
    ${Array.isArray(found) && !found.length ? html`<p class="muted">No game matches.</p>` : null}
    ${picked.size ? html`<p class=${total > free ? "warn-note" : "muted"} id="device-add-room">${size(total)} to add; ${size(free)} free on ${device.name} when it last connected${total > free ? ": it won't all fit" : ""}.</p>` : null}
  <//>`;
}

/** Add a card or drive…: its folder, a name, and the layout its games are laid out by. */
function FolderDialog() {
  const [layouts, setLayouts] = useState([]);
  const [folder, setFolder] = useState("");
  const [name, setName] = useState("");
  const [layout, setLayout] = useState("esde-card");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { api("devices_layouts").then(setLayouts).catch((e) => setError(err(e))); }, []);
  const pick = async () => {
    const f = await ctx.platform.library.pickFolder("The card, drive or folder");
    if (f) setFolder(f);
  };
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const d = await api("devices_folder_add", { folder: folder.trim(), name: name.trim(), layout });
      close();
      changed();
      location.hash = argHash("devices", d.id);
      toast(`${d.name} is a device now: Game Repo knows it again whenever it's connected.`, 8000);
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title="Add a card or drive" id="device-folder-dialog" onSubmit=${submit} busy=${busy || !folder.trim() || !name.trim()} error=${error} submitLabel="Add">
    <p class="muted">An SD card in a reader, a handheld that shows as a drive, an external drive or a share. Game Repo writes a small file at its top, <code>.gamerepo/device.json</code>, to know it again.</p>
    <label class="field">Its folder<div class="pick-row"><input id="device-folder" class="compact" value=${folder} onInput=${(e) => setFolder(e.target.value)} placeholder=${isDesktop() ? "E:\\" : "/media/card"} />
      ${isDesktop() ? html`<button type="button" class="ghost compact" id="device-folder-choose" onClick=${pick}>Choose…</button>` : null}</div></label>
    <label class="field">Its name<input id="device-folder-name" class="compact" value=${name} onInput=${(e) => setName(e.target.value)} placeholder="Handheld's card" /></label>
    <label class="field">What's on it<select id="device-folder-layout" class="compact" value=${layout} onChange=${(e) => setLayout(e.target.value)}>
      ${layouts.map((l) => html`<option key=${l.id} value=${l.id}>${l.name}</option>`)}</select></label>
  <//>`;
}

export function DeviceDialogs({ dialog }) {
  if (dialog.type === "device-add") return html`<${AddDialog} ...${dialog} />`;
  if (dialog.type === "device-folder") return html`<${FolderDialog} />`;
  return null;
}

// ---- folder devices found while Game Repo runs (D189)

let present = [];
const syncing = new Set();

/** A folder device's waiting changes made now, said with Undo. */
export async function syncFolder(d, auto = false) {
  if (syncing.has(d.id)) return;
  syncing.add(d.id);
  try {
    const { job } = await api("devices_folder_sync", { id: d.id, auto });
    const done = await followJob(job, `Changing the games on ${d.name}`);
    changed();
    if (done.error) throw new Error(done.error);
    const r = done.result || {};
    if (r.nothing) { if (!auto) toast(`${d.name} read again: ${plural(r.games, "game", "games")}, ${size(r.free)} free.`, 6000); return; }
    const failed = r.failed?.length ? ` ${plural(r.failed.length, "step", "steps")} failed: ${r.failed[0].error}` : "";
    toast(`Changed the games on ${d.name}: ${plural(r.games, "game", "games")} on it now, ${size(r.free)} free.${failed}`, { ms: 12000, action: r.id ? { label: "Undo", run: async () => { await undoChange(r.id); changed(); } } : null });
  } catch (e) {
    toast(`Couldn't change the games on ${d.name}: ${err(e)}`, 10000);
  } finally {
    syncing.delete(d.id);
  }
}

/** Every few seconds, the folder devices that are here; one that's just come with changes
 *  waiting gets them. */
export async function lookForDevices() {
  if (!ui.get().library || ui.get().library.read_only) return;
  let now;
  try { now = await api("devices_present"); } catch { return; }
  const was = new Set(present.map((p) => p.id));
  present = now;
  ui.set({ devicesHere: now });
  if (!now.some((p) => !was.has(p.id))) return;
  const all = await api("devices_list").catch(() => []);
  for (const p of now.filter((x) => !was.has(x.id))) {
    const d = all.find((x) => x.info.id === p.id);
    if (d?.waiting) syncFolder(d.info, true);
  }
}

/** One device's page. */
function DeviceView({ id, off }) {
  const rev = useStore(ui, (s) => s.changes);
  const here_ = useStore(ui, (s) => s.devicesHere);
  const [v, setV] = useState(null);
  const [q, setQ] = useState("");
  const [picked, setPicked] = useState(() => new Set());
  useEffect(() => { setV(null); setPicked(new Set()); }, [id]);
  useEffect(() => { api("devices_view", { id }).then(setV).catch((e) => setV({ error: err(e) })); }, [id, rev]);
  if (!v) return html`<p class="muted">Reading…</p>`;
  if (v.error) return html`<p class="warn-note" role="alert">${v.error}</p>`;
  const d = v.info;
  const games = v.contents.games || [];
  const going = new Set(v.changes.filter((c) => c.state === "waiting" && c.change.action === "remove").map((c) => c.change.game || c.change.key));
  const shown = games.filter((g) => !q.trim() || g.name.toLowerCase().includes(q.trim().toLowerCase()));
  const systems = [...new Set(shown.map((g) => g.system))].sort();
  const waiting = v.changes.filter((c) => c.state === "waiting");
  const queue = async (changes) => {
    try {
      const r = await api("devices_queue", { id, changes });
      setV(r);
      setPicked(new Set());
      changed();
      toast(`${plural(changes.length, "game", "games")} will be removed from ${d.name} the next time it connects.`, 8000);
    } catch (e) { toast(err(e), 8000); }
  };
  const removeTicked = () => queue(games.filter((g) => picked.has(g.id)).map((g) => ({ action: "remove", key: g.key || "", game: g.id, title: g.name, system: g.system, size: g.size })));
  const cancel = async (ids) => {
    try { setV(await api("devices_cancel", { id, ids })); changed(); } catch (e) { toast(err(e), 8000); }
  };
  const tick = (g) => setPicked((s) => { const n = new Set(s); n.has(g.id) ? n.delete(g.id) : n.add(g.id); return n; });
  const here = d.by === "folder" ? (here_ || []).find((p) => p.id === d.id) : null;
  const read = async () => {
    try { setV(await api("devices_folder_read", { id })); changed(); toast(`${d.name} read again.`); } catch (e) { toast(err(e), 8000); }
  };
  return html`<div class="device-view" id="device-view" data-id=${d.id}>
    <p class="muted" id="device-line">${[kindName(d.kind), `last connected ${seenText(d.seen)}`, plural(d.games, "game", "games"), size(d.size), `${size(d.free)} free`].join(" · ")}</p>
    ${d.by === "folder" ? html`<p id="device-here" data-here=${here ? "yes" : "no"} class=${here ? null : "muted"}>${here ? html`Connected at <code>${here.at}</code>. <button type="button" class="ghost compact" id="device-sync" disabled=${off} onClick=${() => syncFolder(d)}>${waiting.length ? `Make the ${plural(waiting.length, "change", "changes")} now` : "Read it again"}</button>
      ${waiting.length ? html` <button type="button" class="linkish" id="device-read" disabled=${off} onClick=${read}>Read it again</button>` : null}`
      : `Not connected. Put the card in, or plug the drive in, and Game Repo finds it${d.at ? ` (it was at ${d.at})` : ""}.`}</p>` : null}
    <section class="ls-section" id="device-waiting"><h2>Waiting for its next connect</h2>
      ${v.changes.length ? html`<ul class="ls-list">${v.changes.map((c) => html`<li key=${c.change.id} data-change=${c.change.id} data-state=${c.state} data-action=${c.change.action}>
        <span class="op-paths"><span class="op-to">${c.change.action === "add" ? "Add" : "Remove"} ${c.change.key ? html`<a href=${gameHash(c.change.key)}>${c.change.title}</a>` : c.change.title}</span>
          <span class="muted">${[STATE[c.state] || c.state, c.change.size ? size(c.change.size) : null, c.note || null].filter(Boolean).join(" · ")}</span></span>
        ${c.state === "waiting" ? html`<button type="button" class="ghost compact" data-action="cancel" disabled=${off} onClick=${() => cancel([c.change.id])}>Cancel</button>` : null}</li>`)}</ul>
        ${waiting.length ? html`<p class="muted" id="device-waiting-note">${plural(waiting.length, "change waits", "changes wait")} for ${d.name}: ${d.by === "go" ? "Game Repo Go there takes them when it next reaches the library." : "they're made when it's next connected."}</p>` : null}`
        : html`<p class="muted">Nothing. Add games, or tick games below and remove them: they're made the next time ${d.name} connects.</p>`}
    </section>
    <section class="ls-section" id="device-games"><h2>On the device</h2>
      <div class="pick-row"><input type="search" id="device-search" class="compact" placeholder="Search its games" aria-label="Search its games" value=${q} onInput=${(e) => setQ(e.target.value)} />
        <button type="button" class="ghost compact" id="device-remove" disabled=${off || !picked.size} onClick=${removeTicked}>${Icon.trash(14)} Remove ${picked.size ? plural(picked.size, "game", "games") : ""}</button></div>
      ${games.length ? systems.map((sys) => html`<div class="device-system" key=${sys} data-system=${sys}><h3>${sys} <span class="muted">${plural(shown.filter((g) => g.system === sys).length, "game", "games")}</span></h3>
        <ul class="ls-list">${shown.filter((g) => g.system === sys).map((g) => html`<li key=${g.id} data-id=${g.id} data-going=${going.has(g.id) || going.has(g.key) ? "yes" : "no"}>
          <label class="check-row"><input type="checkbox" checked=${picked.has(g.id)} disabled=${off || going.has(g.id)} onChange=${() => tick(g)} /> ${g.key ? html`<a href=${gameHash(g.key)}>${g.name}</a>` : g.name}</label>
          <span class="muted">${[size(g.size), g.from === "elsewhere" ? "not from the library" : null, g.last_played ? `played ${g.last_played.slice(0, 10)}` : null, going.has(g.id) ? "to be removed" : null].filter(Boolean).join(" · ")}</span></li>`)}</ul></div>`)
        : html`<p class="muted">It held no games when it last connected.</p>`}
    </section>
  </div>`;
}

export function Devices() {
  const s = useStore(ui, (st) => ({ library: st.library, changes: st.changes }));
  const off = !!s.library?.read_only;
  const id = useRouteArg("devices");
  const [all, setAll] = useState(null);
  useEffect(() => { if (s.library) api("devices_list").then(setAll).catch((e) => setAll({ error: err(e) })); }, [s.library?.path, s.changes]);
  const list = Array.isArray(all) ? all : [];
  const cur = list.find((x) => x.info.id === id)?.info || (!id ? list[0]?.info : null);
  useSidePanel(Array.isArray(all) ? {
    id: "device-panel", route: "devices", title: "Devices", icon: "monitor", count: list.length, current: cur?.id,
    items: list.map((x) => ({ id: x.info.id, label: x.info.name, sub: [kindName(x.info.kind), x.waiting ? `${x.waiting} waiting` : null].filter(Boolean).join(" · "), href: argHash("devices", x.info.id) })),
  } : null);
  const forget = async () => {
    try {
      await api("devices_forget", { id: cur.id });
      changed();
      location.hash = routeHash("devices");
      toast(`Forgot ${cur.name}. Its saves stay in the library.`, 6000);
    } catch (e) { toast(err(e), 8000); }
  };
  const add = async () => {
    const v = await api("devices_view", { id: cur.id });
    ui.set({ dialog: { type: "device-add", device: cur, free: cur.free, has: (v.contents.games || []).map((g) => g.key).filter(Boolean) } });
  };
  const addFolder = () => ui.set({ dialog: { type: "device-folder" } });
  const head = html`<${CommandBar} id="devices-head" title=${html`<h1 id="devices-title">${cur?.name || "Devices"}</h1>`}
    primary=${cur ? html`<button type="button" class="primary compact" id="device-add" disabled=${off} onClick=${add}>${Icon.plus(15)} Add games…</button>` : null}
    more=${{ id: "device-menu", title: "More", items: () => [
      { id: "device-add-folder", label: "Add a card or drive…", icon: "plus", disabled: off, run: addFolder },
      ...(cur ? [{ id: "device-forget", label: "Forget this device", icon: "trash", disabled: off, run: forget }] : []),
    ] }} />`;
  if (!s.library) return html`${head}<div class="pages"><p class="page-lead">Open a library first.</p></div>`;
  if (all?.error) return html`${head}<div class="pages"><p class="warn-note" role="alert">${all.error}</p></div>`;
  if (!all) return html`${head}<div class="pages"><p class="muted">Reading…</p></div>`;
  return html`${head}<div class="pages devices-page" id="devices-page">
    ${cur ? html`<${DeviceView} id=${cur.id} off=${off} key=${cur.id} />` : html`<div id="devices-none">
      <p class="page-lead">No devices yet.</p>
      <p class="muted">A phone, a Steam Deck or a PC with Game Repo Go shows here once it's paired (Settings › Serve to devices › Pair a device…) and has reached the library: what it holds, and the games to add to it or remove, which it takes the next time it connects.</p>
      <p class="muted">An SD card, a drive or a share is a device too: Game Repo lays its games out for ES-DE, muOS, Knulli or ROCKNIX, and makes the changes you ask for whenever it's connected.</p>
      <div class="home-actions"><a class="button ghost" href=${routeHash("settings")} id="devices-to-settings">Open Settings</a>
        <button type="button" class="ghost" id="devices-add-folder" disabled=${off} onClick=${addFolder}>Add a card or drive…</button></div></div>`}
  </div>`;
}

/** A game's page: the devices that held it when last connected, and Send to a device…. */
export function GameDevices({ g, off }) {
  const rev = useStore(ui, (s) => s.changes);
  const [all, setAll] = useState(null);
  const [holding, setHolding] = useState([]);
  const [to, setTo] = useState("");
  useEffect(() => {
    api("devices_list").then(setAll).catch(() => setAll([]));
    api("devices_holding", { key: g.key }).then(setHolding).catch(() => setHolding([]));
  }, [g.key, rev]);
  if (!Array.isArray(all) || !all.length) return null;
  const others = all.map((x) => x.info).filter((d) => !holding.some((h) => h.id === d.id) && (d.kind !== "android" || g.kind !== "pc"));
  const send = async () => {
    const d = all.find((x) => x.info.id === to)?.info;
    if (!d) return;
    try { await sendToDevice(d, [{ key: g.key, title: g.title, platform: g.platform, size: g.size }]); setTo(""); } catch (e) { toast(err(e), 8000); }
  };
  return html`<section class="ls-section" id="game-devices"><h2>Devices</h2>
    <p class=${holding.length ? null : "muted"} id="game-devices-line">${holding.length ? html`On ${holding.map((h, i) => html`${i ? ", " : ""}<a key=${h.id} href=${argHash("devices", h.id)}>${h.name}</a>`)}.` : "On none of your devices."}</p>
    ${others.length ? html`<div class="pick-row"><select id="game-device-to" class="compact" aria-label="Send to a device" value=${to} onChange=${(e) => setTo(e.target.value)}>
        <option value="">Send to a device…</option>${others.map((d) => html`<option key=${d.id} value=${d.id}>${d.name}</option>`)}</select>
      <button type="button" class="ghost compact" id="game-device-send" disabled=${off || !to} onClick=${send}>Send</button></div>` : null}
  </section>`;
}
