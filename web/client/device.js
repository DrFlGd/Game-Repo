// On this device (D100–D102, D115): every game ES-DE has on the device, with its size (its
// media and manual too), when it was last played and where it came from (the library, another
// computer the library recognises, or elsewhere), the free space at the top, checks to select games,
// Remove… and Make room…. By system first: each system with its games and their size, one
// open at a time, and Remove all… for a system's games. A game's media can be removed or
// added on their own. Removing shows what goes and the room it frees first, and sets files
// aside before deleting them. A game whose files are gone, all or some (a folder changed, a
// disc deleted by hand; D117), is marked, with Download again….
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call, info } from "./backend.js";
import { closeDialog, go, open, say, showDialog } from "./state.js";
import { ago, bytes, count, kindName, message, played, plural } from "./format.js";
import { Dialog } from "./dialog.js";
import { Icon } from "../ui/icons.js";

const FROM = {
  library: ["From the library", ""],
  known: ["The library has it", "Came from elsewhere; the library has the same game"],
  elsewhere: ["From elsewhere", "The library doesn't have this game"],
  unchecked: ["Checking…", "Being checked against the library"],
  missing: ["Files not found", "Game Repo Go added it, but its files aren't where it put them (was a folder changed in Settings?). Remove it to take it off this list, or download it again."],
  partial: ["Files missing", "Some of its files aren't on this device (deleted by hand?). Download it again to put them back, or remove what's left."],
};

/** The orders (also big screen's Options). */
export const DEVICE_ORDERS = [["system", "By system"], ["played", "Played longest ago"], ["size", "Largest"], ["name", "Name"]];

/** Played longest ago first, then those never played by when they came (Make room's order). */
const ORDERS = {
  played: (a, b) => (!a.last_played - !b.last_played) || played(a.last_played) - played(b.last_played) || a.added - b.added || a.name.localeCompare(b.name),
  size: (a, b) => b.size - a.size || a.name.localeCompare(b.name),
  name: (a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }),
  system: (a, b) => a.system.localeCompare(b.system) || a.name.localeCompare(b.name, "en", { sensitivity: "base" }),
};

export function DevicePage() {
  const s = useStore(go, (x) => ({ rev: x.deviceRev, roomFor: x.roomFor, systems: x.systems, status: x.status, sort: x.deviceSort }));
  const [d, setD] = useState(null);
  const [error, setError] = useState("");
  const sort = s.sort;
  const setSort = (deviceSort) => go.set({ deviceSort });
  const [openSys, setOpenSys] = useState(null);
  const [ticked, setTicked] = useState(new Set());
  const [shown, setShown] = useState(null);
  const [room, setRoom] = useState("");
  const load = () => call("on_device").then((r) => { setD(r); setError(""); return r; }).catch((e) => { setError(message(e)); return null; });
  useEffect(() => { load(); }, [s.rev]);
  // Make room, from what's selected in the Library: the games to offer, selected here to change and confirm
  useEffect(() => {
    if (!s.roomFor || !d) return;
    const need = s.roomFor;
    go.set({ roomFor: 0 });
    makeRoom(need);
  }, [s.roomFor, !!d]);
  const makeRoom = (need) => call("make_room", { need }).then((ids) => {
    setTicked(new Set(ids));
    // the games ticked in one list, played longest ago first
    if (ids.length) setSort("played");
    const frees = (d?.games || []).filter((g) => ids.includes(g.id)).reduce((a, g) => a + g.size, 0);
    setRoom(ids.length ? `Selected the ${plural(ids.length, "game")} played longest ago, freeing ${bytes(frees)} of the ${bytes(need)} needed. Change what's selected if you like, then choose Remove….`
      : "There's nothing on this device to remove.");
  }).catch((e) => say(message(e)));
  const name = (id) => s.systems.find((x) => x.id === id)?.name || id;
  if (!d) return html`<section class="go-page go-device">${error ? html`<p class="form-error" role="alert" id="device-error">${error}</p>` : html`<p class="muted">Looking through ES-DE's folders…</p>`}</section>`;
  const rows = [...d.games].sort(ORDERS[sort === "system" ? "played" : sort]);
  const pick = rows.filter((r) => ticked.has(r.id));
  const frees = pick.reduce((a, r) => a + r.size, 0);
  const flip = (id) => setTicked((t) => { const x = new Set(t); x.has(id) ? x.delete(id) : x.add(id); return x; });
  const removed = () => { setTicked(new Set()); setRoom(""); };
  const systems = [...new Set(rows.map((r) => r.system))];
  const row = (r, grouped = false) => {
    const lack = r.missing ? "missing" : r.partial ? "partial" : "";
    const [from, why] = FROM[lack || r.from] || [r.from, ""];
    const open_ = shown === r.id;
    return html`<li class=${`go-row${ticked.has(r.id) ? " is-ticked" : ""}`} key=${r.id} data-id=${r.id}>
      <input type="checkbox" class="go-row-tick" aria-label=${`Select ${r.name}`} checked=${ticked.has(r.id)} onChange=${() => flip(r.id)} />
      <button type="button" class="go-row-open" aria-expanded=${open_ ? "true" : "false"} onClick=${() => setShown(open_ ? null : r.id)}>
        <strong>${r.name}</strong>
        <span class="muted">${grouped ? "" : `${name(r.system)} · `}${r.missing ? "its files aren't on this device" : r.partial ? `${bytes(r.size)}, some of its files missing` : bytes(r.size)} · ${r.last_played ? `played ${ago(played(r.last_played))}` : "never played"}</span>
      </button>
      <span class=${`badge go-from go-from-${lack ? "missing" : r.from}`} title=${why}>${from}</span>
      ${open_ ? html`<div class="go-row-more">
        ${r.media.length ? html`<ul class="go-sizes">${r.media.map(([k, size]) => html`<li key=${k}><span>${kindName(k)}</span><span class="muted">${bytes(size)}</span>
            <button type="button" class="ghost" onClick=${() => showDialog({ type: "remove", asks: [{ id: r.id, key: r.key, kinds: [k] }], media: true, title: `Remove ${kindName(k).toLowerCase()} of ${r.name}` })}>Remove</button></li>`)}</ul>`
          : html`<p class="muted">No pictures or manual on this device.</p>`}
        <div class="go-row-do">
          ${r.key && lack ? html`<button type="button" class="primary go-row-again" onClick=${() => showDialog({ type: "add", keys: [r.key] })}>Download again…</button>` : null}
          ${r.key && !lack ? html`<button type="button" class="ghost" onClick=${() => showDialog({ type: "add-media", row: r })}>Add media…</button>
            <button type="button" class="ghost" onClick=${() => open({ page: "game", key: r.key })}>In the library</button>` : null}
          <button type="button" class="ghost danger-text" onClick=${() => showDialog({ type: "remove", asks: [{ id: r.id, key: r.key }], title: `Remove ${r.name}`, done: removed })}>Remove…</button>
        </div>
        <p class="muted go-row-path">${r.id}${r.files > 1 ? ` and ${plural(r.files - 1, "other file")}` : ""}</p>
      </div>` : null}
    </li>`;
  };
  return html`<section class="go-page go-device" id="go-device">
    <div class="go-page-head">
      <h1>On this device</h1>
      <button type="button" class="ghost" id="device-room" onClick=${() => showDialog({ type: "room", run: makeRoom })}>Make room…</button>
    </div>
    <p class="go-device-sum" id="device-sum"><strong>${bytes(d.free)} free</strong> · ${plural(d.games.length, "game")} · ${bytes(d.size)}</p>
    ${d.checking?.running ? html`<p class="muted" id="device-checking" role="status">Checking ${count(d.checking.of - d.checking.done)} games from elsewhere against the library…</p>` : null}
    ${d.checking?.error ? html`<p class="muted">${d.checking.error} Games from elsewhere are checked once it can.</p>` : null}
    ${room ? html`<p class="go-note" id="device-room-note" role="status">${room}</p>` : null}
    ${d.games.length ? html`<label class="go-sort"><span class="muted">Order</span>
      <select id="device-sort" value=${sort} onChange=${(e) => setSort(e.currentTarget.value)}>
        ${DEVICE_ORDERS.map(([v, label]) => html`<option value=${v}>${label}</option>`)}
      </select></label>` : html`<p class="empty">ES-DE has no games on this device yet.</p>`}
    ${sort === "system" ? html`<${BySystem} rows=${rows} name=${name} open=${systems.length === 1 ? systems[0] : openSys} setOpen=${setOpenSys} row=${row} removed=${removed} />`
      : html`<ul class="go-rows" id="device-list">${rows.map(row)}</ul>`}
    ${pick.length ? html`<div class="go-picked" id="device-picked" role="region" aria-label="Selected games">
      <span class="go-picked-what"><strong>${plural(pick.length, "game")}</strong> · frees ${bytes(frees)}</span>
      <span class="go-picked-do">
        <button type="button" class="ghost" onClick=${removed}>Clear</button>
        <button type="button" class="primary danger" id="device-remove" onClick=${() => showDialog({ type: "remove", asks: pick.map((r) => ({ id: r.id, key: r.key })), title: `Remove ${plural(pick.length, "game")}`, done: removed })}>Remove…</button>
      </span>
    </div>` : null}
  </section>`;
}

/** Make room…: how much, then the games played longest ago are selected (C20). */
export function RoomDialog({ run }) {
  const [gb, setGb] = useState("4");
  const need = Math.round(parseFloat(gb) * 1024 ** 3) || 0;
  return html`<${Dialog} title="Make room" id="room-dialog" onSubmit=${() => { closeDialog(); run(need); }} disabled=${need <= 0} submitLabel="Select games to remove">
    <label class="field-block"><span>Room needed, in GB</span>
      <input id="room-gb" type="number" min="0" step="any" value=${gb} onInput=${(e) => setGb(e.currentTarget.value)} />
      <small class="muted">Games played longest ago are selected first, then those never played, the oldest first. Nothing is removed until you choose Remove….</small>
    </label>
  <//>`;
}

/** What a removal takes (each file, its media and manual, its gamelist entry) and the room
 *  it frees, asked before anything goes (D101). `media`: only those media kinds. */
export function RemoveDialog({ asks, media = false, title, done }) {
  const [r, setR] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { call("removal", { asks }).then(setR).catch((e) => setError(message(e))); }, []);
  const remove = async () => {
    setBusy(true);
    setError("");
    try {
      const out = await call(media ? "remove_media" : "remove", { asks });
      closeDialog();
      done?.();
      const made = out.kept_made || [];
      say((media ? `Removed, freeing ${bytes(out.frees)}.` : `Removed ${plural(out.games.length, "game")}, freeing ${bytes(out.frees)}.`)
        + (made.length ? ` Files ${made.length === 1 ? "the game" : "the games"} made ${made.length === 1 ? "itself" : "themselves"}, such as saves, stay in ${made.map(([f]) => f).join(", ")}.` : ""));
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  };
  const going = r?.files || [];
  return html`<${Dialog} title=${title} id="remove-dialog" onSubmit=${remove} busy=${busy} disabled=${!r} error=${error} danger=${true}
      submitLabel=${r ? `Remove (frees ${bytes(r.frees)})` : "Remove"}>
    ${!r && !error ? html`<p class="muted">Working out what goes…</p>` : null}
    ${r ? html`
      <p id="remove-frees">${media ? "" : `${plural(r.games.length, "game")} ${r.games.length === 1 ? "goes" : "go"}. `}This frees ${bytes(r.frees)}.</p>
      <div class="field-block"><span>${plural(going.length, "file")}</span>
        <ul class="go-files" id="remove-files">${going.slice(0, 40).map((f) => html`<li><span class="go-file">${f.path}</span><span class="muted">${f.kind === "rom" ? "" : `${kindName(f.kind)} · `}${bytes(f.size)}</span></li>`)}
          ${going.length > 40 ? html`<li class="muted">and ${plural(going.length - 40, "more file")}</li>` : null}</ul>
      </div>
      ${r.kept.length ? html`<div class="field-block" id="remove-kept"><span>Staying</span>
        <p class="muted">Another game on this device needs ${r.kept.length === 1 ? "this" : "these"}, so ${r.kept.length === 1 ? "it stays" : "they stay"}:</p>
        <ul class="go-files">${r.kept.map((f) => html`<li><span class="go-file">${f.path}</span><span class="muted">${bytes(f.size)}</span></li>`)}</ul></div>` : null}
      ${r.gone?.length ? html`<p class="muted" id="remove-gone">${r.gone.length === 1 ? "1 file isn't" : `${count(r.gone.length)} files aren't`} on this device any more (was a folder changed in Settings?), so ${r.gone.length === 1 ? "it's" : "they're"} only taken off Game Repo Go's list.</p>` : null}
      ${r.entries.length ? html`<p class="muted">${plural(r.entries.length, "entry", "entries")} in ES-DE's game lists go too${r.lines.length ? `, and ${plural(r.lines.length, "line")} in its custom collections` : ""}.</p>` : null}
      ${going.some((f) => f.place === "pc") ? html`<p class="muted" id="remove-pc">A PC game's files from the library go${info.kind === "windows" ? ", with its Start menu shortcut" : ", with its Steam shortcut"}. Files it made itself, such as saves and settings, stay in its folder.</p>` : null}
      <p class="muted">Saves are never touched. Files are set aside first, so a removal cut short (the battery, say) can be finished or put back.</p>
    ` : null}
  <//>`;
}

/** A removal cut short, found when Game Repo Go opens: Finish removing or Put back. */
export function PendingRemoval({ pending }) {
  const [busy, setBusy] = useState(false);
  const r = pending.removal;
  const names = [...r.games, ...r.media_of];
  const run = async (cmd, said) => {
    setBusy(true);
    try {
      await call(cmd);
      say(said);
      go.set((s) => ({ deviceRev: s.deviceRev + 1 }));
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
    }
  };
  return html`<div class="go-note warn-note" id="go-pending" role="alert">
    <span>A removal was cut short${names.length ? ` (${names.slice(0, 3).join(", ")}${names.length > 3 ? ` and ${names.length - 3} more` : ""})` : ""}. Its files are set aside, not gone yet.</span>
    <span class="go-note-do">
      <button type="button" class="ghost" id="pending-finish" disabled=${busy} onClick=${() => run("finish_removal", "Finished removing.")}>Finish removing</button>
      ${pending.can_put_back ? html`<button type="button" class="ghost" id="pending-back" disabled=${busy} onClick=${() => run("put_back", "Put back as it was.")}>Put back</button>` : null}
    </span>
  </div>`;
}

/** By system (D115): each system a row with its number of games and their size, one open at
 *  a time, its games in it played longest ago first, and Remove all… for every one of them. */
function BySystem({ rows, name, open, setOpen, row, removed }) {
  const groups = new Map();
  for (const r of rows) {
    if (!groups.has(r.system)) groups.set(r.system, []);
    groups.get(r.system).push(r);
  }
  const order = [...groups.keys()].sort((a, b) => name(a).localeCompare(name(b), "en", { sensitivity: "base" }));
  return html`<ul class="go-systems" id="device-list">
    ${order.map((id) => {
      const games = groups.get(id);
      const size = games.reduce((a, r) => a + r.size, 0);
      const on = open === id;
      return html`<li class=${`go-sys${on ? " is-open" : ""}`} key=${id} data-system=${id}>
        <div class="go-sys-head">
          <button type="button" class="go-sys-open" aria-expanded=${on ? "true" : "false"} onClick=${() => setOpen(on ? null : id)}>
            ${Icon.chevron(16)}<span class="go-sys-what"><strong>${name(id)}</strong>
            <span class="muted">${plural(games.length, "game")} · ${bytes(size)}</span></span>
          </button>
          ${on ? html`<button type="button" class="ghost danger-text go-sys-remove" onClick=${() => showDialog({ type: "remove", asks: games.map((r) => ({ id: r.id, key: r.key })), title: `Remove every ${name(id)} game`, done: removed })}>Remove all ${count(games.length)}…</button>` : null}
        </div>
        ${on ? html`<ul class="go-rows">${games.map((r) => row(r, true))}</ul>` : null}
      </li>`;
    })}
  </ul>`;
}
