// About: the app's version and what it's built with, its updates, how a library is laid out,
// what's coming, the licences and the changelog. tools/build_desktop.py copies
// CHANGELOG.md and THIRD_PARTY.md next to the page and writes build-info.json,
// so this works offline.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { CommandBar } from "./layout.js";
import { Markdown, releases } from "./markdown.js";
import { UpdatePanel } from "./updates.js";

const OS = { windows: "Windows", linux: "Linux", macos: "macOS" };

export const TREE = `_library/                       the app's own files
  library.json                  the library's name and settings
  games/<system>/<Game>/        one folder per game with more than its ROM
  games/<system>/<SHA1>/        what Identify found for a file no DAT knows
  games/pc/<id>/                a PC game's details, pictures and documents
    game.json                   its id, DAT match, ROM hashes and details
    sources/<source>.json       what each source said about it, kept apart
    media/                      the chosen pictures and video: cover.png, screenshot.jpg…
      <kind>/<source>.<ext>     the other sources' pictures of each kind
    docs/                       its guides, manuals, maps and scans (docs.json: their
                                kinds and titles), scraped manuals among them
  docs/<system>/  docs/General/
                                documents that go with no one game
  dats/<system>/                the DAT files you imported, every version
  dats/magazines/               lists of magazine issues, to check against
  collections/<id>.json         your collections, their rules and history
  collections/<id>/             a collection's own DATs, when it was made from folders or a DAT
  exports/<id>.json             what each export wrote, for Update and Check
  fixdats/  reports/            what's missing as DATs, and saved audit reports
  made-dats/                    DATs you made from folders of your own
  layouts/                      export layouts of your own (JSON), for Export…
  journal/                      every change to your files, for Undo
BIOS/<system>/                  BIOS files, imported from the BIOS files page
Nintendo - SNES/                system folders, when you import or move games in
  Chrono Trigger (USA).zip
Sony - PlayStation/
  Final Fantasy VII (USA)/      a folder for a game of several files
    … (Disc 1).chd
Magazines/
  <Title>/                      a magazine's issues, with magazine.json
PC/
  <Title> (<Year>)/             imported PC games
    install/  extracted/  patches/  extras/  dlc/  game.json`;

const DONE = [
  ["Phase 0", "The app, its library folder, the index on this computer and settings"],
  ["Phase 1", "Scanning and hashing your ROMs, importing DAT files, auditing cartridge systems, reports, fixdats and DATs made from your sets"],
  ["Phase 2a", "Organising sets by their DATs (copy, rename or move) with a preview and Undo, TorrentZips, 7z and RAR, and noticing changes in your folders"],
  ["Phase 2b", "Disc systems audited against Redump's DATs, CHD, CSO, ZSO and RVZ read as the games they hold, and converting to and from them"],
  ["Phase 3", "Collections from rules with one game per title, saved as DATs and exported to ES-DE or a folder and kept in step, and systems of your own"],
  ["Phase 4", "PC games and installers imported into one layout, identified from Steam, GOG and LaunchBox, unpacked and added to Steam; Add a game… for any system"],
  ["Phase 5", "Details, pictures, video and manuals scraped from ScreenScraper, IGDB, TheGamesDB and LaunchBox for a system's games or its whole DAT; your documents imported, paired with their games and searched by their words; exports that carry what you choose"],
  ["Phase 5b", "Exports to your handhelds' SD cards (muOS, Knulli, ROCKNIX, ES-DE on Android or Linux) in layouts you can copy and change in a visual editor, and BIOS files audited, imported and exported with the games"],
  ["Phase 6", "The Library: every game across the systems by its covers, in a grid, list or table, searched and filtered by its details and saved as a collection; a page for each game where its details are changed by hand and kept over scrapes"],
  ["Phase 7", "Arcade sets from MAME and FinalBurn Neo: BIOS sets, devices, disks and samples audited, what's playable, sets kept non-merged, split or merged, and exports that carry what each game needs"],
  ["Phase 8", "Exports to RetroArch, Pegasus, LaunchBox and Playnite; updates, DLC and patches kept with their game; a collection's history with Undo; and a collection of magazines, read here and checked against lists of issues"],
  ["UI passes", "The pages laid out again: the Library and Audit with their pages under them, one Import page, History, Help, collection groups, and imported games sorted into each system's own folder"],
  ["Hardening, part 1", "After a review of the whole app, changes to your files made safer: a file that turns up where a game goes is never written over, and Undo never removes what it can't put back"],
  ["Phase 9", "Game Repo Server, started from Settings, and Game Repo Go on Windows, Linux, the Steam Deck and Android: paired with a code, browsing the library (on a television too, in the big-screen view) and downloading games, with PC games installed and added to Steam"],
  ["Phase 10", "Variants: translations, hacks and prototypes kept in a DAT of your own beside each system's, with their tags, and a system audited with several DATs"],
  ["Your lists, October 2026", "Updates in the app for all three, signed; the Library's memory of where you were; videos, magazines and documents as shelves of their own, with magazines' text read; the backlog; updates and DLC with their game; Add a system, Redump's own DATs and every system's DATs updated; games on several discs as one"],
  ["Review, October 2026", "A review of the whole app's pages: the fixes it asked for, games converted to their system's format as they're imported, and the tools Game Repo uses downloaded from Settings"],
  ["Tailscale on the Steam Deck", "Game Repo Go sets up Tailscale on the Deck from its Settings, the first part of reaching your library away from home"],
];
export const NEXT = [
  ["Save sync and Devices", "Saves and save states synced between your devices with every version kept (RetroArch, PPSSPP, Dolphin, the PCSX2 family with ARMSX2, DuckStation, Eden, Cemu, RPCS3 and more), in the background with Game Repo Go closed, from handhelds' SD cards, then a Devices page for Game Repo Go devices, SD cards, drives and shares whose changes wait for the next connect (handhelds with no app among them)", true],
  ["Phase 9d", "The Server on Unraid and reached away from home over Tailscale, pairing by QR code, downloads on Wi-Fi only"],
  ["Android games", "APKs kept in the library and installed on Android devices through Game Repo Go"],
  ["GOG installers", "A PC game that has only its GOG installer unpacked, so Game Repo Go can install it too"],
];

export { Markdown, releases } from "./markdown.js";

const fetchText = (path) => fetch(path, { cache: "no-store" }).then((r) => (r.ok ? r.text() : null)).catch(() => null);

export function About() {
  const lib = useStore(ui, (s) => s.library);
  const info = ctx.platform?.info || {};
  const version = info.version || "";
  const [files, setFiles] = useState(null);
  useEffect(() => {
    let live = true;
    Promise.all([fetchText("CHANGELOG.md"), fetchText("THIRD_PARTY.md"), fetchText("build-info.json")]).then(([log, third, build]) => {
      let b = null;
      try { b = build ? JSON.parse(build) : null; } catch { b = null; }
      if (live) setFiles({ log, third, build: b });
    });
    return () => { live = false; };
  }, []);
  const list = files?.log ? releases(files.log) : [];
  const released = list.some((r) => r.version === version);
  const shown = list.filter((r) => r.version || r.body).map((r) => r.version ? { ...r, label: r.title, now: r.version === version }
    : { ...r, label: released || !version ? "Coming in the next version" : `${version} (this version)`, now: !released && !!version });
  const b = files?.build;
  const rows = [
    ["Game Repo", version || "unknown"],
    ["Running on", OS[info.os] || info.os || "unknown"],
    ["Library format", `${info.library_format ?? "?"} (this app writes)${lib?.format != null ? ` · the open library: ${lib.format}` : ""}`],
  ];
  const built = [
    ...(b ? [["Built", [b.date, b.commit].filter(Boolean).join(" · ")]] : []),
    ...(b?.parts || []).map((p) => [p.name, p.version]),
  ];
  return html`<${CommandBar} id="about-head" title="About" /><div class="pages"><div class="library-settings about-page" id="about-page">
    <p class="page-lead">Game Repo: audit, organise and export large collections of video games, with each game's details kept in a library folder that makes sense without the app.</p>
    <section class="ls-section" id="about-versions">
      <h2>Versions</h2>
      <dl class="about-versions">${rows.map(([k, v]) => html`<div key=${k}><dt>${k}</dt><dd>${v}</dd></div>`)}</dl>
      ${built.length ? html`<details class="about-built" id="about-built"><summary>How it was built</summary>
        <dl class="about-versions">${built.map(([k, v]) => html`<div key=${k}><dt>${k}</dt><dd>${v}</dd></div>`)}</dl></details>` : null}
      ${ctx.platform?.kind === "desktop" ? html`<${UpdatePanel} call=${ctx.platform.api} />`
        : html`<p><a href=${info.releases || "https://github.com/DrFlGd/Game-Repo/releases"} target="_blank" rel="noopener noreferrer">All releases on GitHub</a></p>`}
    </section>
    <section class="ls-section" id="about-changelog">
      <h2>What's changed</h2>
      ${!files ? html`<p class="muted">Reading the changelog…</p>`
        : !shown.length ? html`<p class="muted">The changelog isn't included in this copy of the app (it's added when the app is built).</p>`
        : shown.map((r, i) => html`<details class=${`about-release${r.now ? " now" : ""}`} key=${r.label} open=${r.now || (!shown.some((x) => x.now) && i === 0)}>
            <summary><strong>${r.label}</strong>${r.now ? html` <span class="about-badge">This version</span>` : null}</summary>
            <${Markdown} text=${r.body} />
          </details>`)}
    </section>
    <section class="ls-section" id="about-roadmap">
      <h2>What's coming</h2>
      <ol class="phase-list">${NEXT.map(([g, what, now]) => html`<li class=${now ? "now" : ""} key=${g}><strong>${g}:</strong> ${what}${now ? " (in progress)" : ""}</li>`)}</ol>
      <details><summary>Done so far</summary><ol class="phase-list">${DONE.map(([g, what]) => html`<li key=${g}><strong>${g}:</strong> ${what}</li>`)}</ol></details>
    </section>
    <section class="ls-section" id="about-layout">
      <h2>How a library is laid out</h2>
      <p class="muted">The app keeps its own files in <code>_library</code>; your games stay ordinary files, so the library makes sense in any file manager, with or without this app. Scraped details and documents are tied to a game by its ROM's hash (a PC game by its id), never by a file name.</p>
      <pre class="folder-tree" aria-label="Example folder layout">${TREE}</pre>
    </section>
    <section class="ls-section" id="about-licences">
      <h2>Licences and credits</h2>
      ${files?.third ? html`<${Markdown} text=${files.third.replace(/^# .*\n/, "")} />` : html`<p class="muted">The licence list isn't included in this copy of the app.</p>`}
    </section>
  </div></div>`;
}
