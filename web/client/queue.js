// Downloads (D99, D114): each game coming with its progress, Pause, Cancel and Pause all;
// waiting for the library while it can't be reached, going on by itself; the games queued,
// not started, with Remove from queue and Start downloads…, which shows their size on disc
// against the free space first; and, once games are in, "Restart ES-DE to see the new
// games" (C17), as ES-DE reads its gamelists when it starts (not for PC games, which ES-DE
// doesn't list); a PC game waits while the library unpacks it (D111), showing how far
// that's got (D126); on Linux, once a PC game's in while Steam's running, Restart Steam…,
// as its shortcut is written only while Steam's closed (D109, D125).
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call, info } from "./backend.js";
import { closeDialog, go, open, say, showDialog } from "./state.js";
import { bytes, message, plural } from "./format.js";
import { Dialog } from "./dialog.js";

const STATE = {
  waiting: "Waiting its turn",
  downloading: "Downloading",
  paused: "Paused",
  offline: "Waiting for the library",
  done: "Done",
  failed: "Didn't finish",
};

/** A PC game in on Linux, its Steam shortcut waiting while Steam runs (D125): Restart
 *  Steam… here, as in Settings › Steam, asked first since Steam closes with what it runs. */
function SteamWaiting({ rev }) {
  const [st, setSt] = useState(null);
  const [sure, setSure] = useState(false);
  const [busy, setBusy] = useState(false);
  const read = () => call("steam").then(setSt).catch(() => setSt(null));
  useEffect(() => { read(); }, [rev]);
  // while they wait, Steam may close (or be restarted) at any time
  useEffect(() => {
    if (!st?.waiting) return undefined;
    const t = setInterval(read, 3000);
    return () => clearInterval(t);
  }, [st?.waiting]);
  if (!st?.waiting || !st.running) return null;
  const restart = async () => {
    setBusy(true);
    try {
      await call("steam_restart");
      say("Restarting Steam: Game Repo Go's shortcuts are written while it's closed.");
      await read();
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
      setSure(false);
    }
  };
  return html`<div class="go-note" id="queue-steam" role="status">
    <span><strong>Restart Steam to see the new PC games in it.</strong>${" "}${sure
      ? `Steam closes, with any game it's running${st.game_mode ? " and Game Repo Go" : ""}, their shortcuts are written, and Steam starts again.`
      : "Their shortcuts are added while Steam is closed."}</span>
    ${sure ? html`<span class="go-set-do"><button type="button" class="primary" id="queue-steam-now" disabled=${busy} onClick=${restart}>Restart Steam now</button>
        <button type="button" class="ghost" id="queue-steam-not-now" onClick=${() => setSure(false)}>Not now</button></span>`
      : html`<button type="button" class="ghost" id="queue-steam-restart" onClick=${() => setSure(true)}>Restart Steam…</button>`}
  </div>`;
}

export function QueuePage() {
  const rev = useStore(go, (x) => x.queueRev);
  const [q, setQ] = useState(null);
  const [error, setError] = useState("");
  const load = () => call("queue").then((r) => { setQ(r); setError(""); }).catch((e) => setError(message(e)));
  useEffect(() => { load(); }, [rev]);
  // progress isn't an event: read it while something's on its way
  const moving = q?.items.some((i) => i.state === "downloading" || i.state === "waiting");
  useEffect(() => {
    if (!moving) return;
    const t = setInterval(load, 700);
    return () => clearInterval(t);
  }, [moving]);
  const run = (cmd, args = {}) => call(cmd, args).then(load).catch((e) => say(message(e)));
  if (!q) return html`<section class="go-page go-queue">${error ? html`<p class="form-error">${error}</p>` : html`<p class="muted">Opening the queue…</p>`}</section>`;
  const going = q.items.filter((i) => i.state !== "done" && i.state !== "queued");
  const queued = q.items.filter((i) => i.state === "queued");
  const done = q.items.filter((i) => i.state === "done");
  const pausedAll = going.length && going.every((i) => i.state === "paused" || i.state === "failed");
  return html`<section class="go-page go-queue" id="go-queue">
    <div class="go-page-head">
      <h1>Downloads</h1>
      ${going.length ? html`<button type="button" class="ghost" id="queue-all" onClick=${() => run(pausedAll ? "resume" : "pause")}>${pausedAll ? "Resume all" : "Pause all"}</button>` : null}
    </div>
    ${q.added && !going.length ? html`<div class="go-note" id="queue-restart" role="status">
        <span><strong>Restart ES-DE to see the new games.</strong> ES-DE reads its gamelists when it starts.</span>
        <button type="button" class="ghost" onClick=${() => run("clear_done")}>OK</button>
      </div>` : null}
    ${info.kind === "linux" && done.some((i) => i.key.startsWith("pcgames/")) ? html`<${SteamWaiting} rev=${`${rev}-${done.length}`} />` : null}
    ${q.offline ? html`<p class="warn-note" id="queue-offline" role="status">Waiting for the library: it can't be reached from here. The queue goes on by itself once it can.</p>` : null}
    ${going.length ? html`<p class="muted" id="queue-sum">${plural(q.left, "game")} to come · ${bytes(q.done)} of ${bytes(q.size)}${q.speed ? ` · ${bytes(q.speed)}/s` : ""}</p>` : null}
    ${!q.items.length ? html`<p class="empty" id="queue-empty">Nothing to download. Choose Download… on a game to download it now, or Add to queue to start it later with others.</p>` : null}
    <ul class="go-queue-list" id="queue-list">
      ${going.map((i) => html`<li class=${`go-q go-q-${i.state}`} key=${i.id} data-key=${i.key}>
        <div class="go-q-what"><strong>${i.title}</strong><span class="muted">${i.media_only ? "Pictures and manuals only · " : ""}${i.unpacking ? "Being unpacked in the library first" : STATE[i.state] || i.state}${i.state === "downloading" && i.speed ? ` · ${bytes(i.speed)}/s` : ""}</span></div>
        ${i.unpacking && i.unpack_size ? html`<progress class="go-q-unpack" max=${i.unpack_size} value=${i.unpacked || 0} aria-label=${`${i.title}: ${bytes(i.unpacked || 0)} of ${bytes(i.unpack_size)} unpacked`}></progress>`
          : i.size && !i.unpacking ? html`<progress max=${i.size} value=${i.done} aria-label=${`${i.title}: ${bytes(i.done)} of ${bytes(i.size)}`}></progress>` : html`<progress aria-label=${i.title}></progress>`}
        <span class="muted go-q-size">${i.unpacking && i.unpack_size ? `${bytes(i.unpacked || 0)} of ${bytes(i.unpack_size)} unpacked` : i.size && !i.unpacking ? `${bytes(i.done)} of ${bytes(i.size)}` : ""}</span>
        ${i.error ? html`<p class="form-error go-q-error">${i.error}</p>` : null}
        <span class="go-q-do">
          ${i.state === "paused" || i.state === "failed"
            ? html`<button type="button" class="ghost" onClick=${() => run("resume", { ids: [i.id] })}>${i.state === "failed" ? "Try again" : "Resume"}</button>`
            : html`<button type="button" class="ghost" onClick=${() => run("pause", { ids: [i.id] })}>Pause</button>`}
          <button type="button" class="ghost" onClick=${() => run("cancel", { ids: [i.id] })}>Cancel</button>
        </span>
      </li>`)}
    </ul>
    ${queued.length ? html`<div class="go-page-head go-q-done-head" id="queue-queued-head"><h2>Queued, not started</h2>
        <button type="button" class="primary" id="queue-start" onClick=${() => showDialog({ type: "start" })}>Start downloads…</button></div>
      <ul class="go-queue-list" id="queue-queued">${queued.map((i) => html`<li class="go-q go-q-queued" key=${i.id} data-key=${i.key}>
        <div class="go-q-what"><strong>${i.title}</strong><span class="muted">Waits for Start downloads</span></div>
        <span class="go-q-do"><button type="button" class="ghost" onClick=${() => run("cancel", { ids: [i.id] })}>Remove from queue</button></span>
      </li>`)}</ul>` : null}
    ${done.length ? html`<div class="go-page-head go-q-done-head"><h2>Done</h2>
        <button type="button" class="ghost" id="queue-clear" onClick=${() => run("clear_done")}>Clear</button></div>
      <ul class="go-queue-list" id="queue-done">${done.map((i) => html`<li class="go-q go-q-done" key=${i.id} data-key=${i.key}>
        <div class="go-q-what"><strong>${i.title}</strong><span class="muted">${i.media_only ? "Pictures and manuals added" : "On this device"} · ${bytes(i.size)}</span></div>
      </li>`)}</ul>` : null}
  </section>`;
}

/** Start downloads… (D114): each queued game's size on disc, the whole against the space
 *  free where they go and what's left after; without room, how much more is needed and
 *  Make room…, and nothing starts until there's room. */
export function StartDialog() {
  const [r, setR] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { call("start_summary").then(setR).catch((e) => setError(message(e))); }, []);
  const start = async () => {
    setBusy(true);
    try {
      const out = await call("start_queue", { ids: r.games.map((g) => g.id) });
      closeDialog();
      go.set((s) => ({ queueRev: s.queueRev + 1 }));
      say(`Downloading ${plural(out.started, "game")}.`);
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  };
  const n = r?.games.length || 0;
  const room = () => { closeDialog(); go.set({ roomFor: r.short + (64 << 20) }); open({ page: "device" }); };
  return html`<${Dialog} title="Start downloads" id="start-dialog" onSubmit=${start} busy=${busy} disabled=${!r || !n || r.short > 0} error=${error} cancelLabel="Not yet"
      submitLabel=${r ? `Start ${plural(n, "download")} (${bytes(r.size)})` : "Start"}>
    ${!r && !error ? html`<p class="muted">Asking the library what the games need…</p>` : null}
    ${r ? html`
      <ul class="go-files" id="start-games">${r.games.map((g) => html`<li key=${g.id}><span>${g.title}</span><span class="muted">${bytes(g.size)}</span></li>`)}</ul>
      <dl class="go-facts" id="start-sum">
        <dt>Size on disc</dt><dd>${bytes(r.size)}</dd>
        ${r.places.map((p) => html`<dt>${r.places.length > 1 ? (p.what === "pc" ? "Free for PC games" : "Free for the rest") : "Free on this device"}</dt><dd>${bytes(p.free)}</dd>
          <dt>Left after</dt><dd class=${p.size > p.free ? "danger-text" : ""}>${p.size > p.free ? `${bytes(p.size - p.free)} short` : bytes(p.free - p.size)}</dd>`)}
      </dl>
      <p class="muted">They go in ${r.places.map((p) => p.folder).join(" and ")}, with the pictures and manuals chosen when each was queued.</p>
      ${r.short > 0 ? html`<div class="warn-note" id="start-short"><span>There isn't room for all of them: ${bytes(r.short)} more is needed. Remove games from the queue, or make room on this device first.</span>
        <button type="button" class="ghost" id="start-room" onClick=${room}>Make room…</button></div>` : null}
    ` : null}
  <//>`;
}
