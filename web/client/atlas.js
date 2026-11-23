// Install Atlas (docs/PLAN.md, "Maps and Guides notes, part 13", D250–D255), in Settings ›
// Maps and guides for Atlas on Windows and Linux: offered only while Atlas isn't installed,
// and only on a press. Game Repo Go fetches Atlas's latest release from GitHub, checks it
// against Atlas's signature, and installs it: the installer run for this user on Windows,
// Atlas.AppImage put in ~/Applications on Linux and the Steam Deck. Once Atlas is there the
// block says where, and there's nothing to update: Atlas updates itself. Wherever Go says
// Atlas wasn't found (Show in Atlas, the launch line), an Install Atlas link brings you here.
import { html, useEffect, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { call, info } from "./backend.js";
import { go, open, say } from "./state.js";
import { bytes, message } from "./format.js";

/** The block's id, which the links scroll to. */
export const INSTALL_ID = "set-atlas-install";

/** Settings, at Install Atlas. */
export function openInstall() {
  open({ page: "settings" });
  setTimeout(() => document.getElementById(INSTALL_ID)?.scrollIntoView({ block: "center" }), 300);
}

/** A refusal to start Atlas said with its link: Install Atlas when Atlas wasn't found,
 *  else Open Settings when the words send you there. */
export function sayAtlas(m) {
  if (/^Atlas wasn't found/.test(m) && info.kind !== "android") say(m, `settings#${INSTALL_ID}`, "Install Atlas");
  else say(m, /Settings/.test(m) ? "settings" : null);
}

/** The link beside "Atlas isn't there" (the launch line, D241). */
export function InstallLink({ id }) {
  return html`<button type="button" class="ghost compact" id=${id} data-say="Settings, at Install Atlas" onClick=${openInstall}>Install Atlas</button>`;
}

const RUNNING = ["reading", "downloading", "installing"];

export function AtlasInstall() {
  const rev = useStore(go, (x) => x.atlasRev);
  const [st, setSt] = useState(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { call("atlas_install_state").then(setSt).catch(() => setSt(null)); }, [rev]);
  if (!st?.offered || info.kind === "android") return null;
  const p = st.progress || {};
  const running = RUNNING.includes(p.state);
  const found = st.installed;
  const install = async () => {
    setBusy(true);
    try {
      setSt(await call("atlas_install"));
    } catch (e) {
      say(message(e));
    } finally {
      setBusy(false);
    }
  };
  const linux = !!st.place;
  // the version: as Windows has it, else the one just installed
  const version = found?.version || (p.state === "done" ? p.version : null);
  const state = running ? p.state : found ? "installed" : "missing";
  const pct = p.total ? Math.min(100, Math.round((p.done / p.total) * 100)) : null;
  const doing = p.state === "reading" ? "Looking for Atlas's latest release…"
    : p.state === "downloading" ? `Downloading Atlas ${p.version || ""}… ${bytes(p.done || 0)}${p.total ? ` of ${bytes(p.total)}` : ""}`
    : `Installing Atlas ${p.version || ""}…`;
  return html`<div class="go-atlas-install" id=${INSTALL_ID} data-state=${state}>
    <h3>Atlas</h3>
    ${running ? html`<p id="set-atlas-install-doing" role="status">${doing}</p>
        ${p.state === "downloading" ? html`<progress class="update-progress" id="set-atlas-install-progress" max="100" value=${pct ?? undefined}></progress>` : null}`
      : found ? html`<p id="set-atlas-installed">Atlas${version ? ` ${version}` : ""} is installed: <span class="go-file">${found.program}</span>.</p>
        ${found.runnable ? null : html`<p class="warn-note" id="set-atlas-not-runnable">It can't be run as it is: tick <i>Is executable</i> in its Properties › Permissions.</p>`}
        <p class="muted" id="set-atlas-updates">Atlas updates itself, so Game Repo Go never changes it.</p>`
      : html`<p id="set-atlas-missing-said">Atlas isn't installed on this device.</p>
        ${p.state === "failed" && p.message ? html`<p class="warn-note" role="alert" id="set-atlas-install-said">${p.message}</p>` : null}
        <div><button type="button" class="primary" id="set-atlas-install-button" disabled=${busy} data-say="Downloads Atlas from its releases on GitHub, checks it and installs it" onClick=${install}>Install Atlas</button></div>
        <p class="muted" id="set-atlas-install-note">${linux
          ? html`Game Repo Go downloads Atlas.AppImage from Atlas's latest release on GitHub, checks it's signed by Atlas, and puts it in <span class="go-file">${st.place}</span>, ready to run. From then on Atlas updates itself.`
          : "Game Repo Go downloads the installer of Atlas's latest release from GitHub, checks it's signed by Atlas, and installs Atlas for you only, without asking for an administrator. From then on Atlas updates itself."}</p>`}
  </div>`;
}
