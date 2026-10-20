// Add to queue… (D99, C9, C22): the media kinds the library has for the games selected, each
// with its size, ticked as last time on this device; the whole against the free space; and
// "Add 3 games (2.4 GB)", which puts them in the queue; PC games to the PC games folder
// (D111). Also a game's media added on its own, to a game already on the device (D102).
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call, info } from "./backend.js";
import { closeDialog, go, say } from "./state.js";
import { bytes, kindName, message, plural } from "./format.js";
import { Dialog } from "./dialog.js";

export function AddDialog({ keys }) {
  const where = useStore(go, (x) => x.where);
  const folder = useStore(go, (x) => x.status?.pc?.folder);
  const pc = keys.filter((k) => k.startsWith("pcgames/")).length;
  const [p, setP] = useState(null);
  const [ticked, setTicked] = useState(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    call("prepare", { keys })
      .then((r) => { setP(r); setTicked(new Set(r.kinds.filter((k) => k.ticked).map((k) => k.kind))); })
      .catch((e) => setError(message(e)));
  }, []);
  const media = p ? p.kinds.filter((k) => ticked.has(k.kind)).reduce((a, k) => a + k.size, 0) : 0;
  const total = p ? p.size + media : 0;
  const short = p && total > p.free;
  const n = keys.length;
  const flip = (k) => setTicked((t) => { const x = new Set(t); x.has(k) ? x.delete(k) : x.add(k); return x; });
  const add = async () => {
    setBusy(true);
    setError("");
    try {
      await call("add", { keys, media: [...ticked].filter((k) => k !== "manual"), manuals: ticked.has("manual"), collection: where.collection || "" });
      // the games added come off what's selected in the Library
      go.set((s) => { const picked = { ...s.picked }; for (const k of keys) delete picked[k]; return { picked }; });
      closeDialog();
      say(`${plural(n, "game")} added to the queue.`, "queue");
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${n === 1 ? "Add to the queue" : `Add ${plural(n, "game")} to the queue`} id="add-dialog" onSubmit=${add} busy=${busy} disabled=${!p}
      error=${error} submitLabel=${p ? `Add ${plural(n, "game")} (${bytes(total)})` : "Add"}>
    ${!p && !error ? html`<p class="muted">Asking the library what the games need…</p>` : null}
    ${p ? html`
      <div class="field-block" id="add-kinds">
        <span>With</span>
        ${p.kinds.length ? p.kinds.map((k) => html`<label class="check-row go-kind" key=${k.kind}>
            <input type="checkbox" name=${k.kind} checked=${ticked.has(k.kind)} onChange=${() => flip(k.kind)} />
            <span>${k.name || kindName(k.kind)}</span>
            <span class="muted go-kind-size">${k.estimated ? "about " : ""}${bytes(k.size)}${k.games < n ? ` · ${k.games} of ${n}` : ""}</span>
          </label>`)
          : html`<p class="muted">The library has no pictures or manuals for ${n === 1 ? "this game" : "these games"}.</p>`}
      </div>
      <p class="go-add-sum" id="add-sum">
        ${plural(n, "game")}: ${bytes(p.size)}${media ? ` with ${bytes(media)} of pictures and manuals` : ""}. <span class=${short ? "danger-text" : "muted"}>${bytes(p.free)} free on this device.</span>
      </p>
      ${short ? html`<p class="warn-note" id="add-short">There isn't room for all of that. Leave out some media, add fewer games, or make room on this device first.</p>` : null}
      ${p.left?.length ? html`<div class="warn-note"><p>Some can't go:</p><ul>${p.left.slice(0, 8).map((l) => html`<li>${l.what}: ${l.why}</li>`)}</ul></div>` : null}
      ${p.notes?.map((t) => html`<p class="muted">${t}</p>`)}
      ${p.unpacking ? html`<p class="muted" id="add-unpacking">${p.unpacking === 1 ? "A PC game is" : `${plural(p.unpacking, "PC game")} are`} being unpacked on the library first, so ${p.unpacking === 1 ? "it waits" : "they wait"} in the queue until that's done.</p>` : null}
      ${pc < n ? html`<p class="muted">Games go into ES-DE's folders one at a time. Each shows in ES-DE once all of its files are in, after ES-DE is restarted.</p>` : null}
      ${pc ? html`<p class="muted" id="add-pc">PC games go in ${folder || "the PC games folder"}, each in a folder of its own, with a shortcut ${info.kind === "windows" ? "in the Start menu" : "in Steam"}.</p>` : null}
    ` : null}
  <//>`;
}

/** A game on the device: media kinds the library has that it doesn't, added without the
 *  game being downloaded again (D102). `row` is On this device's. */
export function AddMediaDialog({ row }) {
  const [kinds, setKinds] = useState(null);
  const [ticked, setTicked] = useState(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    call("game", { key: row.key })
      .then((g) => {
        const have = new Set(row.media.map(([k]) => k));
        setKinds((g.details?.media || []).filter((m) => !have.has(m.kind)));
      })
      .catch((e) => setError(message(e)));
  }, []);
  const flip = (k) => setTicked((t) => { const x = new Set(t); x.has(k) ? x.delete(k) : x.add(k); return x; });
  const size = (kinds || []).filter((k) => ticked.has(k.kind)).reduce((a, k) => a + k.size, 0);
  const add = async () => {
    setBusy(true);
    try {
      await call("add_media", { id: row.id, key: row.key, kinds: [...ticked] });
      closeDialog();
      say(`${row.name}'s media added to the queue.`, "queue");
    } catch (e) {
      setError(message(e));
      setBusy(false);
    }
  };
  return html`<${Dialog} title=${`Add media to ${row.name}`} id="add-media-dialog" onSubmit=${add} busy=${busy} disabled=${!ticked.size} error=${error} submitLabel=${`Add (${bytes(size)})`}>
    ${kinds === null && !error ? html`<p class="muted">Asking the library what it has…</p>` : null}
    ${kinds && !kinds.length ? html`<p class="muted">This device already has every kind the library has for this game.</p>` : null}
    ${kinds?.length ? html`<div class="field-block">${kinds.map((k) => html`<label class="check-row" key=${k.kind}>
        <input type="checkbox" name=${k.kind} checked=${ticked.has(k.kind)} onChange=${() => flip(k.kind)} />
        <span>${kindName(k.kind)}</span><span class="muted">${bytes(k.size)}</span></label>`)}</div>` : null}
    ${row.from === "known" ? html`<p class="muted">This game came from elsewhere, but the library has it: its media are named for this device's file, and Game Repo Go looks after it from now on.</p>` : null}
  <//>`;
}
