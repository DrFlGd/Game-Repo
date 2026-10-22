// The parts around the main view: the menu's head and the status bar (the theme is chosen in Settings › Appearance).
import { html, useEffect, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, setPref } from "./state.js";
import { Icon } from "./icons.js";
import { updates } from "./updates.js";

export function useMedia(query) {
  const [on, setOn] = useState(() => matchMedia(query).matches);
  useEffect(() => {
    const q = matchMedia(query);
    const changed = () => setOn(q.matches);
    q.addEventListener("change", changed);
    return () => q.removeEventListener("change", changed);
  }, [query]);
  return on;
}

export const NARROW = "(max-width: 900px)";

/** The app's mark: a folder with a d-pad and buttons (tools/make_icons.py), drawn for each skin. */
export function Mark({ cls = "brand-mark" }) {
  return html`<svg class=${`${cls} icon`} viewBox="0 0 40 40" aria-hidden="true">
    <g data-sk="classic" transform="scale(1.6667)"><path d="M3 7a1.5 1.5 0 0 1 1.5-1.5h4.5l1.5 2h9a1.5 1.5 0 0 1 1.5 1.5v8.5a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z" /><path d="M6.6 12h1.6v-1.6h1.8v1.6h1.6v1.8h-1.6v1.6h-1.8v-1.6h-1.6z" class="accent" /><circle cx="15" cy="11.6" r="1.15" class="accent" /><circle cx="17.2" cy="14" r="1.15" class="accent" /></g>
  </svg>`;
}

/** The menu's head: its fold button (the drawer's close button in a narrow
 *  window) and the app's mark, a link to Home. */
export function NavHead() {
  const nav = useStore(ui, (s) => ({ open: s.navOpen, collapsed: s.navCollapsed }));
  const narrow = useMedia(NARROW);
  const toggleNav = () => {
    if (narrow) ui.set({ navOpen: !ui.get().navOpen });
    else setPref({ navCollapsed: !nav.collapsed });
  };
  return html`<div class="nav-head-row">
    <button type="button" class="ghost topicon nav-burger" aria-label=${narrow ? "Close the menu" : nav.collapsed ? "Unfold the menu" : "Fold the menu"} aria-expanded=${nav.open ? "true" : "false"} title="Fold or unfold the menu" onClick=${toggleNav}>${Icon.menu(18)}</button>
    <a class="brand" href="#/" title="Home"><${Mark} /><span class="nav-label">Game Repo</span></a>
  </div>`;
}

function Elapsed({ since }) {
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 1000); return () => clearInterval(t); }, []);
  return html`${Math.round((Date.now() - since) / 1000)} s`;
}

/** Settings › Updates, scrolled to. */
function showUpdates() {
  location.hash = "#/settings";
  setTimeout(() => document.getElementById("settings-updates")?.scrollIntoView({ block: "start" }), 50);
}

export function StatusBar() {
  const s = useStore(ui, (st) => ({ jobs: st.jobs, ready: st.ready, library: st.library }));
  // a newer version found as the app started (D134)
  const upd = useStore(updates, (x) => (x.state?.available && !x.state.installing && !x.state.restarting ? x.state : null));
  const [open, setOpen] = useState(false);
  if (!s.ready) return null;
  const latest = s.jobs[s.jobs.length - 1];
  return html`<div class="statusbar-inner">
    <span class="status-library" title=${s.library?.path || ""}>${s.library ? s.library.name : "No library open"}</span>
    ${s.library?.read_only ? html`<span>Read-only</span>` : null}
    ${upd ? html`<button type="button" class="status-update" id="status-update" title="Settings › Updates" onClick=${showUpdates}>${Icon.download(13)} ${upd.title} ${upd.available.version} is out</button>` : null}
    <span class="status-jobs">
      ${latest ? html`<button type="button" class="status-job" id="status-job" aria-expanded=${open ? "true" : "false"} title=${latest.open && s.jobs.length === 1 ? "Show it" : "Show what's running"}
        onClick=${() => (latest.open && s.jobs.length === 1 ? latest.open() : setOpen(!open))}>
        <span class="dot busy"></span>${latest.label}… <${Elapsed} since=${latest.started} />${s.jobs.length > 1 ? ` · ${s.jobs.length} running` : ""}
        ${latest.part != null ? html`<progress class="status-progress" id="status-progress" max="1" value=${latest.part}></progress>` : html`<progress class="status-progress" id="status-progress"></progress>`}</button>`
        : html`<span class="status-idle"><span class="dot"></span>Ready</span>`}
      ${open && s.jobs.length ? html`<div class="jobs-panel" role="dialog" aria-label="Background work">
        ${s.jobs.map((j) => html`<div class="job" key=${j.id}><span>${j.label}</span>${j.part != null ? html`<progress max="1" value=${j.part}></progress>` : null}<span class="muted"><${Elapsed} since=${j.started} /></span>
          ${j.open ? html`<button type="button" class="ghost" onClick=${() => { setOpen(false); j.open(); }}>Show</button>` : null}
          ${j.cancel ? html`<button type="button" class="ghost" onClick=${() => j.cancel()}>Stop</button>` : null}</div>`)}
      </div>` : null}
    </span>
  </div>`;
}
