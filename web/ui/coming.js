// The pages still to come, each saying what it will do and in which phase
// (docs/PLAN.md, section 7). They're in the menu from the start so the app's
// shape is clear; each is replaced by the real page in its phase.
import { html } from "../lib/html.js";
import { CommandBar } from "./layout.js";
import { Icon } from "./icons.js";
import { routeHash } from "./context.js";

// (none just now: Metadata and Documents came in Phase 5)
export const COMING = {};

export function Coming({ page }) {
  const p = COMING[page];
  if (!p) return null;
  return html`<${CommandBar} id="coming-head" title=${p.title} count=${`Phase ${p.phase}`} /><div class="pages"><div class="library-settings coming-page" id=${`coming-${page}`}>
    <p class="page-lead">${Icon[p.icon](18)} ${p.lead}</p>
    <section class="ls-section">
      <h2>Coming in Phase ${p.phase}</h2>
      <ul class="coming-list">${p.points.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>
      <p class="muted">The plan for every phase is on the <a href=${routeHash("about")}>About</a> page.</p>
    </section>
  </div></div>`;
}
