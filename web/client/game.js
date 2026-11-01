// A game's page (C7): its pictures, description, players and genre, its files and media
// with their sizes, and whether it's on the device already, with Download… and Add to queue
// (D114). Its
// details are fetched when it's opened and kept, so it shows away from home too. An installed
// PC game's files can be checked and the game put back (D207–D208), its mods picked
// and applied to it as it's installed (D209), and a mod put in by hand recorded as its,
// taken off by itself, or kept in the library (D210).
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call, pictureUrl } from "./backend.js";
import { back, go, open, showDialog } from "./state.js";
import { ago, bytes, kindName, message } from "./format.js";
import { pick } from "./library.js";
import { enqueue } from "./add.js";

function Picture({ k, kind, size, alt }) {
  const [bad, setBad] = useState(false);
  if (bad) return null;
  return html`<img class=${`go-game-pic go-pic-${kind}`} src=${pictureUrl(k, kind, size)} alt=${alt} onError=${() => setBad(true)} />`;
}

const list = (v) => (Array.isArray(v) ? v.join(", ") : v);

const files = (n) => `${n} ${n === 1 ? "file" : "files"}`;
const checkSay = (c) => (c.state === "as_installed" ? (c.hand?.length ? `As it was installed, with ${c.hand.join(", ")} put in by hand.` : c.mods?.length ? `As it was installed, with mods from Game Repo: ${c.mods.join(", ")}.` : "As it was installed: nothing has changed.") : c.state === "not_there" ? "Its folder isn't there." : `Changed: ${[c.changed.length ? `${c.changed.length} changed` : null, c.missing.length ? `${c.missing.length} gone` : null, c.added.length ? `${c.added.length} added` : null].filter(Boolean).join(", ")}.`);

/** An installed PC game's files checked against how they went in, and put back from the
 *  library (D207–D208): unticked files are left as they are, added ones put aside. */
/** Its mods from the library, ticked on this device (D209), applied to it as it's installed:
 *  a file changed by hand since is asked about first, replaced or kept. */
function PcMods({ k, mods, applied, busy, run, onDone }) {
  const goes = mods.filter((m) => m.goes_in);
  const now = applied ?? goes.filter((m) => m.on).map((m) => m.name);
  const [picked, setPicked] = useState(null);
  const [asks, setAsks] = useState(null);
  const [choice, setChoice] = useState({});
  useEffect(() => { setPicked(null); setAsks(null); }, [applied]);
  const want = picked ?? now;
  const same = want.length === now.length && want.every((x) => now.includes(x));
  const apply = () => run("Applying…", async () => {
    const replace = asks ? asks.filter((p) => choice[p] === "replace") : [];
    const keep = asks ? asks.filter((p) => choice[p] !== "replace") : [];
    const r = await call("pc_apply_mods", { key: k, mods: want, replace, keep });
    if (r.ask?.length) {
      setAsks(r.ask);
      setChoice({});
      return;
    }
    setAsks(null);
    onDone(`Its mods applied: ${files(r.fetched)} fetched, ${files(r.removed)} taken away.`);
  });
  const later = mods.filter((m) => m.to_choose);
  if (!goes.length && !later.length) return null;
  return html`<div class="go-pc-mods" id="go-pc-mods">
    <h2>Its mods</h2>
    <p class="muted" id="go-pc-mods-now">${applied ? (applied.length ? `In it now: ${applied.join(", ")}.` : "No mods in it now.") : "Tick the mods it should have, and apply them to it as it's installed."}</p>
    ${goes.map((m) => html`<label class="check-row" key=${m.name} data-mod=${m.name}><input type="checkbox" checked=${want.includes(m.name)} disabled=${!!busy}
      onChange=${(e) => setPicked(e.target.checked ? [...want, m.name] : want.filter((x) => x !== m.name))} /> ${m.title || m.name}${m.version ? ` v${m.version}` : ""}</label>`)}
    ${later.map((m) => html`<p class="muted" key=${m.name} data-mod-later=${m.name}>${m.title || m.name}: it has an installer, so choose its options in Game Repo first (its mods, Choose its options…).</p>`)}
    ${asks ? html`<fieldset class="go-pc-list" id="go-pc-asks"><legend>Changed since, by you or another program: replace it, or keep yours?</legend>
      ${asks.map((p) => html`<div class="go-pc-ask" key=${p} data-file=${p}><span class="go-file">${p}</span>
        <select aria-label=${`What to do with ${p}`} value=${choice[p] || "keep"} onChange=${(e) => setChoice({ ...choice, [p]: e.target.value })}><option value="keep">Keep mine</option><option value="replace">Replace it</option></select></div>`)}
    </fieldset>` : null}
    ${goes.length ? html`<div class="go-game-do"><button type="button" class="primary" id="go-pc-apply" disabled=${!!busy || (same && !asks)} onClick=${apply}>${busy === "Applying…" ? busy : asks ? "Apply, as chosen" : "Apply"}</button></div>` : null}
  </div>`;
}

/** The mods that go in by hand or by a file to run (D210): their readme and notes, *I've
 *  installed it* to record what changed as theirs, then *Take it off* and *Keep these files
 *  in the library*. */
function HandMods({ k, mods, hand, folder, busy, run, onDone }) {
  const names = [...mods.filter((m) => !m.goes_in && !m.to_choose).map((m) => m.name), ...Object.keys(hand).filter((n) => !mods.some((m) => m.name === n))];
  if (!names.length) return null;
  const record = (n, again) => run(`Recording ${n}…`, async () => {
    const fp = await call("pc_record_hand", { key: k, name: n });
    onDone(`${again ? "Recorded again" : "Recorded"}: ${files(fp.files.length)} changed or added${fp.gone.length ? `, ${files(fp.gone.length)} taken away` : ""}, as ${n}'s.`);
  });
  const takeOff = (n) => run(`Taking ${n} off…`, async () => {
    const r = await call("pc_put_back", { key: k, only: n });
    onDone(`${n} taken off: ${files(r.fetched)} fetched from the library, ${files(r.put_aside)} put aside.`);
  });
  const keep = (n) => run(`Sending ${n}…`, async () => {
    const r = await call("pc_keep_hand", { key: k, name: n });
    onDone(`Kept in the library as ${r.name}: it goes in by itself now, on this device and others (tick it above).`);
  });
  return html`<div class="go-pc-hand" id="go-pc-hand">
    <h2>Mods put in by hand</h2>
    <p class="muted">Put the mod in as its readme says${folder ? html` (the game's folder is <span class="go-file">${folder}</span>)` : null}, then say so: what changed is recorded as the mod's, so it can be taken off by itself.</p>
    ${names.map((n) => {
      const m = mods.find((x) => x.name === n) || { name: n };
      const when = hand[n];
      return html`<div class="go-pc-hand-mod" key=${n} data-hand=${n} data-recorded=${when ? "yes" : "no"}>
        <h3>${m.title || n}${m.version ? ` v${m.version}` : ""} <span class="muted">${when ? `recorded ${when.slice(0, 8).replace(/(\d{4})(\d{2})(\d{2})/, "$1-$2-$3")}` : m.how === "run" ? "a file to run" : "by hand"}</span></h3>
        ${m.notes ? html`<p class="go-pc-notes">${m.notes}</p>` : null}
        ${m.readme ? html`<details class="go-pc-readme"><summary>Its readme</summary><pre>${m.readme}</pre></details>` : null}
        <div class="go-game-do">
          ${when
            ? html`<button type="button" class="ghost" data-action="record" disabled=${!!busy} onClick=${() => record(n, true)}>I've changed it</button>
              <button type="button" class="ghost" data-action="take-off" disabled=${!!busy} onClick=${() => takeOff(n)}>Take it off</button>
              <button type="button" class="ghost" data-action="keep" disabled=${!!busy} onClick=${() => keep(n)}>Keep these files in the library</button>`
            : html`<button type="button" class="primary" data-action="record" disabled=${!!busy} onClick=${() => record(n, false)}>I've installed it</button>`}
        </div>
      </div>`;
    })}
  </div>`;
}

function PcFiles({ k, title, mods = [] }) {
  const [c, setC] = useState(null);
  const [leave, setLeave] = useState([]);
  const [busy, setBusy] = useState("");
  const [said, setSaid] = useState("");
  const [error, setError] = useState("");
  const run = async (what, f) => {
    setBusy(what);
    setError("");
    try {
      await f();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy("");
    }
  };
  const check = () => run("Checking…", async () => { setSaid(""); setLeave([]); setC(await call("pc_check", { key: k })); });
  const putBack = () => run("Putting it back…", async () => {
    const r = await call("pc_put_back", { key: k, leave });
    setSaid(`Put back: ${files(r.fetched)} fetched from the library, ${files(r.put_aside)} put aside${r.kept ? `, ${files(r.kept)} left as they are` : ""}.`);
    setLeave([]);
    setC(await call("pc_check", { key: k }));
  });
  const empty = () => run("Emptying…", async () => { await call("pc_empty_aside", { key: k }); setC(await call("pc_check", { key: k })); });
  // what mods it has, without reading its files (a big game's first check reads them all)
  const [applied, setApplied] = useState(null);
  const [hand, setHand] = useState({});
  const [rev, setRev] = useState(0);
  useEffect(() => { call("pc_mods_in", { key: k }).then((v) => { setApplied(v.mods ?? null); setHand(v.hand || {}); }).catch(() => {}); }, [k, c, rev]);
  const done = async (text) => { setSaid(text); setRev((r) => r + 1); if (c) setC(await call("pc_check", { key: k })); };
  const all = c ? [...c.changed, ...c.missing, ...c.added] : [];
  const flip = (p, on) => setLeave(on ? leave.filter((x) => x !== p) : [...leave, p]);
  const groups = [["changed", "Changed since it was installed", "fetched again"], ["missing", "Gone", "fetched again"], ["added", "Added", "put aside, not deleted"]];
  return html`<section class="go-pc-files" id="go-pc-files">
    <${PcMods} k=${k} mods=${mods} applied=${applied} busy=${busy} run=${run} onDone=${async (text) => { setSaid(text); setC(await call("pc_check", { key: k })); }} />
    <${HandMods} k=${k} mods=${mods} hand=${hand} folder=${c?.folder} busy=${busy} run=${run} onDone=${done} />
    <h2>Its files</h2>
    <p class="muted">Check ${title}'s folder against how it was installed: a mod put in by hand or a file changed or deleted shows here, and the game can be put back as the library has it.</p>
    <div class="go-game-do">
      <button type="button" class="ghost" id="go-pc-check" disabled=${!!busy} onClick=${check}>${busy === "Checking…" ? busy : "Check the game's files"}</button>
      ${c && all.length ? html`<button type="button" class="primary" id="go-pc-put-back" disabled=${!!busy || leave.length === all.length} onClick=${putBack}>${busy === "Putting it back…" ? busy : "Put it back as it was installed"}</button>` : null}
    </div>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${said ? html`<p id="go-pc-said">${said}</p>` : null}
    ${c ? html`<p id="go-pc-state" data-state=${c.state}>${checkSay(c)}</p>
      ${groups.filter(([g]) => c[g]?.length).map(([g, label, what]) => html`<fieldset class="go-pc-list" key=${g} data-list=${g}><legend>${label} (${c[g].length}): ${what}</legend>
        ${c[g].map((p) => html`<label class="check-row" key=${p}><input type="checkbox" data-file=${p} checked=${!leave.includes(p)} disabled=${!!busy} onChange=${(e) => flip(p, e.target.checked)} /> <span class="go-file">${p}</span>${c.owners?.[p] ? html` <span class="muted">(${c.owners[p]})</span>` : null}</label>`)}
      </fieldset>`)}
      ${all.length ? html`<p class="muted">Untick a file to leave it as it is: it's kept as the game's own from then on (a save made in the game's folder, say).</p>` : null}
      ${c.kept || c.played ? html`<p class="muted" data-kept=${c.kept || 0}>${[c.kept ? `${c.kept} kept as the game's own` : null, c.played ? `${c.played} the game made as it was played (logs, caches), not listed` : null].filter(Boolean).join("; ")}.</p>` : null}
      ${c.put_aside?.length ? html`<p class="muted" id="go-pc-aside">Put aside: ${c.put_aside.map((a) => `${a.files} files (${bytes(a.size)})`).join(", ")}, in its folder's .gamerepo-mods/put-aside. <button type="button" class="ghost compact" id="go-pc-empty" disabled=${!!busy} onClick=${empty}>Empty</button></p>` : null}` : null}
  </section>`;
}

export function GamePage({ gameKey }) {
  const s = useStore(go, (x) => ({ systems: x.systems, picked: x.picked, deviceRev: x.deviceRev, savesRev: x.savesRev }));
  const [g, setG] = useState(null);
  const [saves, setSaves] = useState([]);
  // its saves as last synced (D191)
  useEffect(() => { call("saves").then((v) => setSaves(v?.games?.[gameKey] || [])).catch(() => setSaves([])); }, [gameKey, s.savesRev]);
  const [error, setError] = useState("");
  useEffect(() => {
    setError("");
    call("game", { key: gameKey }).then(setG).catch((e) => setError(message(e)));
  }, [gameKey, s.deviceRev]);
  const game = g?.game;
  const d = g?.details?.details || {};
  const media = g?.details?.media || [];
  const files = g?.details?.files || [];
  const system = game && (s.systems.find((x) => x.id === game.system)?.name || game.system);
  const ticked = !!s.picked[gameKey];
  const facts = [["Players", d.players], ["Genres", list(d.genres)], ["Developer", d.developer], ["Publisher", d.publisher], ["Released", d.released]].filter(([, v]) => v);
  return html`<article class="go-page go-game" id="go-game">
    <div class="go-game-top">
      <button type="button" class="ghost" id="go-game-back" onClick=${back}>${Icon.chevronsLeft(14)} Back</button>
    </div>
    ${error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${!g && !error ? html`<p class="muted">Opening…</p>` : null}
    ${g && !game ? html`<p class="empty">The library doesn't have this game any more.</p>` : null}
    ${game ? html`
      <div class="go-game-body">
        <div class="go-game-pics">
          ${game.picture ? html`<${Picture} k=${game.key} kind=${game.picture} size=${512} alt=${`${game.title}'s box`} />` : html`<div class="go-cover go-game-nocover"><span class="go-cover-title">${game.title}</span></div>`}
          ${media.some((m) => m.kind === "screenshot") && game.picture !== "screenshot" ? html`<${Picture} k=${game.key} kind="screenshot" size=${512} alt="A screenshot" />` : null}
        </div>
        <div class="go-game-info">
          <h1 id="go-game-title">${game.title}</h1>
          <p class="muted">${[system, game.year, bytes(game.size), game.discs > 1 ? `${game.discs} discs` : null].filter(Boolean).join(" · ")}</p>
          <div class="go-game-do">
            ${g.on_device
              ? html`<span class="badge go-badge-on" id="go-game-on">On this device</span>
                  <button type="button" class="ghost" onClick=${() => open({ page: "device" })}>See what's on this device</button>`
              : html`<button type="button" class="primary" id="go-game-add" onClick=${() => showDialog({ type: "add", keys: [game.key] })}>Download…</button>
                  <button type="button" class="ghost" id="go-game-enqueue" onClick=${() => enqueue([game.key])}>Add to queue</button>
                  <button type="button" class="ghost" id="go-game-tick" aria-pressed=${ticked ? "true" : "false"} onClick=${() => pick([game])}>${ticked ? "Selected" : "Select"}</button>`}
          </div>
          ${g.on_device && game.system === "pcgames" ? html`<${PcFiles} k=${game.key} title=${game.title} mods=${g.details?.mods || []} />` : null}
          ${saves.length ? html`<ul class="go-sizes" id="go-game-saves">${saves.map((x) => html`<li data-conflict=${x.conflict ? "yes" : "no"}><span>${x.label}</span><span class="muted">${x.conflict ? "differs from the library's: choose one in Game Repo" : `synced ${ago(x.at)}`}</span></li>`)}</ul>` : null}
          ${d.description ? html`<div class="go-game-text">${String(d.description).split(/\n\s*\n/).map((p) => html`<p>${p}</p>`)}</div>` : null}
          ${facts.length ? html`<dl class="go-facts">${facts.map(([k, v]) => html`<dt>${k}</dt><dd>${v}</dd>`)}</dl>` : null}
          ${media.length ? html`<h2>Pictures and manuals</h2>
            <ul class="go-sizes">${media.map((m) => html`<li><span>${kindName(m.kind)}</span><span class="muted">${bytes(m.size)}</span></li>`)}</ul>` : null}
          ${files.length ? html`<h2>Files</h2>
            <ul class="go-sizes">${files.map((f) => html`<li><span class="go-file">${f.name}</span><span class="muted">${bytes(f.size)}</span></li>`)}</ul>` : null}
          ${!g.details ? html`<p class="muted">The rest of its details show once the library can be reached.</p>` : null}
        </div>
      </div>` : null}
  </article>`;
}
