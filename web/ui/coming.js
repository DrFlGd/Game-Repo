// The pages still to come, each saying what it will do and in which phase
// (docs/PLAN.md, section 7). They're in the menu from the start so the app's
// shape is clear; each is replaced by the real page in its phase.
import { html } from "../lib/html.js";
import { CommandBar } from "./layout.js";
import { Icon } from "./icons.js";
import { routeHash } from "./context.js";

export const COMING = {
  scraper: {
    title: "Metadata", icon: "scrape", phase: 5,
    lead: "Fill in each game's details, pictures, videos and manuals from online sources such as ScreenScraper.",
    points: [
      "Scraped details are tied to the game, found by its ROM's hash, never by its file name, so renaming or converting a ROM keeps them.",
      "Attach your own documents (guides, manuals, posters, maps) to a game; they're kept in its folder in the library with the scraped media.",
      "Accounts and keys for each source are yours to set in Settings; none are built in.",
    ],
  },
};

export function Coming({ page }) {
  const p = COMING[page];
  return html`<${CommandBar} id="coming-head" title=${p.title} count=${`Phase ${p.phase}`} /><div class="pages"><div class="library-settings coming-page" id=${`coming-${page}`}>
    <p class="page-lead">${Icon[p.icon](18)} ${p.lead}</p>
    <section class="ls-section">
      <h2>Coming in Phase ${p.phase}</h2>
      <ul class="coming-list">${p.points.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>
      <p class="muted">The plan for every phase is on the <a href=${routeHash("about")}>About</a> page.</p>
    </section>
  </div></div>`;
}
