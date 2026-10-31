// Saves (docs/PLAN.md, "Save sync and the Devices page", D180–D191): a game's saves and
// states as Game Repo Go devices sent them, each with its versions, *Use this one* when two
// differ and *Put this version back* (each a new version, so nothing's lost); and Settings ›
// Saves, whether states are synced, where saves are kept, the save profiles built in, the
// memory cards every game of a system shares, and the saves not matched to a game (D195),
// each given one by hand or thrown away.
import { html, useState, useEffect } from "../lib/html.js";
import { ui } from "./state.js";
import { api, toast, isDesktop, size, plural } from "./library.js";
import { reveal } from "./games.js";
import { Dialog, close, confirmDialog } from "./dialogs.js";

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

/** A memory card every game of a system shares (D192), kept as one save, in Settings › Saves. */
function CardSaves({ card, off }) {
  const [r, setR] = useState(null);
  const read = () => api("saves_of", { key: card.key }).then(setR).catch((e) => setR({ error: err(e), units: [] }));
  useEffect(() => { read(); }, [card.key]);
  if (!r) return null;
  return html`<li data-cards=${card.key}><span class="op-paths"><span class="op-to">${card.system}</span>
    ${r.error ? html`<span class="warn-note" role="alert">${r.error}</span>` : null}
    <ul class="ls-list">${r.units.map((u) => html`<${Unit} key=${u.id} u=${u} off=${off} onChange=${read} />`)}</ul></span></li>`;
}

/** *Give it a game…* for a save not matched (D195): one game of the library chosen. */
function GiveDialog({ save, onDone }) {
  // (its name as far as its tags: "Alpha" from "Alpha (E) [!]")
  const [q, setQ] = useState(() => save.save.game.replace(/\s*[([].*$/, "").trim() || save.save.game);
  const [found, setFound] = useState(null);
  const [picked, setPicked] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (q.trim().length < 2) { setFound(null); return; }
    let gone = false;
    const t = setTimeout(() => api("browse").then(() => api("browse_query", { query: { search: q, one_per_title: false, limit: 40 } }))
      .then((r) => { if (!gone) setFound((r.items || []).filter((g) => !g.key.startsWith("pc/"))); }).catch((e) => { if (!gone) setFound({ error: err(e) }); }), 200);
    return () => { gone = true; clearTimeout(t); };
  }, [q]);
  const submit = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await api("saves_loose_give", { sha1: save.save.file.sha1, key: picked });
      close();
      toast(`${save.save.file.name} is ${(Array.isArray(found) ? found.find((g) => g.key === picked)?.title : null) || picked}'s save now: it goes to your devices with the game.`, 8000);
      onDone?.(r);
    } catch (e) {
      setError(err(e));
      setBusy(false);
    }
  };
  const list = Array.isArray(found) ? found : [];
  return html`<${Dialog} title=${`Give ${save.save.file.name} a game`} id="saves-give-dialog" wide=${true} onSubmit=${submit} busy=${busy || !picked} error=${error} submitLabel="Give it this game">
    <p class="muted">It becomes the game's newest save (one the library has already is kept as a version), and goes to each device with the game at its next sync.</p>
    <div class="pick-row"><input type="search" id="saves-give-search" class="compact" placeholder="Search the Library by title" aria-label="Search the Library by title" value=${q} onInput=${(e) => setQ(e.target.value)} /></div>
    ${found?.error ? html`<p class="warn-note" role="alert">${found.error}</p>` : null}
    <ul class="ls-list organise-list" id="saves-give-list">${list.map((g) => html`<li key=${g.key} data-key=${g.key}>
      <label class="check-row"><input type="radio" name="saves-give" checked=${picked === g.key} onChange=${() => setPicked(g.key)} /> ${g.title}</label>
      <span class="muted">${[g.platform_name, g.year].filter(Boolean).join(" · ")}</span></li>`)}</ul>
    ${Array.isArray(found) && !found.length ? html`<p class="muted">No game matches.</p>` : null}
  <//>`;
}

export function SavesDialogs({ dialog }) {
  if (dialog.type === "saves-give") return html`<${GiveDialog} ...${dialog} />`;
  return null;
}

/** The saves not matched to a game (D195), in Settings › Saves. */
function LooseSaves({ loose, off, onChange }) {
  const drop = (s) => confirmDialog({
    title: `Throw away ${s.save.file.name}?`, button: "Throw it away",
    text: `It's deleted from the library. ${s.device_name || "The device"} keeps its own, and it's sent here again if its file changes.`,
    run: async () => onChange(await api("saves_loose_drop", { sha1: s.save.file.sha1 })),
  });
  return html`<h3>Saves not matched</h3>
    <p class="muted">Saves a device has that none of its games is named like (a game from elsewhere, or one the library has under another name). Give one a game and it's that game's save from then on.</p>
    <ul class="ls-list" id="saves-loose">${loose.map((s) => html`<li key=${s.save.file.sha1} data-loose=${s.save.file.sha1}>
      <span class="op-paths"><span class="op-to">${s.save.file.name}</span>
        <span class="muted">${[s.save.emulator, s.device_name ? `from ${s.device_name}` : null, s.save.file.modified ? `played ${playedOn(s.save.file.modified)}` : null, size(s.save.file.size)].filter(Boolean).join(" · ")}</span></span>
      <button type="button" class="ghost compact" data-action="give" disabled=${off} onClick=${() => ui.set({ dialog: { type: "saves-give", save: s, onDone: onChange } })}>Give it a game…</button>
      <button type="button" class="linkish" data-action="drop" disabled=${off} onClick=${() => drop(s)}>Throw away</button></li>`)}</ul>`;
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
      ${r.cards?.length ? html`<h3>Memory cards</h3>
        <p class="muted">Some emulators keep every game's saves on one memory card (PCSX2's, Dolphin's, DuckStation's shared card, Flycast's VMUs): each is kept here as one save, with its versions.</p>
        <ul class="ls-list" id="saves-cards">${r.cards.map((c) => html`<${CardSaves} key=${c.key} card=${c} off=${!!lib.read_only} />`)}</ul>` : null}
      ${r.loose?.length ? html`<${LooseSaves} loose=${r.loose} off=${!!lib.read_only} onChange=${setR} />` : null}
      ${isDesktop() && r.folder ? html`<div class="home-actions"><button type="button" class="ghost compact" id="saves-show" onClick=${() => reveal(r.folder)}>Show the saves folder</button></div>` : null}` : null}
  </section>`;
}
