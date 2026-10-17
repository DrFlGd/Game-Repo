// The parts around the main view: the menu's head and theme button, and the status bar.
import { html, useEffect, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, setPref, cycleTheme, resolvedTheme, nextTheme, THEMES } from "./state.js";
import { Icon } from "./icons.js";

const themeName = (t) => (t === "system" ? "System" : THEMES.find(([v]) => v === t)?.[1].replace(/ \(.*\)$/, "") || t);

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

/** The app's mark: a cartridge, drawn for each skin. */
export function Mark({ cls = "brand-mark" }) {
  return html`<svg class=${`${cls} icon`} viewBox="0 0 40 40" aria-hidden="true">
    <g data-sk="classic" transform="scale(1.6667)"><path d="M6.5 3.5h11v13.5l-1.5 1.5v2h-8v-2l-1.5-1.5z" /><rect x="8.5" y="6" width="7" height="6" rx=".5" class="accent" /><path d="M9.5 18.5v2M12 18.5v2M14.5 18.5v2" /></g>
    <g data-sk="drafting" fill="none"><rect x="2.5" y="2.5" width="35" height="35" stroke="var(--ink)" stroke-width="1.5"/><path d="M12 6.5h16v19.5l-2.5 2.5v3h-11v-3L12 26z" stroke="var(--tool)" stroke-width="1.6" stroke-linejoin="miter"/><path d="M15 10h10v8H15z" stroke="var(--tool)" stroke-width="1.6"/><path d="M17 28.5v3M20 28.5v3M23 28.5v3" stroke="var(--tool)" stroke-width="1.2"/><path d="M7 34.5h26M7 33v3M33 33v3" stroke="var(--ink)" stroke-width="1"/></g>
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
    <button type="button" class="ghost topicon nav-burger" aria-label=${narrow ? "Close the menu" : nav.collapsed ? "Expand navigation" : "Collapse navigation"} aria-expanded=${nav.open ? "true" : "false"} title="Expand or collapse navigation" onClick=${toggleNav}>${Icon.menu(18)}</button>
    <a class="brand" href="#/" title="Home"><${Mark} /><span class="nav-label">Game Repo</span></a>
  </div>`;
}

/** The theme button at the foot of the menu: the same four choices as Settings, in turn. */
export function ThemeButton() {
  const theme = useStore(ui, (s) => s.theme);
  const [, force] = useState(0);
  useEffect(() => { const f = () => force((n) => n + 1); window.addEventListener("gr-theme", f); return () => window.removeEventListener("gr-theme", f); }, []);
  const cur = resolvedTheme();
  const next = nextTheme(theme);
  const icon = theme === "system" ? Icon.auto(15) : cur === "light" ? Icon.sun(15) : cur === "dark" ? Icon.moon(15) : Icon.night(15);
  const now = theme === "system" ? `Follow the system (${cur})` : themeName(theme);
  return html`<li class="nav-row"><span class="nav-toggle-space"></span>
    <button type="button" class="nav-link nav-theme" id="theme-toggle" data-theme=${theme} aria-label=${`Theme: ${now}. Switch to ${themeName(next)}`} title=${`Theme: ${now}. Click for ${themeName(next)}.`}
      onClick=${cycleTheme}><span class="nav-icon">${icon}</span><span class="nav-label">Theme: ${themeName(theme)}</span></button></li>`;
}

function Elapsed({ since }) {
  const [, tick] = useState(0);
  useEffect(() => { const t = setInterval(() => tick((n) => n + 1), 1000); return () => clearInterval(t); }, []);
  return html`${Math.round((Date.now() - since) / 1000)} s`;
}

export function StatusBar() {
  const s = useStore(ui, (st) => ({ jobs: st.jobs, ready: st.ready, library: st.library }));
  const [open, setOpen] = useState(false);
  if (!s.ready) return null;
  const latest = s.jobs[s.jobs.length - 1];
  return html`<div class="statusbar-inner">
    <span class="status-library" title=${s.library?.path || ""}>${s.library ? s.library.name : "No library open"}</span>
    ${s.library?.read_only ? html`<span>Read-only</span>` : null}
    <span class="status-jobs">
      ${latest ? html`<button type="button" class="status-job" aria-expanded=${open ? "true" : "false"} title="Show what's running" onClick=${() => setOpen(!open)}>
        <span class="dot busy"></span>${latest.label}… <${Elapsed} since=${latest.started} />${s.jobs.length > 1 ? ` · ${s.jobs.length} running` : ""}</button>`
        : html`<span class="status-idle"><span class="dot"></span>Ready</span>`}
      ${open && s.jobs.length ? html`<div class="jobs-panel" role="dialog" aria-label="Background work">
        ${s.jobs.map((j) => html`<div class="job" key=${j.id}><span>${j.label}</span><span class="muted"><${Elapsed} since=${j.started} /></span>
          ${j.cancel ? html`<button type="button" class="ghost" onClick=${() => j.cancel()}>Stop</button>` : null}</div>`)}
      </div>` : null}
    </span>
  </div>`;
}
