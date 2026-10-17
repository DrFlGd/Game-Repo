// The menu on the left: Home, Audit, Collections, PC games, Metadata, Documents, the pages each phase still brings, and Settings. In a
// narrow window it's a drawer; on a wide one it folds to its icons.
import { html } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { routeHash } from "./context.js";
import { Icon } from "./icons.js";
import { COMING } from "./coming.js";
import { NavHead, ThemeButton, useMedia, NARROW } from "./chrome.js";

function NavLink({ route, label, icon, current, note = null }) {
  const active = current === route;
  return html`<li class="nav-row">
    <span class="nav-toggle-space"></span>
    <a class=${`nav-link${active ? " active" : ""}`} id=${`nav-${route}`} href=${routeHash(route)} title=${note ? `${label} (${note})` : label} aria-label=${label} aria-current=${active ? "page" : null}
      onClick=${() => ui.set({ navOpen: false })}>
      <span class="nav-icon">${Icon[icon](15)}</span><span class="nav-label">${label}</span>
</a></li>`;
}

export function Sidebar() {
  const s = useStore(ui, (st) => ({ route: st.route, navOpen: st.navOpen, navCollapsed: st.navCollapsed }));
  const narrow = useMedia(NARROW);
  const folded = s.navCollapsed && !narrow;
  return html`${s.navOpen ? html`<div class="nav-backdrop" onClick=${() => ui.set({ navOpen: false })}></div>` : null}
    <nav class=${`sidebar${s.navOpen ? " open" : ""}${folded ? " collapsed" : ""}`} aria-label="Library">
      <${NavHead} />
      <ul class="nav-list">
        <${NavLink} route="home" label="Home" icon="home" current=${s.route} />
        <${NavLink} route="audit" label="Audit" icon="audit" current=${s.route} />
        <${NavLink} route="collections" label="Collections" icon="collection" current=${s.route} />
        <${NavLink} route="pc" label="PC games" icon="monitor" current=${s.route} />
        <${NavLink} route="metadata" label="Metadata" icon="scrape" current=${s.route} />
        <${NavLink} route="documents" label="Documents" icon="book" current=${s.route} />
      </ul>
      ${Object.keys(COMING).length ? html`<p class="nav-head">Coming</p>
      <ul class="nav-list" id="coming-pages">
        ${Object.entries(COMING).map(([route, p]) => html`<${NavLink} key=${route} route=${route} label=${p.title} icon=${p.icon} current=${s.route} note=${`coming in Phase ${p.phase}`} />`)}
      </ul>` : null}
      <ul class="nav-list nav-foot">
        <${NavLink} route="settings" label="Settings" icon="cog" current=${s.route} />
        <${NavLink} route="about" label="About" icon="info" current=${s.route} />
        <${ThemeButton} />
      </ul>
    </nav>`;
}
