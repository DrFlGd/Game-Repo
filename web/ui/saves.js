// Saves (docs/PLAN.md, "Save sync and the Devices page", D180–D191): a game's saves and
// states as Game Repo Go devices sent them, each with its versions, *Use this one* when two
// differ and *Put this version back* (each a new version, so nothing's lost); and Settings ›
// Saves, whether states are synced, where saves are kept and the save profiles built in.
import { html, useState, useEffect } from "../lib/html.js";
import { api, toast, isDesktop, size, plural } from "./library.js";
import { reveal } from "./games.js";

const err = (e) => e.message || String(e);
const when = (ms) => new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
const playedOn = (secs) => (secs ? new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "");

/** A version's line: from which device, when it was played, its size, and what it was. */
function versionFacts(v) {
  return [v.device_name || v.device, v.played ? `played ${playedOn(v.played)}` : null, size(v.files.reduce((n, f) => n + f.size, 0)), v.emulator || null,
    v.chosen ? "chosen again" : null, v.behind ? "kept beside the newest" : null].filter(Boolean).join(" · ");
}

/** One save or state: the newest, Two saves differ, and the versions folded under it. */
function Unit({ u, off, onChange }) {
  const [open, setOpen] = useState(false);
  const use = async (v, word) => {
    try {
      await api("saves_use", { unit: u.id, version: v.id });
      toast(`${word} ${u.label}: devices get it the next time they sync.`, 8000);
      await onChange();
    } catch (e) {
      toast(`Couldn't change it: ${err(e)}`, 8000);
    }
  };
  const others = u.versions.filter((v) => u.against.includes(v.id));
  return html`<li data-unit=${u.id} data-state=${u.state ? "yes" : "no"} data-conflict=${u.conflict ? "yes" : "no"}>
    <span class="op-paths"><span class="op-to">${u.label}</span>
      <span class="muted">${versionFacts(u.current)} · ${plural(u.versions.length, "version", "versions")}</span>
      ${u.conflict ? html`<span class="warn-note save-conflict" role="status">Two saves differ: the newest is ${u.current.device_name}'s, and ${others.map((o) => `${o.device_name}'s from ${playedOn(o.played)}`).join(", ")} is kept beside it. That device keeps its own until one is chosen.
        ${others.map((o) => html` <button type="button" class="ghost compact" data-action="use" key=${o.id} disabled=${off} onClick=${() => use(o, "Using")}>Use ${o.device_name}'s</button>`)}
        <button type="button" class="ghost compact" data-action="keep" disabled=${off} onClick=${() => use(u.current, "Keeping")}>Keep the newest</button></span>` : null}
      ${open ? html`<ol class="save-versions">${u.versions.map((v) => html`<li key=${v.id} data-version=${v.id} data-current=${v.id === u.current.id ? "yes" : "no"}>
        <span>${when(v.made)}</span> <span class="muted">${versionFacts(v)}</span>
        ${v.id === u.current.id ? html` <span class="bl-badge">Newest</span>` : html` <button type="button" class="linkish" data-action="put-back" disabled=${off} onClick=${() => use(v, "Put back")}>Put this version back</button>`}</li>`)}</ol>` : null}</span>
    <span class="seg"><button type="button" class="ghost compact" data-action="versions" aria-expanded=${open ? "true" : "false"} onClick=${() => setOpen(!open)}>${open ? "Hide versions" : "Versions"}</button>
      ${isDesktop() ? html`<button type="button" class="ghost compact" data-action="show" onClick=${() => reveal(u.folder)}>Show</button>` : null}</span>
  </li>`;
}

/** A game page's Saves (D191). */
export function GameSaves({ gameKey, off }) {
  const [r, setR] = useState(null);
  const read = () => api("saves_of", { key: gameKey }).then(setR).catch((e) => setR({ error: err(e), units: [] }));
  useEffect(() => { read(); }, [gameKey]);
  if (!r) return null;
  const saves = r.units.filter((u) => !u.state);
  const states = r.units.filter((u) => u.state);
  return html`<section class="ls-section" id="game-saves"><h2>Saves</h2>
    ${r.error ? html`<p class="warn-note" role="alert">${r.error}</p>` : null}
    ${r.units.length ? html`<ul class="ls-list" id="game-saves-list">${[...saves, ...states].map((u) => html`<${Unit} key=${u.id} u=${u} off=${off} onChange=${read} />`)}</ul>`
      : html`<p class="muted" id="game-saves-none">None yet. When it's played on a device with Game Repo Go, its saves and states come here, and go to your other devices.</p>`}
  </section>`;
}

/** Settings › Saves (D180, D182). */
export function SavesSettings({ lib }) {
  const [r, setR] = useState(null);
  useEffect(() => { api("saves_settings").then(setR).catch((e) => setR({ error: err(e) })); }, [lib.path]);
  const setStates = async (on) => {
    try {
      setR(await api("saves_set", { states: on }));
      toast(on ? "Save states are synced too." : "Only saves are synced; states stay on each device.");
    } catch (e) {
      toast(err(e), 8000);
    }
  };
  return html`<section class="ls-section" id="settings-saves">
    <h2>Saves</h2>
    <p class="muted">Game Repo Go sends each game's saves here and gives them to your other devices, keeping every version. A save state goes only to devices with the same emulator, since another can't load it. Saves are kept in the library's <code>Saves</code> folder${r?.folder ? html` (<code>${r.folder}</code>${r.size ? `, ${size(r.size)}` : ""})` : null}, so they're backed up with it.</p>
    ${r?.error ? html`<p class="warn-note" role="alert">${r.error}</p>` : null}
    ${r && !r.error ? html`<label class="check-row"><input type="checkbox" id="saves-states" checked=${r.states} disabled=${!!lib.read_only} onChange=${(e) => setStates(e.target.checked)} /> Sync save states too <span class="muted">(a state slot keeps its newest ten versions)</span></label>
      <h3>Emulators</h3>
      <ul class="ls-list" id="saves-profiles">${r.profiles.map((p) => html`<li key=${p.id} data-profile=${p.id}><span class="op-paths"><span class="op-to">${p.name}</span><span class="muted">${p.about}</span></span></li>`)}</ul>
      ${isDesktop() && r.folder ? html`<div class="home-actions"><button type="button" class="ghost compact" id="saves-show" onClick=${() => reveal(r.folder)}>Show the saves folder</button></div>` : null}` : null}
  </section>`;
}
