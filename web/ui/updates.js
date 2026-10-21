// Updates (docs/PLAN.md, "In-app updates", D134): whether a newer Game Repo, Game Repo Server
// or Game Repo Go is out, what's new in it, and Update and restart. One panel for Game Repo's
// Settings and About, the server's window and Game Repo Go's Settings, each passing how to
// reach its program: `call(cmd, args)` runs update_state, update_check, update_install or
// update_set (desktop/update, through the program's own `api` command). Like serve.js it
// imports nothing of the app's, so the server's window loads only these.
import { html, useEffect } from "../lib/html.js";
import { createStore, useStore } from "../lib/store.js";
import { Markdown, releases } from "./markdown.js";

const message = (e) => e?.message || String(e);

/** How updates stand (`state`: update_state's answer), shared by the panels and the notices. */
export const updates = createStore({ state: null, error: "", call: null });

/** Is version a newer than b? */
export function newer(a, b) {
  const pa = String(a).split(".").map(Number), pb = String(b).split(".").map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) > (pb[i] || 0);
  return false;
}

/** An update's notes for the versions after `version`, newest first. */
export const whatsNew = (notes, version) => releases(notes || "").filter((r) => r.version && newer(r.version, version));

async function ask(cmd, args = {}) {
  const call = updates.get().call;
  if (!call) return null;
  try {
    const state = await call(cmd, args);
    updates.set({ state, error: "" });
    return state;
  } catch (e) {
    updates.set({ error: message(e) });
    if (cmd !== "update_state") await ask("update_state");
    return null;
  }
}

/** When the program opens: how updates stand once the check it makes as it starts is done
 *  (Game Repo's status bar and Game Repo Go's notice show a newer version). */
export function startUpdates(call) {
  updates.set({ call });
  return ask("update_state", { wait: true });
}

/** How updates stand, asked again (Game Repo Go's "go-changed" "update"). */
export const refreshUpdates = () => ask("update_state");

const time = (secs) => new Date(secs * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** The panel: this version, a check, a newer version with what's new and Update and restart,
 *  a download's progress, and (with `settings`) whether to check when the program starts. */
export function UpdatePanel({ call, settings = false }) {
  const { state: s, error } = useStore(updates, (x) => ({ state: x.state, error: x.error }));
  useEffect(() => {
    if (updates.get().call !== call) updates.set({ call });
    ask("update_state");
  }, [call]);
  const busy = !!(s?.checking || s?.installing || s?.restarting);
  // asked again often while something's under way, and now and then otherwise (the server's
  // tray menu checks too)
  useEffect(() => {
    const t = setInterval(() => ask("update_state"), busy ? 700 : 10000);
    return () => clearInterval(t);
  }, [busy, call]);
  if (!s) return html`<div class="update-panel" id="update-panel"><p class="muted">${error || "Asking about updates…"}</p></div>`;
  const title = s.title || "Game Repo";
  const got = s.available;
  const notes = got ? whatsNew(got.notes, s.version) : [];
  const pct = s.installing?.size ? Math.min(100, Math.round((s.installing.done / s.installing.size) * 100)) : null;
  let line;
  if (!s.can) line = html`<span class="muted" id="update-why">${s.why_not}</span>`;
  else if (s.restarting) line = html`<span>Starting ${title} ${got?.version}: ${title} closes, and opens again once it's in place.</span>`;
  else if (s.installing) line = html`<span>Downloading ${title} ${got?.version}${pct != null ? ` (${pct}%)` : ""}…</span>
    <progress class="update-progress" id="update-progress" max="100" value=${pct ?? undefined}></progress>`;
  else if (s.checking) line = html`<span class="muted">Checking for updates…</span>`;
  else if (got) line = html`<span><strong>${title} ${got.version} is out.</strong> This is ${s.version}.</span>`;
  else if (s.checked && !s.error) line = html`<span>${title} ${s.version} is the newest version <span class="muted">(checked at ${time(s.checked)})</span>.</span>`;
  else line = html`<span class="muted">${title} ${s.version}.</span>`;
  return html`<div class="update-panel" id="update-panel" data-state=${!s.can ? "off" : busy ? "busy" : got ? "available" : "newest"}>
    <div class="update-state" id="update-state" aria-live="polite">${line}</div>
    ${s.error && !busy ? html`<p class="warn-note" role="alert" id="update-error">${s.error}</p>` : null}
    ${error && !s.error ? html`<p class="form-error" role="alert">${error}</p>` : null}
    ${s.can ? html`<div class="home-actions">
      ${got ? html`<button type="button" class="primary" id="update-install" disabled=${busy} onClick=${() => ask("update_install")}>Update and restart</button>` : null}
      <button type="button" class="ghost" id="update-check" disabled=${busy} onClick=${() => ask("update_check")}>${s.error ? "Check again" : "Check for updates"}</button>
    </div>` : null}
    ${notes.length ? html`<details class="update-new" id="update-new" open=${!busy}><summary>What's new</summary>
      ${notes.map((r) => html`<div class="update-release" key=${r.version}><h4>${r.title}</h4><${Markdown} text=${r.body} /></div>`)}</details>` : null}
    ${settings && s.can ? html`<label class="check-row"><input type="checkbox" id="update-at-start" checked=${s.check_at_start}
        onChange=${(e) => ask("update_set", { check_at_start: e.target.checked })} /> Check for updates when ${title} starts</label>
      <small class="muted">Nothing is installed until you choose Update and restart. Each update is checked against the key ${title} was built with before it runs.</small>` : null}
    ${s.releases ? html`<p class="muted"><a href=${s.releases} target="_blank" rel="noopener noreferrer">All releases on GitHub</a></p>` : null}
  </div>`;
}
