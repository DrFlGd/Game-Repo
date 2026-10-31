// Settings (D103): the library paired with, ES-DE's folders found (each can be changed, C3),
// the media ticked by default, downloads at once, the ES-DE custom collection games go in
// (C16), what the Library shows, the device's name and screen, the picture cache, the look
// (the view, D113, the big screen's size, D132, and Game Repo's skins and themes) and the
// version with its updates (D134); on Windows and Linux, the PC games folder (D111) and full screen (D127); on
// Linux, Steam (D107, D109); Tailscale, set up on the Steam Deck (D179); and saves synced
// with the library (D186). Each change is kept as it's made.
import { html, useEffect, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { SKINS, THEMES } from "../ui/state.js";
import { call, info, pickFolder } from "./backend.js";
import { go, look, refreshLists, refreshSettings, refreshStatus, say, setLook, showDialog, usualView } from "./state.js";
import { ago, bytes, count, KINDS, message } from "./format.js";
import { UpdatePanel } from "../ui/updates.js";

function Folder({ id, label, value, fallback, setting, error }) {
  const [text, setText] = useState(null);
  const [busy, setBusy] = useState(false);
  const save = async (v) => {
    setBusy(true);
    try {
      await call("set_settings", { [setting]: v });
      setText(null);
      await refreshStatus();
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
    }
  };
  const choose = async () => {
    const p = await pickFolder(`Choose ${label}`).catch(() => null);
    if (p) save(p);
  };
  return html`<div class="field-block go-folder" id=${id}>
    <span>${label}</span>
    <div class="go-folder-row">
      <input value=${text ?? (value || "")} placeholder=${fallback || "Not found"} spellcheck="false" aria-label=${label}
        onInput=${(e) => setText(e.currentTarget.value)} onKeyDown=${(e) => { if (e.key === "Enter" && text != null) { e.preventDefault(); save(text.trim()); } }} />
      ${text != null ? html`<button type="button" class="ghost" disabled=${busy} onClick=${() => save(text.trim())}>Save</button>` : null}
      <button type="button" class="ghost" disabled=${busy} onClick=${choose}>Choose…</button>
    </div>
    ${error ? html`<small class="danger-text">${error}</small>` : null}
  </div>`;
}

/** Steam on Linux (D107, D109): Game Repo Go in Steam's library, so Game Mode can start
 *  it, and Steam's files written only while Steam's closed, with Restart Steam to do it. */
function SteamSettings({ rev }) {
  const [st, setSt] = useState(null);
  const [sure, setSure] = useState(false);
  const [busy, setBusy] = useState(false);
  const read = () => call("steam").then(setSt).catch(() => setSt({ found: false }));
  useEffect(() => { read(); }, [rev]);
  // while something waits, Steam may close (or come back) at any time
  useEffect(() => {
    if (!st?.waiting) return undefined;
    const t = setInterval(read, 3000);
    return () => clearInterval(t);
  }, [st?.waiting]);
  const run = async (cmd, args = {}) => {
    setBusy(true);
    try {
      const r = await call(cmd, args);
      if (cmd === "steam_restart") say("Restarting Steam: Game Repo Go's shortcuts are written while it's closed.");
      else if (r && "found" in r) setSt(r);
      await read();
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
      setSure(false);
    }
  };
  if (!st) return html`<section class="go-set" id="set-steam"><h2>Steam</h2><p class="muted">Looking for Steam…</p></section>`;
  return html`<section class="go-set" id="set-steam">
    <h2>Steam</h2>
    ${!st.found ? html`<p class="muted" id="set-steam-none">Steam isn't on this device, or isn't where it usually is.</p>
      <${Folder} id="set-steam-folder" label="Steam's folder" value="" setting="steam" fallback="The folder with Steam's userdata folder in it" />` : html`
      ${st.users?.length > 1 ? html`<label class="field-block"><span>Shortcuts for</span>
        <select id="set-steam-user" class="compact" value=${st.user} onChange=${(e) => call("set_settings", { steam_user: e.currentTarget.value }).then(read).catch((e2) => say(message(e2)))}>
          ${st.users.map((u) => html`<option value=${u.id}>${u.name || u.account || u.id}</option>`)}</select></label>` : null}
      <label class="check-row"><input type="checkbox" id="set-steam-self" checked=${st.own} disabled=${busy} onChange=${() => run("steam_add_self", { on: !st.own })} />
        <span>Game Repo Go in Steam's library, so Game Mode can start it</span></label>
      ${st.waiting ? html`<div class="warn-note" id="set-steam-waiting">
          <p>${st.running ? "Steam is running, so Game Repo Go's shortcuts wait until it's closed." : "Game Repo Go's shortcuts aren't in Steam yet."}</p>
          ${st.running && !sure ? html`<button type="button" class="ghost" id="set-steam-restart" disabled=${busy} onClick=${() => setSure(true)}>Restart Steam…</button>` : null}
          ${st.running && sure ? html`<p id="set-steam-sure">Steam closes, with any game it's running${st.game_mode ? " and Game Repo Go" : ""}, the shortcuts are written, and Steam starts again.</p>
            <div class="go-set-do"><button type="button" id="set-steam-restart-now" disabled=${busy} onClick=${() => run("steam_restart")}>Restart Steam now</button>
            <button type="button" class="ghost" onClick=${() => setSure(false)}>Not now</button></div>` : null}
          ${!st.running ? html`<button type="button" class="ghost" id="set-steam-write" disabled=${busy} onClick=${() => run("steam_apply")}>Write them now</button>` : null}
        </div>` : st.wanted?.length ? html`<p class="muted" id="set-steam-done">${st.wanted.length === 1 ? "Its shortcut is" : `Its ${st.wanted.length} shortcuts are`} in Steam${st.users?.length ? ` for ${(st.users.find((u) => u.id === st.user) || {}).name || st.user}` : ""}.</p>` : null}
    `}
  </section>`;
}

/** What Tailscale's doing, in a line. */
function tailscaleLine(ts) {
  if (!ts.answers) return "Tailscale is installed but isn't running.";
  if (ts.state === "Running") return `Tailscale is on: this device is ${ts.dns || ts.name || "on your Tailscale network"}${ts.ip ? ` at ${ts.ip}` : ""}${ts.tailnet ? ` on ${ts.tailnet}` : ""}${ts.version ? `, with Tailscale ${ts.version}` : ""}.`;
  if (ts.state === "NeedsLogin") return "Tailscale is installed and waiting for you to sign in.";
  if (ts.state === "Stopped") return "Tailscale is installed but turned off.";
  return "Tailscale is starting…";
}

/** Tailscale (D179), for reaching the library away from home. On the Steam Deck, Set up
 *  Tailscale runs Game Repo Go's own installer in a terminal window (Desktop Mode has one),
 *  where the Deck's password is typed, and Go opens Tailscale's sign-in page once Tailscale
 *  gives one; it says so when a SteamOS update has taken Tailscale away. Elsewhere, where to
 *  get Tailscale. */
function TailscaleSettings() {
  const rev = useStore(go, (x) => x.tailscaleRev);
  const [ts, setTs] = useState(null);
  const [sure, setSure] = useState(false);
  const [busy, setBusy] = useState(false);
  const linux = info.kind === "linux";
  const read = () => call("tailscale").then(setTs).catch(() => setTs({ found: false, answers: false }));
  useEffect(() => { if (linux) read(); }, [rev, linux]);
  // while a set-up runs, what Tailscale's doing shows as it changes
  useEffect(() => {
    if (!ts?.setting_up) return undefined;
    const t = setInterval(read, 3000);
    return () => clearInterval(t);
  }, [ts?.setting_up]);
  const run = async (cmd) => {
    setBusy(true);
    try {
      setTs(await call(cmd));
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
      setSure(false);
    }
  };
  const intro = html`<p class="muted">Tailscale lets Game Repo Go reach your library when you're away from home, through your own Tailscale network.</p>`;
  if (!linux) {
    return html`<section class="go-set" id="set-tailscale"><h2>Tailscale</h2>${intro}
      <p id="set-tailscale-app">Tailscale has an app of its own for ${info.kind === "android" ? "Android" : "Windows"}: install it from tailscale.com/download and sign in with the account your library's server uses.</p></section>`;
  }
  if (!ts) return html`<section class="go-set" id="set-tailscale"><h2>Tailscale</h2><p class="muted">Looking for Tailscale…</p></section>`;
  const game = info.game_mode;
  const mine = ts.steamos && (!ts.found || ts.ours);
  const label = ts.missing ? "Set up Tailscale again" : ts.found ? "Install the newest Tailscale" : "Set up Tailscale";
  return html`<section class="go-set" id="set-tailscale">
    <h2>Tailscale</h2>
    ${intro}
    ${ts.missing ? html`<p class="warn-note" id="set-tailscale-gone">Tailscale has gone from this Deck, most likely with a SteamOS update. Set it up again to reach your library away from home.</p>` : null}
    ${ts.found ? html`<p id="set-tailscale-state">${tailscaleLine(ts)}</p>`
      : ts.missing ? null
      : html`<p id="set-tailscale-none">${ts.steamos ? "Tailscale isn't on this Steam Deck yet." : "Tailscale isn't on this PC. Install it from your Linux distribution's packages (tailscale.com/download/linux) and sign in."}</p>`}
    ${ts.found && !ts.ours ? html`<p class="muted" id="set-tailscale-theirs">It was installed another way, so Game Repo Go leaves it as it is.</p>` : null}
    ${mine && !ts.found && ts.password === "none" ? html`<p class="muted" id="set-tailscale-password">This Deck has no password yet. The set-up asks you to choose one, since installing needs it.</p>` : null}
    ${ts.state === "NeedsLogin" && ts.auth_url ? html`<div><button type="button" id="set-tailscale-sign-in" disabled=${busy} onClick=${() => run("tailscale_sign_in")}>Sign in to Tailscale…</button></div>` : null}
    ${ts.setting_up ? html`<p class="muted" id="set-tailscale-running" role="status">The set-up is running in a terminal window: follow it there. Game Repo Go opens Tailscale's sign-in page when it's ready.</p>` : null}
    ${mine && game ? html`<p class="warn-note" id="set-tailscale-desktop">The set-up runs in a terminal window, which Game Mode can't show. Switch to Desktop Mode (Steam button › Power › Switch to Desktop), open Game Repo Go there and press ${label} in Settings. Tailscale carries on in Game Mode afterwards.</p>` : null}
    ${mine && !sure ? html`<div><button type="button" class=${ts.found && !ts.missing ? "ghost" : ""} id="set-tailscale-set-up" disabled=${busy || game} onClick=${() => setSure(true)}>${label}…</button></div>` : null}
    ${mine && sure ? html`<div class="warn-note" id="set-tailscale-sure">
        <p>A terminal window opens. In it:</p>
        <ol>
          ${ts.password === "none" ? html`<li>you choose a password for this Deck, since it has none and installing needs one;</li>` : null}
          <li>you type the Deck's password, and Tailscale is installed from Tailscale's own website, checked against what Tailscale published;</li>
          <li>${ts.state === "Running" ? "Tailscale stays signed in as it is." : "Game Repo Go opens Tailscale's sign-in page in your browser."}</li>
        </ol>
        <p class="muted">Game Repo Go never sees the password.${ts.found ? " Tailscale's settings and sign-in stay as they are." : ""}</p>
        <div class="go-set-do"><button type="button" id="set-tailscale-go" disabled=${busy} onClick=${() => run("tailscale_set_up")}>Open the terminal</button>
          <button type="button" class="ghost" onClick=${() => setSure(false)}>Not now</button></div>
      </div>` : null}
  </section>`;
}

/** Saves (D180–D186): synced with the library while Game Repo Go's open, where RetroArch
 *  and each emulator of its own keep them on this device, each folder one you can set
 *  (D192, D193), and the last sync. */
function SavesSettings({ settings, set, paired }) {
  const rev = useStore(go, (x) => x.savesRev);
  const [sv, setSv] = useState(null);
  useEffect(() => { call("saves").then(setSv).catch(() => setSv(null)); }, [rev, settings.saves, settings.background_saves, settings.retroarch, JSON.stringify(settings.save_folders || {})]);
  const last = sv?.last;
  const ra = sv?.retroarch;
  return html`<section class="go-set" id="set-saves">
    <h2>Saves</h2>
    <label class="check-row"><input type="checkbox" id="set-saves-on" checked=${settings.saves} onChange=${() => set({ saves: !settings.saves })} />
      <span>Sync saves and save states with the library</span></label>
    ${sv?.background?.can ? html`<label class="check-row"><input type="checkbox" id="set-saves-background" checked=${settings.background_saves} disabled=${!settings.saves} onChange=${() => set({ background_saves: !settings.background_saves })} />
      <span>Sync them in the background too, with Game Repo Go closed</span></label>
      <p class="muted" id="set-saves-background-note">${sv.background.error ? html`<span class="warn-note">${sv.background.error}</span>` : settings.saves && settings.background_saves && paired
        ? sv.background.how === "android"
          ? `About every ${sv.background.every} minutes while there's a network, when Android lets it: it puts background work off to save the battery, so open Game Repo Go before you play to be sure of the newest save.`
          : `Every ${sv.background.every} minutes while you're signed in (${sv.background.how === "schtasks" ? "a task in Windows' Task Scheduler" : "a systemd timer, in Game Mode too"}).`
        : "Saves are synced only while Game Repo Go is open."}</p>` : null}
    <p class="muted">Each game's saves go to the library, every version kept, and saves played on your other devices come here when Game Repo Go is open. A save is never written while its emulator is running, and one played both here and elsewhere is kept as it is here until you choose one in Game Repo.</p>
    ${sv ? html`<p id="set-saves-retroarch" class=${ra ? null : "warn-note"}>${ra ? `RetroArch's saves are in ${ra.saves || "the games' own folders"}${ra.states && ra.states !== ra.saves ? `, its states in ${ra.states}` : ""}${ra.config ? ` (as ${ra.config} says)` : ""}.` : "RetroArch isn't found on this device: set where its retroarch.cfg is below."}</p>` : null}
    ${last?.at ? html`<p class="muted" id="set-saves-last">Last synced ${ago(last.at)}: ${count(last.sent)} sent, ${count(last.brought)} brought${last.waiting ? `, ${count(last.waiting)} waiting for an emulator to close` : ""}.${last.error ? ` ${last.error}` : ""}</p>` : null}
    ${last?.conflicts?.length ? html`<p class="warn-note" id="set-saves-conflicts">${last.conflicts.length === 1 ? "A save differs" : `${count(last.conflicts.length)} saves differ`} here and in the library: this device keeps its own until you choose one on the game's page in Game Repo.</p>` : null}
    <label class="field-block"><span>RetroArch's settings file</span>
      <input id="set-retroarch" value=${settings.retroarch} placeholder="Found by itself (retroarch.cfg)" onChange=${(e) => set({ retroarch: e.currentTarget.value.trim() })} /></label>
    ${sv?.emulators?.length ? html`<h3>Emulators of their own</h3>
      <p class="muted">Each keeps its saves in a folder of its own. Those found are synced; set the folder of one that isn't found, or that you've moved (on Android 11 and later an app's own Android/data folder can't be read, so move it somewhere it can).</p>
      <ul class="go-emus" id="set-saves-emulators">${sv.emulators.map((e) => html`<li key=${e.id} data-emulator=${e.id} data-found=${e.folder ? "yes" : "no"}>
        <label class="field-block"><span>${e.name} <span class=${e.folder ? "muted" : "warn-note"}>${e.folder ? `found in ${e.folder}` : "not found"}</span></span>
          <input id=${`set-folder-${e.id}`} data-folder=${e.id} value=${e.set} placeholder=${e.folder && !e.set ? e.folder : "Its folder"} title=${e.about}
            onChange=${(ev) => set({ save_folders: { ...(settings.save_folders || {}), [e.id]: ev.currentTarget.value.trim() } })} /></label></li>`)}</ul>` : null}
    <div class="go-set-do"><button type="button" class="ghost" id="set-saves-sync" disabled=${!paired || !settings.saves || sv?.running} onClick=${() => call("saves_sync").catch((e) => say(message(e)))}>${sv?.running ? "Syncing saves…" : "Sync saves now"}</button></div>
  </section>`;
}

export function SettingsPage() {
  const s = useStore(go, (x) => ({ settings: x.settings, status: x.status }));
  const [error, setError] = useState("");
  const [lk, setLk] = useState(look());
  const st = s.status;
  const v = s.settings;
  if (!v || !st) return html`<section class="go-page go-settings"><p class="muted">Opening Settings…</p></section>`;
  const set = async (patch) => {
    setError("");
    try {
      go.set({ settings: await call("set_settings", patch) });
      if ("show_every_system" in patch || "show_on_device" in patch) refreshLists();
    } catch (e) {
      setError(message(e));
      refreshSettings();
    }
  };
  const kinds = new Set(v.media);
  const flip = (k) => set({ media: KINDS.map(([x]) => x).filter((x) => x !== "manual" && (x === k ? !kinds.has(k) : kinds.has(x))) });
  const dev = st.device || {};
  const synced = st.catalogue?.synced;
  const changeLook = (patch) => { setLook(patch); setLk(look()); };
  return html`<section class="go-page go-settings" id="go-settings">
    <h1>Settings</h1>
    ${error ? html`<p class="form-error" role="alert" id="settings-error">${error}</p>` : null}

    <section class="go-set" id="set-library">
      <h2>Library</h2>
      ${st.paired ? html`<p><strong id="set-library-name">${v.library || "Library"}</strong> <span class="muted">at ${st.library?.address}</span></p>
        <p class="muted" id="set-library-kept">${count(st.catalogue?.games)} games kept on this device${synced ? `, read ${ago(synced)}` : ""}.
          ${st.sync?.running ? " Reading it now…" : st.sync?.error ? ` ${st.sync.error}` : ""}</p>
        <div class="go-set-do">
          <button type="button" class="ghost" id="set-sync" disabled=${st.sync?.running} onClick=${() => call("sync").then(refreshStatus).catch((e) => say(message(e)))}>Read the library now</button>
          <button type="button" class="ghost" id="set-pair-again" onClick=${() => showDialog({ type: "pair" })}>Pair with another library…</button>
          <button type="button" class="ghost danger-text" id="set-unpair" onClick=${() => showDialog({ type: "unpair", library: v.library })}>Unpair…</button>
        </div>` : html`<p class="muted">Not paired with a library.</p>`}
    </section>

    <section class="go-set" id="set-esde">
      <h2>ES-DE on this device</h2>
      ${dev.error ? html`<p class="warn-note" id="set-no-esde">${dev.error}</p>` : null}
      ${(dev.installs || []).length > 1 || (dev.installs?.length && dev.error) ? html`<div class="field-block" id="set-installs"><span>ES-DE set up on this device</span>
        ${dev.installs.map((i) => { const on = dev.esde === i.esde; return html`<div class=${`go-install${on ? " is-on" : ""}`} key=${i.esde}>
          <span><strong>${i.by}</strong> <span class="muted">· ROMs in ${i.roms}</span></span>
          ${on ? html`<span class="badge go-from go-from-library">In use</span>` : html`<button type="button" class="ghost" onClick=${() => set({ esde: i.esde, roms: "" }).then(refreshStatus)}>Use this one</button>`}
        </div>`; })}
      </div>` : dev.by && dev.by !== "ES-DE" ? html`<p class="muted" id="set-by">Set up by ${dev.by}.</p>` : null}
      ${dev.esde_roms ? html`<div class="warn-note" id="set-roms-mismatch"><span>ES-DE's own settings say its ROMs folder is ${dev.esde_roms}, so it won't find the games Game Repo Go puts in ${dev.roms}.</span>
        <button type="button" class="ghost" onClick=${() => set({ roms: "" }).then(refreshStatus)}>Use ES-DE's</button></div>` : null}
      <${Folder} id="set-esde-folder" label="ES-DE's folder" value=${v.esde || dev.esde} setting="esde" fallback="ES-DE's folder, with settings/es_settings.xml in it" />
      <${Folder} id="set-roms-folder" label="ROMs folder" value=${v.roms || dev.roms} setting="roms" fallback="As ES-DE's settings say" />
      ${dev.media ? html`<p class="muted">Pictures and manuals go in ${dev.media}, where ES-DE looks for them.</p>` : null}
      ${v.esde || v.roms ? html`<div><button type="button" class="ghost" id="set-folders-found" onClick=${() => set({ esde: "", roms: "" }).then(refreshStatus)}>Use the folders ES-DE says</button></div>` : null}
    </section>

    ${st.pc ? html`<section class="go-set" id="set-pc">
      <h2>PC games</h2>
      <${Folder} id="set-pc-folder" label="PC games folder" value=${st.pc.folder} setting="pc_games" fallback=${info.kind === "windows" ? "C:\\Games" : "Games in your home folder"} />
      <p class="muted" id="set-pc-shortcuts">Each game goes in a folder of its own there, with a shortcut ${info.kind === "windows" ? "in the Start menu's Game Repo Go folder" : "in Steam, through Proton for a Windows game"}. Removing a game keeps the files it made itself, such as saves.</p>
      ${!st.pc.default ? html`<div><button type="button" class="ghost" id="set-pc-usual" onClick=${() => set({ pc_games: "" }).then(refreshStatus)}>Use the usual folder</button></div>` : null}
    </section>` : null}

    <section class="go-set" id="set-downloads">
      <h2>Downloads</h2>
      <div class="field-block" id="set-kinds"><span>Pictures and manuals chosen when adding games</span>
        <div class="go-kinds">${KINDS.filter(([k]) => k !== "manual").map(([k, name]) => html`<label class="check-row" key=${k}>
          <input type="checkbox" name=${k} checked=${kinds.has(k)} onChange=${() => flip(k)} /><span>${name}</span></label>`)}
          <label class="check-row"><input type="checkbox" name="manual" checked=${v.manuals} onChange=${() => set({ manuals: !v.manuals })} /><span>Manual</span></label>
        </div>
        <small class="muted">Download… starts with these ticked and keeps what you choose there for next time; Add to queue takes them as they are.</small>
      </div>
      <label class="field-block"><span>Files downloaded at once</span>
        <select id="set-at-once" class="compact" value=${String(v.at_once)} onChange=${(e) => set({ at_once: +e.currentTarget.value })}>
          ${[1, 2, 3, 4].map((n) => html`<option value=${String(n)}>${n}</option>`)}
        </select>
      </label>
      <label class="field-block"><span>Put the games added in an ES-DE custom collection</span>
        <select id="set-collection" class="compact" value=${v.collection} onChange=${(e) => set({ collection: e.currentTarget.value })}>
          <option value="none">No</option>
          <option value="one">Yes, one for every game Game Repo Go adds</option>
          <option value="came-from">Yes, the collection they were added from</option>
        </select>
      </label>
      ${v.collection === "one" ? html`<label class="field-block"><span>Its name</span>
        <input id="set-collection-name" value=${v.collection_name} onChange=${(e) => set({ collection_name: e.currentTarget.value.trim() || "Game Repo Go" })} /></label>` : null}
    </section>

    <section class="go-set" id="set-showing">
      <h2>The Library</h2>
      <label class="check-row"><input type="checkbox" id="set-every-system" checked=${v.show_every_system} onChange=${() => set({ show_every_system: !v.show_every_system })} />
        <span>Show every system, not only those ES-DE has a folder for on this device</span></label>
      <label class="check-row"><input type="checkbox" id="set-on-device" checked=${v.show_on_device} onChange=${() => set({ show_on_device: !v.show_on_device })} />
        <span>Show games already on this device</span></label>
    </section>

    <section class="go-set" id="set-device">
      <h2>This device</h2>
      <label class="field-block"><span>Its name, as Game Repo shows it</span>
        <input id="set-name" value=${v.name} onChange=${(e) => set({ name: e.currentTarget.value.trim() })} />
        <small class="muted">Game Repo learns a new name when this device pairs again.</small></label>
      <label class="field-block"><span>Pictures made for a screen of</span>
        <select id="set-screen" class="compact" value=${String(v.screen)} onChange=${(e) => set({ screen: +e.currentTarget.value })}>
          ${[[640, "640 pixels (small handhelds)"], [960, "960 pixels"], [1280, "1280 pixels (most handhelds)"], [1920, "1920 pixels (full HD)"], [2560, "2560 pixels"], [3840, "3840 pixels (4K)"]].map(([n, t]) => html`<option value=${String(n)}>${t}</option>`)}
        </select></label>
      <label class="field-block"><span>Box fronts kept for browsing</span>
        <select id="set-pictures" class="compact" value=${String(v.pictures_mb)} onChange=${(e) => set({ pictures_mb: +e.currentTarget.value })}>
          ${[100, 250, 500, 1000, 2000, 5000].map((n) => html`<option value=${String(n)}>Up to ${bytes(n * 1024 * 1024)}</option>`)}
        </select>
        <small class="muted" id="set-pictures-kept">${count(st.pictures?.kept)} kept, ${bytes(st.pictures?.size)}. The least recently shown go first.</small></label>
      ${info.kind === "linux" || info.kind === "windows" ? html`<label class="field-block"><span>Full screen</span>
        <select id="set-full-screen" class="compact" value=${v.full_screen} onChange=${(e) => set({ full_screen: e.currentTarget.value })}>
          ${(info.kind === "linux" ? [["usual", "In Steam's Game Mode (the usual)"], ["always", "Always"], ["never", "Never"]] : [["usual", "No (the usual)"], ["always", "Yes"]]).map(([n, t]) => html`<option value=${n}>${t}</option>`)}
        </select>
        <small class="muted">Otherwise it opens as a window, as big as the screen when the screen is small.</small></label>` : null}
    </section>

    ${info.kind === "linux" ? html`<${SteamSettings} rev=${v} />` : null}

    <${SavesSettings} settings=${v} set=${set} paired=${st.paired} />

    <${TailscaleSettings} />

    <section class="go-set" id="set-look">
      <h2>Look</h2>
      <label class="field-block"><span>View</span>
        <select id="set-view" class="compact" value=${lk.view || usualView()} onChange=${(e) => changeLook({ view: e.currentTarget.value })}>
          <option value="big">Big screen, for a controller</option>
          <option value="classic">Classic, for a mouse and keyboard</option></select>
        <small class="muted">The big screen is like Steam's Big Picture: shelves of box fronts, large type, and the controller's buttons shown along the bottom. ${usualView() === "big" ? "It's the usual view on this device." : "The classic view is the usual one on this device."}</small></label>
      ${(lk.view || usualView()) === "big" ? html`<label class="field-block"><span>Size</span>
        <select id="set-size" class="compact" value=${lk.size} onChange=${(e) => changeLook({ size: e.currentTarget.value })}>
          <option value="smaller">Smaller, for a phone</option>
          <option value="">As it is, for a TV or the Steam Deck</option>
          <option value="larger">Larger, for a TV across the room</option></select>
        <small class="muted">The big screen's type, box fronts and buttons, kept for this device.</small></label>` : null}
      <label class="field-block"><span>Skin</span>
        <select id="set-skin" class="compact" value=${lk.skin} onChange=${(e) => changeLook({ skin: e.currentTarget.value })}>
          ${SKINS.map(([k, name]) => html`<option value=${k}>${name}</option>`)}</select></label>
      <label class="field-block"><span>Theme</span>
        <select id="set-theme" class="compact" value=${lk.theme} onChange=${(e) => changeLook({ theme: e.currentTarget.value })}>
          ${THEMES.map(([k, name]) => html`<option value=${k}>${name}</option>`)}</select></label>
    </section>

    <section class="go-set" id="set-about">
      <h2>About and updates</h2>
      <p>Game Repo Go <span id="set-version">${st.version}</span>${info.kind ? ` for ${info.kind[0].toUpperCase()}${info.kind.slice(1)}` : ""}. Free software under the MIT licence.</p>
      <${UpdatePanel} call=${call} settings=${true} />
    </section>
  </section>`;
}
