// The pages still to come, each saying what it will do and in which phase
// (docs/PLAN.md, section 7). They're in the menu from the start so the app's
// shape is clear; each is replaced by the real page in its phase.
import { html } from "../lib/html.js";
import { CommandBar } from "./layout.js";
import { Icon } from "./icons.js";
import { routeHash } from "./context.js";

export const COMING = {
  audit: {
    title: "Audit", icon: "audit", phase: 1,
    lead: "Check your ROM sets against DAT files from No-Intro, Redump and others.",
    points: [
      "Scan the folders where your games are: every file is hashed once (inside ZIPs too) and remembered, so a second scan only reads what changed.",
      "Import DAT files and see, for each platform, what you have, what's missing, what's misnamed and what no DAT knows.",
      "Write a fixdat of what's missing, and rename or rebuild a set to match its DAT, with a dry run first and Undo after (Phase 2).",
    ],
  },
  collections: {
    title: "Collections", icon: "collection", phase: 3,
    lead: "Build collections from rules and export them for ES-DE or as a plain folder tree.",
    points: [
      "Pick games by platform, region, language, genre or a DAT of your own, with one game per title (1G1R) when you want it.",
      "Export to ES-DE with its system folders, gamelist.xml, media and custom collections, then keep the export in step as the library changes.",
    ],
  },
  import: {
    title: "PC games", icon: "monitor", phase: 4,
    lead: "Bring zipped PC games and installers into one layout.",
    points: [
      "Drop archives and installers in an inbox; the app works out what each game is (GOG ids, names and years) and asks when it isn't sure.",
      "Each game gets its own folder: PC/<Title> (<Year>)/ with install, extracted, patches and extras, and a game.json.",
      "Add your PC games to Steam as non-Steam shortcuts, with their artwork.",
    ],
  },
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
