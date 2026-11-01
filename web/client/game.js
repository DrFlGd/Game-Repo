// A game's page (C7): its pictures, description, players and genre, its files and media
// with their sizes, and whether it's on the device already, with Download… and Add to queue
// (D114). Its
// details are fetched when it's opened and kept, so it shows away from home too. An installed
// PC game's files can be checked and the game put back (D207–D208).
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

const checkSay = (c) => (c.state === "as_installed" ? "As it was installed: nothing has changed." : c.state === "not_there" ? "Its folder isn't there." : `Changed: ${[c.changed.length ? `${c.changed.length} changed` : null, c.missing.length ? `${c.missing.length} gone` : null, c.added.length ? `${c.added.length} added` : null].filter(Boolean).join(", ")}.`);

/** An installed PC game's files checked against how they went in, and put back from the
 *  library (D207–D208): unticked files are left as they are, added ones put aside. */
function PcFiles({ k, title }) {
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
    setSaid(`Put back: ${r.fetched} fetched from the library, ${r.put_aside} put aside${r.kept ? `, ${r.kept} left as they are` : ""}.`);
    setLeave([]);
    setC(await call("pc_check", { key: k }));
  });
  const empty = () => run("Emptying…", async () => { await call("pc_empty_aside", { key: k }); setC(await call("pc_check", { key: k })); });
  const all = c ? [...c.changed, ...c.missing, ...c.added] : [];
  const flip = (p, on) => setLeave(on ? leave.filter((x) => x !== p) : [...leave, p]);
  const groups = [["changed", "Changed since it was installed", "fetched again"], ["missing", "Gone", "fetched again"], ["added", "Added", "put aside, not deleted"]];
  return html`<section class="go-pc-files" id="go-pc-files">
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
        ${c[g].map((p) => html`<label class="check-row" key=${p}><input type="checkbox" data-file=${p} checked=${!leave.includes(p)} disabled=${!!busy} onChange=${(e) => flip(p, e.target.checked)} /> <span class="go-file">${p}</span></label>`)}
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
          ${g.on_device && game.system === "pcgames" ? html`<${PcFiles} k=${game.key} title=${game.title} />` : null}
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
