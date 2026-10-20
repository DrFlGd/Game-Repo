// A game's page (C7): its pictures, description, players and genre, its files and media
// with their sizes, and whether it's on the device already, with Add to queue…. Its
// details are fetched when it's opened and kept, so it shows away from home too.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { Icon } from "../ui/icons.js";
import { call, pictureUrl } from "./backend.js";
import { back, go, open, showDialog } from "./state.js";
import { bytes, kindName, message } from "./format.js";
import { pick } from "./library.js";

function Picture({ k, kind, size, alt }) {
  const [bad, setBad] = useState(false);
  if (bad) return null;
  return html`<img class=${`go-game-pic go-pic-${kind}`} src=${pictureUrl(k, kind, size)} alt=${alt} onError=${() => setBad(true)} />`;
}

const list = (v) => (Array.isArray(v) ? v.join(", ") : v);

export function GamePage({ gameKey }) {
  const s = useStore(go, (x) => ({ systems: x.systems, picked: x.picked, deviceRev: x.deviceRev }));
  const [g, setG] = useState(null);
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
              : html`<button type="button" class="primary" id="go-game-add" onClick=${() => showDialog({ type: "add", keys: [game.key] })}>Add to queue…</button>
                  <button type="button" class="ghost" id="go-game-tick" aria-pressed=${ticked ? "true" : "false"} onClick=${() => pick([game])}>${ticked ? "Selected" : "Select"}</button>`}
          </div>
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
