// On this device (D100–D102): every game ES-DE has on the device, with its size (its media
// and manual too), when it was last played and where it came from (the library, another
// computer the library recognises, or elsewhere), the free space at the top, ticks,
// Remove… and Make room…. A game's media can be removed or added on their own. Removing
// shows what goes and the room it frees first, and sets files aside before deleting them.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call } from "./backend.js";
import { closeDialog, go, open, say, showDialog } from "./state.js";
import { ago, bytes, count, kindName, message, played, plural } from "./format.js";
import { Dialog } from "./dialog.js";

const FROM = {
  library: ["From the library", ""],
  known: ["The library has it", "Came from elsewhere; the library has the same game"],
  elsewhere: ["From elsewhere", "The library doesn't have this game"],
  unchecked: ["Checking…", "Being checked against the library"],
};

/** Played longest ago first, then those never played by when they came (Make room's order). */
const ORDERS = {
  played: (a, b) => (!a.last_played - !b.last_played) || played(a.last_played) - played(b.last_played) || a.added - b.added || a.name.localeCompare(b.name),
  size: (a, b) => b.size - a.size || a.name.localeCompare(b.name),
  name: (a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }),
  system: (a, b) => a.system.localeCompare(b.system) || a.name.localeCompare(b.name, "en", { sensitivity: "base" }),
};

export function DevicePage() {
  const s = useStore(go, (x) => ({ rev: x.deviceRev, roomFor: x.roomFor, systems: x.systems, status: x.status }));
  const [d, setD] = useState(null);
  const [error, setError] = useState("");
  const [sort, setSort] = useState("played");
  const [ticked, setTicked] = useState(new Set());
  const [shown, setShown] = useState(null);
  const [room, setRoom] = useState("");
  const load = () => call("on_device").then((r) => { setD(r); setError(""); return r; }).catch((e) => { setError(message(e)); return null; });
  useEffect(() => { load(); }, [s.rev]);
  // Make room, from the Library's ticks: the games to offer, ticked here to change and confirm
  useEffect(() => {
    if (!s.roomFor || !d) return;
    const need = s.roomFor;
    go.set({ roomFor: 0 });
    makeRoom(need);
  }, [s.roomFor, !!d]);
  const makeRoom = (need) => call("make_room", { need }).then((ids) => {
    setTicked(new Set(ids));
    const frees = (d?.games || []).filter((g) => ids.includes(g.id)).reduce((a, g) => a + g.size, 0);
    setRoom(ids.length ? `Ticked ${plural(ids.length, "game")} played longest ago, freeing ${bytes(frees)} of the ${bytes(need)} needed. Change the ticks if you like, then choose Remove….`
      : "There's nothing on this device to remove.");
  }).catch((e) => say(message(e)));
  const name = (id) => s.systems.find((x) => x.id === id)?.name || id;
  if (!d) return html`<section class="go-page go-device">${error ? html`<p class="form-error" role="alert" id="device-error">${error}</p>` : html`<p class="muted">Looking through ES-DE's folders…</p>`}</section>`;
  const rows = [...d.games].sort(ORDERS[sort]);
  const pick = rows.filter((r) => ticked.has(r.id));
  const frees = pick.reduce((a, r) => a + r.size, 0);
  const flip = (id) => setTicked((t) => { const x = new Set(t); x.has(id) ? x.delete(id) : x.add(id); return x; });
  const removed = () => { setTicked(new Set()); setRoom(""); };
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
        <option value="played">Played longest ago</option>
        <option value="size">Largest</option>
        <option value="name">Name</option>
        <option value="system">System</option>
      </select></label>` : html`<p class="empty">ES-DE has no games on this device yet.</p>`}
    <ul class="go-rows" id="device-list">
      ${rows.map((r) => {
        const [from, why] = FROM[r.from] || [r.from, ""];
        const open_ = shown === r.id;
        return html`<li class=${`go-row${ticked.has(r.id) ? " is-ticked" : ""}`} key=${r.id} data-id=${r.id}>
          <input type="checkbox" class="go-row-tick" aria-label=${`Tick ${r.name}`} checked=${ticked.has(r.id)} onChange=${() => flip(r.id)} />
          <button type="button" class="go-row-open" aria-expanded=${open_ ? "true" : "false"} onClick=${() => setShown(open_ ? null : r.id)}>
            <strong>${r.name}</strong>
            <span class="muted">${name(r.system)} · ${bytes(r.size)} · ${r.last_played ? `played ${ago(played(r.last_played))}` : "never played"}</span>
          </button>
          <span class=${`badge go-from go-from-${r.from}`} title=${why}>${from}</span>
          ${open_ ? html`<div class="go-row-more">
            ${r.media.length ? html`<ul class="go-sizes">${r.media.map(([k, size]) => html`<li key=${k}><span>${kindName(k)}</span><span class="muted">${bytes(size)}</span>
                <button type="button" class="ghost" onClick=${() => showDialog({ type: "remove", asks: [{ id: r.id, key: r.key, kinds: [k] }], media: true, title: `Remove ${kindName(k).toLowerCase()} of ${r.name}` })}>Remove</button></li>`)}</ul>`
              : html`<p class="muted">No pictures or manual on this device.</p>`}
            <div class="go-row-do">
              ${r.key ? html`<button type="button" class="ghost" onClick=${() => showDialog({ type: "add-media", row: r })}>Add media…</button>
                <button type="button" class="ghost" onClick=${() => open({ page: "game", key: r.key })}>In the library</button>` : null}
              <button type="button" class="ghost danger-text" onClick=${() => showDialog({ type: "remove", asks: [{ id: r.id, key: r.key }], title: `Remove ${r.name}`, done: removed })}>Remove…</button>
            </div>
            <p class="muted go-row-path">${r.id}${r.files > 1 ? ` and ${plural(r.files - 1, "other file")}` : ""}</p>
          </div>` : null}
        </li>`;
      })}
    </ul>
    ${pick.length ? html`<div class="go-picked" id="device-picked" role="region" aria-label="Ticked games">
      <span class="go-picked-what"><strong>${plural(pick.length, "game")}</strong> · frees ${bytes(frees)}</span>
      <span class="go-picked-do">
        <button type="button" class="ghost" onClick=${removed}>Clear</button>
        <button type="button" class="primary danger" id="device-remove" onClick=${() => showDialog({ type: "remove", asks: pick.map((r) => ({ id: r.id, key: r.key })), title: `Remove ${plural(pick.length, "game")}`, done: removed })}>Remove…</button>
      </span>
    </div>` : null}
  </section>`;
}

/** Make room…: how much, then the games played longest ago are ticked (C20). */
export function RoomDialog({ run }) {
  const [gb, setGb] = useState("4");
  const need = Math.round(parseFloat(gb) * 1024 ** 3) || 0;
  return html`<${Dialog} title="Make room" id="room-dialog" onSubmit=${() => { closeDialog(); run(need); }} disabled=${need <= 0} submitLabel="Tick games to remove">
    <label class="field-block"><span>Room needed, in GB</span>
      <input id="room-gb" type="number" min="0" step="any" value=${gb} onInput=${(e) => setGb(e.currentTarget.value)} />
      <small class="muted">Games played longest ago are ticked first, then those never played, the oldest first. Nothing is removed until you choose Remove….</small>
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
      say(media ? `Removed, freeing ${bytes(out.frees)}.` : `Removed ${plural(out.games.length, "game")}, freeing ${bytes(out.frees)}.`);
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
      ${r.entries.length ? html`<p class="muted">${plural(r.entries.length, "entry", "entries")} in ES-DE's game lists go too${r.lines.length ? `, and ${plural(r.lines.length, "line")} in its custom collections` : ""}.</p>` : null}
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
