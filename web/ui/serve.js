// Serve to devices (docs/PLAN.md, "Phase 9a design", D89): Game Repo Server's state, the
// pairing code, the paired devices and the server's settings. One panel for Game Repo's
// Settings and for the server's own window (server.html), each passing how to reach the
// server: `call(cmd, args)` runs one of the serve_* commands (desktop/core/src/api.rs in
// Game Repo, desktop/server/src/main.rs in the server's window). It imports nothing of the
// app's, so the server's window loads only this.
import { html, useState, useEffect } from "../lib/html.js";

const message = (e) => e?.message || String(e);

function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n < 10 && i ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/** When a device was last seen, as people say it. */
function seen(secs) {
  if (!secs) return "not seen yet";
  const ago = Date.now() / 1000 - secs;
  if (ago < 120) return "seen just now";
  if (ago < 3600) return `seen ${Math.round(ago / 60)} minutes ago`;
  if (ago < 86400) return `seen ${plural(Math.round(ago / 3600), "hour", "hours")} ago`;
  return `seen ${new Date(secs * 1000).toLocaleDateString()}`;
}

/** The code's minutes and seconds left. */
const left = (secs) => `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;

/** `alone`: in the server's own window, where Stop ends the server and another library can
 *  be opened; in Game Repo, a switch starts and stops it with the open library. */
export function ServePanel({ call, alone = false }) {
  const [s, setS] = useState(null);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [said, setSaid] = useState("");
  const [networks, setNetworks] = useState([]);
  const [removing, setRemoving] = useState(null);
  const [port, setPort] = useState("");
  const [, tick] = useState(0);

  const load = () => call("serve_status").then(setS).catch((e) => setError(message(e)));
  const st = s?.status || null;
  const code = s?.running && st?.pairing ? st.pairing : null;
  useEffect(() => {
    load();
    // the code counts down each second; otherwise every two
    const t = setInterval(() => { load(); tick((n) => n + 1); }, code ? 1000 : 2000);
    return () => clearInterval(t);
  }, [!!code]);
  useEffect(() => { call("serve_networks").then(setNetworks).catch(() => {}); }, []);
  useEffect(() => { if (s && port === "") setPort(String(s.settings.port)); }, [s]);
  // a device that paired: the code goes, and the device is in the list
  const [pairing, setPairing] = useState(null);
  useEffect(() => {
    if (!s) return;
    if (code) setPairing({ devices: (s.devices || []).length });
    else if (pairing) {
      const now = s.devices || [];
      if (now.length > pairing.devices) setSaid(`Paired with ${now[now.length - 1].name}.`);
      setPairing(null);
    }
  }, [s]);

  const run = async (what, cmd, args = {}) => {
    setBusy(what);
    setError("");
    setSaid("");
    try {
      const r = await call(cmd, args);
      if (r && "running" in r) setS(r);
      else await load();
      return r;
    } catch (e) {
      setError(message(e));
      await load();
    } finally {
      setBusy("");
    }
  };

  if (!s) {
    return html`<section class="ls-section" id="settings-serve"><h2>Serve to devices</h2>
      ${error ? html`<p class="warn-note" role="alert">${error}</p>` : html`<p class="muted">Asking Game Repo Server…</p>`}</section>`;
  }
  const devices = s.devices || [];
  const publicNets = networks.filter((n) => n.public);
  const another = s.running && !s.this_library && !alone;
  const portNumber = Number(port);
  const portOk = Number.isInteger(portNumber) && portNumber >= 1024 && portNumber <= 65535;
  return html`<section class="ls-section serve-panel" id="settings-serve" data-running=${s.running ? "yes" : "no"}>
    <h2>${alone ? "Game Repo Server" : "Serve to devices"}</h2>
    <p class="muted">Phones and handhelds on your home network browse the library and download games into ES-DE's folders, with Game Repo's app on the device. The library is only read: nothing in it changes.${alone ? "" : " Game Repo Server does the serving, so it can keep going when Game Repo is closed."}</p>
    ${!s.installed ? html`<p class="warn-note" role="alert" id="serve-missing">Game Repo Server isn't installed beside Game Repo: install Game Repo again.</p>` : null}

    ${alone ? null : html`<label class="check-row"><input type="checkbox" id="serve-on" checked=${busy === "Starting" || (s.running && busy !== "Stopping")} disabled=${!!busy || !s.installed}
      onChange=${(e) => run(e.target.checked ? "Starting" : "Stopping", e.target.checked ? "serve_start" : "serve_stop")} /> Serve ${another ? "a library" : "this library"} to devices</label>`}
    <div class="serve-state" id="serve-state" aria-live="polite">
      ${busy ? html`<span class="muted">${busy}…</span>`
        : s.running && st ? html`<span>Serving <b>${st.library_name}</b> at <code id="serve-address">${st.address || `127.0.0.1:${st.port}`}</code>.
          ${st.error ? null : st.ready ? " Paired devices can browse it." : html` <span class="muted">${st.doing || "Getting ready"}…</span>`}</span>
          ${st.address ? null : html`<span class="muted">This computer has no address on a network, so devices can't reach it.</span>`}`
        : html`<span class="muted">Not serving.</span>`}
    </div>
    ${s.running && st?.error ? html`<p class="warn-note" role="alert">${st.error}</p>` : null}
    ${another ? html`<div class="pick-row" id="serve-another"><span>It's serving another library: <code>${st?.library}</code>.</span>
      <button type="button" class="ghost" id="serve-restart" disabled=${!!busy} onClick=${() => run("Starting", "serve_restart")}>Serve this one instead</button></div>` : null}
    ${s.start_error && !s.running ? html`<p class="warn-note" role="alert" id="serve-start-error">${s.start_error}</p>` : null}
    ${error ? html`<p class="form-error" role="alert" id="serve-error">${error}</p>` : null}
    ${said ? html`<p id="serve-said">${said}</p>` : null}
    ${s.windows && publicNets.length ? html`<p class="warn-note" id="serve-public">Windows treats ${publicNets.map((n) => n.name).join(" and ")} as a public network, where it keeps devices from reaching the server. If it's your home network, make it private: Windows Settings › Network & internet › ${publicNets[0].interface || "your connection"} › Network profile type › Private network.</p>` : null}

    ${s.running ? html`<div class="field-block"><span class="field-label">Pair a device</span>
      ${code ? html`<div class="serve-code-row"><span class="serve-code" id="serve-code" aria-label=${`Pairing code ${code[0].split("").join(" ")}`}>${code[0].slice(0, 3)} ${code[0].slice(3)}</span>
          <span class="muted" id="serve-code-left">for ${left(code[1])}</span>
          <button type="button" class="ghost" id="serve-unpair" disabled=${!!busy} onClick=${() => run("Ending the pairing", "serve_unpair")}>Cancel</button></div>
        <small class="muted">In Game Repo's app on the device, choose this computer (or type its address) and enter the code.</small>`
      : html`<div><button type="button" class="ghost" id="serve-pair" disabled=${!!busy} onClick=${() => run("Making a code", "serve_pair")}>Pair a device…</button></div>
        <small class="muted">Shows a code for ten minutes that a device enters once; after that it connects by itself.</small>`}
    </div>` : null}

    ${s.running || devices.length ? html`<div class="field-block"><span class="field-label">Devices</span>
      ${devices.length ? html`<ul class="ls-list" id="serve-devices">${devices.map((d) => html`<li key=${d.id} data-device=${d.id}>
        <span><b>${d.name}</b> <span class="muted">${[d.kind, d.holding?.games ? `${plural(d.holding.games, "game", "games")} (${bytes(d.holding.size)})` : "no games from here yet", seen(d.seen)].filter(Boolean).join(" · ")}</span></span>
        ${removing === d.id ? html`<span class="serve-confirm">Remove it? It'll need a new code to connect again.</span>
            <button type="button" class="primary danger" data-action="remove-yes" disabled=${!!busy} onClick=${async () => { await run("Removing", "serve_remove_device", { id: d.id }); setRemoving(null); setSaid(`Removed ${d.name}.`); }}>Remove</button>
            <button type="button" class="ghost" data-action="remove-no" onClick=${() => setRemoving(null)}>Keep</button>`
          : html`<button type="button" class="ghost" data-action="remove" disabled=${!!busy || !s.running} onClick=${() => setRemoving(d.id)}>Remove…</button>`}
      </li>`)}</ul>` : html`<p class="muted" id="serve-devices-none">No devices yet.</p>`}
    </div>` : null}

    <details class="serve-more" open=${alone}><summary>Port${alone ? "" : ", keeping it serving"}${s.windows ? " and starting with Windows" : ""}</summary>
      <form class="field-block" onSubmit=${(e) => { e.preventDefault(); if (portOk) run("Changing the port", "serve_set", { port: portNumber }); }}>
        <span class="field-label">Port</span>
        <div class="pick-row"><input type="number" id="serve-port" class="compact" min="1024" max="65535" value=${port} onInput=${(e) => setPort(e.target.value)} aria-label="Port" />
          <button type="submit" class="ghost" id="serve-port-set" disabled=${!!busy || !portOk || portNumber === s.settings.port}>Change</button></div>
        <small class="muted">Devices find the server on the network by themselves. Change it when another program on this computer uses ${s.settings.port}.${s.running ? " The server starts again on the new one." : ""}</small>
      </form>
      ${alone ? null : html`<div class="field-block"><label class="check-row"><input type="checkbox" id="serve-keep" checked=${s.settings.keep} disabled=${!!busy}
          onChange=${(e) => run("Saving", "serve_set", { keep: e.target.checked })} /> Keep serving when Game Repo is closed</label>
        <small class="muted">The server stays, with its icon by the clock, where you can stop it. Otherwise it stops with Game Repo.</small></div>`}
      ${s.windows ? html`<div class="field-block"><label class="check-row"><input type="checkbox" id="serve-with-windows" checked=${s.with_windows} disabled=${!!busy || !s.can_start_with_windows}
          onChange=${(e) => run("Saving", "serve_set", { with_windows: e.target.checked })} /> Start with Windows</label>
        <small class="muted">It serves the library it served last, with its icon by the clock.</small></div>` : null}
    </details>

    ${alone ? html`<div class="home-actions">
      ${!s.running && s.last_library ? html`<button type="button" class="ghost" id="serve-start" disabled=${!!busy} title=${s.last_library} onClick=${() => run("Starting", "serve_start")}>Serve ${s.last_library_name || "the library"} again</button>` : null}
      <button type="button" class="ghost" id="serve-open" disabled=${!!busy} onClick=${() => run("Opening the library", "serve_open_library")}>Open another library…</button>
      <button type="button" class="ghost" id="serve-stop" disabled=${!!busy} title="Game Repo Server ends, with its icon by the clock" onClick=${() => run("Stopping", "serve_stop")}>${s.running ? "Stop serving" : "Quit"}</button></div>` : null}
  </section>`;
}
