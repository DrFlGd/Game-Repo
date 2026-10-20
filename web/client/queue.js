// The queue (D99): each game with its progress, Pause, Cancel and Pause all; waiting for
// the library while it can't be reached, going on by itself; and, once games are in,
// "Restart ES-DE to see the new games" (C17), as ES-DE reads its gamelists when it starts.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call } from "./backend.js";
import { go, say } from "./state.js";
import { bytes, message, plural } from "./format.js";

const STATE = {
  waiting: "Waiting its turn",
  downloading: "Downloading",
  paused: "Paused",
  offline: "Waiting for the library",
  done: "Done",
  failed: "Didn't finish",
};

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
  const going = q.items.filter((i) => i.state !== "done");
  const done = q.items.filter((i) => i.state === "done");
  const pausedAll = going.length && going.every((i) => i.state === "paused" || i.state === "failed");
  return html`<section class="go-page go-queue" id="go-queue">
    <div class="go-page-head">
      <h1>Queue</h1>
      ${going.length ? html`<button type="button" class="ghost" id="queue-all" onClick=${() => run(pausedAll ? "resume" : "pause")}>${pausedAll ? "Resume all" : "Pause all"}</button>` : null}
    </div>
    ${q.added && !going.length ? html`<div class="go-note" id="queue-restart" role="status">
        <span><strong>Restart ES-DE to see the new games.</strong> ES-DE reads its game lists when it starts.</span>
        <button type="button" class="ghost" onClick=${() => run("clear_done")}>OK</button>
      </div>` : null}
    ${q.offline ? html`<p class="warn-note" id="queue-offline" role="status">Waiting for the library: it can't be reached from here. The queue goes on by itself once it can.</p>` : null}
    ${going.length ? html`<p class="muted" id="queue-sum">${plural(q.left, "game")} to come · ${bytes(q.done)} of ${bytes(q.size)}${q.speed ? ` · ${bytes(q.speed)}/s` : ""}</p>` : null}
    ${!q.items.length ? html`<p class="empty" id="queue-empty">Nothing in the queue. Tick games in the Library and choose Add to device….</p>` : null}
    <ul class="go-queue-list" id="queue-list">
      ${going.map((i) => html`<li class=${`go-q go-q-${i.state}`} key=${i.id} data-key=${i.key}>
        <div class="go-q-what"><strong>${i.title}</strong><span class="muted">${i.media_only ? "Media only · " : ""}${STATE[i.state] || i.state}${i.state === "downloading" && i.speed ? ` · ${bytes(i.speed)}/s` : ""}</span></div>
        ${i.size ? html`<progress max=${i.size} value=${i.done} aria-label=${`${i.title}: ${bytes(i.done)} of ${bytes(i.size)}`}></progress>` : html`<progress aria-label=${i.title}></progress>`}
        <span class="muted go-q-size">${i.size ? `${bytes(i.done)} of ${bytes(i.size)}` : ""}</span>
        ${i.error ? html`<p class="form-error go-q-error">${i.error}</p>` : null}
        <span class="go-q-do">
          ${i.state === "paused" || i.state === "failed"
            ? html`<button type="button" class="ghost" onClick=${() => run("resume", { ids: [i.id] })}>${i.state === "failed" ? "Try again" : "Resume"}</button>`
            : html`<button type="button" class="ghost" onClick=${() => run("pause", { ids: [i.id] })}>Pause</button>`}
          <button type="button" class="ghost" onClick=${() => run("cancel", { ids: [i.id] })}>Cancel</button>
        </span>
      </li>`)}
    </ul>
    ${done.length ? html`<div class="go-page-head go-q-done-head"><h2>Done</h2>
        <button type="button" class="ghost" id="queue-clear" onClick=${() => run("clear_done")}>Clear</button></div>
      <ul class="go-queue-list" id="queue-done">${done.map((i) => html`<li class="go-q go-q-done" key=${i.id} data-key=${i.key}>
        <div class="go-q-what"><strong>${i.title}</strong><span class="muted">${i.media_only ? "Media added" : "On this device"} · ${bytes(i.size)}</span></div>
      </li>`)}</ul>` : null}
  </section>`;
}
