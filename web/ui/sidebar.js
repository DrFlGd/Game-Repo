// The menu on the left, by what each page does (D71–D72): Home; the Library with PC games, Magazines and Videos under it;
// Import; Audit with BIOS files, Metadata and Documents under it; Collections; Export layouts; History; the pages
// each phase still brings; Settings, Help and About. In a narrow window it's a drawer; on a wide one it folds to its icons.
import { html } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { routeHash } from "./context.js";
import { Icon } from "./icons.js";
import { COMING } from "./coming.js";
import { NavHead, useMedia, NARROW } from "./chrome.js";
import { SidePanel, DrawerRows } from "./panels.js";

function NavLink({ route, label, icon, current, note = null, off = false, child = false }) {
  const active = current === route;
  return html`${NavRow({ route, label, icon, active, note, off, child })}${active && ui.get().navOpen ? html`<${DrawerRows} route=${route} />` : null}`;
}

function NavRow({ route, label, icon, active, note, off, child }) {
  return html`<li class=${child ? "nav-row nav-child" : "nav-row"}>
    <span class="nav-toggle-space"></span>
    <a class=${`nav-link${active ? " active" : ""}${off ? " off" : ""}`} id=${`nav-${route}`} href=${routeHash(route)} title=${off ? `${label} (open a library first)` : note ? `${label} (${note})` : label} aria-label=${label} aria-current=${active ? "page" : null}
      onClick=${() => ui.set({ navOpen: false })}>
      <span class="nav-icon">${Icon[icon](15)}</span><span class="nav-label">${label}</span>
</a></li>`;
}

export function Sidebar() {
  const s = useStore(ui, (st) => ({ route: st.route, navOpen: st.navOpen, navCollapsed: st.navCollapsed, open: !!st.library }));
  const narrow = useMedia(NARROW);
  const folded = s.navCollapsed && !narrow;
  return html`${s.navOpen ? html`<div class="nav-backdrop" onClick=${() => ui.set({ navOpen: false })}></div>` : null}
    <nav class=${`sidebar${s.navOpen ? " open" : ""}${folded ? " collapsed" : ""}`} aria-label="Library">
      <${NavHead} />
      <ul class="nav-list">
        <${NavLink} route="home" label="Home" icon="home" current=${s.route} />
        <${NavLink} route="library" label="Library" icon="grid" current=${s.route === "game" ? "library" : s.route} off=${!s.open} />
        <${NavLink} route="pc" label="PC games" icon="monitor" current=${s.route} off=${!s.open} child=${true} />
        <${NavLink} route="magazines" label="Magazines" icon="stack" current=${s.route} off=${!s.open} child=${true} />
        <${NavLink} route="videos" label="Videos" icon="video" current=${s.route} off=${!s.open} child=${true} />
        <${NavLink} route="import" label="Import" icon="inbox" current=${s.route} off=${!s.open} />
        <${NavLink} route="audit" label="Audit" icon="audit" current=${s.route} off=${!s.open} />
        <${NavLink} route="bios" label="BIOS files" icon="chip" current=${s.route} off=${!s.open} child=${true} />
        <${NavLink} route="metadata" label="Metadata" icon="scrape" current=${s.route} off=${!s.open} child=${true} />
        <${NavLink} route="documents" label="Documents" icon="book" current=${s.route} off=${!s.open} child=${true} />
        <${NavLink} route="collections" label="Collections" icon="collection" current=${s.route} off=${!s.open} />
        <${NavLink} route="layouts" label="Export layouts" icon="layers" current=${s.route} off=${!s.open} />
        <${NavLink} route="history" label="History" icon="clock" current=${s.route} off=${!s.open} />
      </ul>
      ${Object.keys(COMING).length ? html`<p class="nav-head">Coming</p>
      <ul class="nav-list" id="coming-pages">
        ${Object.entries(COMING).map(([route, p]) => html`<${NavLink} key=${route} route=${route} label=${p.title} icon=${p.icon} current=${s.route} note=${`coming in Phase ${p.phase}`} />`)}
      </ul>` : null}
      <ul class="nav-list nav-foot">
        <${NavLink} route="settings" label="Settings" icon="cog" current=${s.route} />
        <${NavLink} route="help" label="Help" icon="help" current=${s.route} />
        <${NavLink} route="about" label="About" icon="info" current=${s.route} />
      </ul>
    </nav>
    <${SidePanel} />`;
}
