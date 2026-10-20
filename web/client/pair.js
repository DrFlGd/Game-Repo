// Pairing with a library (D85): the libraries Game Repo Server announces on the network
// (DNS-SD), or one typed by its address, and the code Game Repo shows in Settings › Serve
// to devices › Pair a device. The first page until Game Repo Go is paired, and Settings'
// Pair with another library….
import { html, useState, useEffect } from "../lib/html.js";
import { call } from "./backend.js";
import { closeDialog, refreshLists, refreshSettings, refreshStatus, say } from "./state.js";
import { message } from "./format.js";
import { Dialog } from "./dialog.js";

function usePairing(done) {
  const [found, setFound] = useState(null);
  const [address, setAddress] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const find = () => {
    setFound(null);
    call("find", { wait: 3000 })
      .then((f) => { setFound(f); if (f.length === 1 && !address) setAddress(f[0].address); })
      .catch(() => setFound([]));
  };
  useEffect(find, []);
  const pair = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await call("pair", { address: address.trim(), code: code.trim() });
      await Promise.all([refreshStatus(), refreshSettings(), refreshLists()]);
      say(`Paired with ${r.library}. Reading its games onto this device…`);
      done?.();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  };
  return { found, find, address, setAddress, code, setCode, busy, error, pair };
}

function Fields({ p }) {
  return html`
    <div class="field-block">
      <span>Libraries on this network</span>
      ${p.found === null
        ? html`<p class="muted" id="pair-looking">Looking for Game Repo Server…</p>`
        : p.found.length
          ? html`<div class="go-found" role="radiogroup" aria-label="Libraries found">${p.found.map((f) => html`
              <button type="button" class="go-found-item" role="radio" aria-checked=${p.address === f.address} key=${f.address} onClick=${() => p.setAddress(f.address)}>
                <strong>${f.library}</strong><span class="muted">${f.address} · Game Repo ${f.version}</span>
              </button>`)}</div>`
          : html`<p class="muted" id="pair-none">None found. Is Game Repo serving the library, and is this device on the same network? You can type its address instead.</p>`}
      <div><button type="button" class="ghost" id="pair-look" onClick=${p.find}>Look again</button></div>
    </div>
    <label class="field-block"><span>Address</span>
      <input id="pair-address" placeholder="192.168.1.20:47210" autocomplete="off" spellcheck="false" value=${p.address} onInput=${(e) => p.setAddress(e.currentTarget.value)} />
    </label>
    <label class="field-block"><span>Code</span>
      <input id="pair-code" inputmode="numeric" maxlength="6" autocomplete="one-time-code" placeholder="6 digits" value=${p.code} onInput=${(e) => p.setCode(e.currentTarget.value.replace(/\D/g, ""))} />
      <small class="muted">In Game Repo on the computer with your games: Settings › Serve to devices › Pair a device. The code works for 10 minutes.</small>
    </label>`;
}

/** The first page, until Game Repo Go is paired. */
export function PairPage() {
  const p = usePairing();
  return html`<form class="go-page go-pair" id="pair-page" onSubmit=${(e) => { e.preventDefault(); p.pair(); }}>
    <h1>Pair with your library</h1>
    <p>Game Repo Go downloads games from a Game Repo library on your home network into ES-DE's folders on this device. Pair it with the library once; after that it keeps a copy of the list here, so you can browse it anywhere.</p>
    <${Fields} p=${p} />
    ${p.error ? html`<p class="form-error" role="alert" id="pair-error">${p.error}</p>` : null}
    <div><button type="submit" class="primary" id="pair-go" disabled=${p.busy || !p.address.trim() || p.code.length !== 6}>${p.busy ? "Pairing…" : "Pair"}</button></div>
  </form>`;
}

/** Settings › Pair with another library…: the games on the device stay. */
export function PairDialog() {
  const p = usePairing(closeDialog);
  return html`<${Dialog} title="Pair with another library" id="pair-dialog" onSubmit=${p.pair} busy=${p.busy} disabled=${!p.address.trim() || p.code.length !== 6} error=${p.error} submitLabel=${p.busy ? "Pairing…" : "Pair"}>
    <p class="muted">The games already on this device stay. This device's copy of the list is read again from the new library.</p>
    <${Fields} p=${p} />
  <//>`;
}
