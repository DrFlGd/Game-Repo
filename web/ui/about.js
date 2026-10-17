// About: the app's version and what it's built with, how a library is laid out,
// what's coming, the licences and the changelog. tools/build_desktop.py copies
// CHANGELOG.md and THIRD_PARTY.md next to the page and writes build-info.json,
// so this works offline.
import { html, useState, useEffect } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { ctx } from "./context.js";
import { CommandBar } from "./layout.js";

const OS = { windows: "Windows", linux: "Linux", macos: "macOS" };
const RELEASES = "https://github.com/DrFlGd/Game-Repo/releases";

export const TREE = `_library/                       the app's own files
  library.json                  the library's name and settings
  games/<platform>/<Game>/      one folder per game with more than its ROM
  games/<platform>/<SHA1>/      what Identify found for a file no DAT knows
  games/pc/<id>/media/          a PC game's artwork
    game.json                   its id, DAT match, ROM hashes and details
    media/                      scraped pictures and video: cover.png, screenshot.jpg…
    docs/                       its guides, manuals, maps and scans (docs.json: their
                                kinds and titles), scraped manuals among them
  docs/<platform>/  docs/General/
                                documents that go with no one game
  dats/<platform>/              the DAT files you imported, every version
  fixdats/  reports/            what's missing as DATs, and saved audit reports
  made-dats/                    DATs you made from your own sets
  layouts/                      export layouts of your own (JSON), for Export…
  journal/                      moves and renames, for Undo
BIOS/<system>/                  BIOS files, gathered from the BIOS files page
Nintendo - SNES/                platform folders, when you move ROMs in
  Chrono Trigger (USA).zip
Sony - PlayStation/
  Final Fantasy VII (USA)/      a folder for a game of several files
    … (Disc 1).chd
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
  ["Phase 5b", "Exports to your handhelds' SD cards (muOS, Knulli, ROCKNIX, ES-DE on Android or Linux) in layouts you can copy and change in a visual editor, and BIOS files audited, gathered and exported with the games"],
  ["Phase 6", "The Library: every game across the systems by its covers, in a grid, list or table, searched and filtered by its details and saved as a collection; a page for each game where its details are changed by hand and kept over scrapes"],
];
export const NEXT = [
  ["Phase 7", "Arcade sets (MAME and FBNeo)"],
  ["Phase 8", "More frontends (RetroArch, Pegasus, Playnite, LaunchBox), MobyGames, and a collection of magazines"],
  ["Phase 9", "Serving games to your other devices: Android, Linux and handhelds on your home network"],
];

/** Text with `code`, **bold**, *italic* and [links](https://…), as nodes (never as HTML). */
function inline(text) {
  const out = [];
  const re = /`([^`]+)`|\*\*(.+?)\*\*|\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)|\*([^*\s][^*]*)\*/g;
  let at = 0, m;
  while ((m = re.exec(text))) {
    if (m.index > at) out.push(text.slice(at, m.index));
    if (m[1] != null) out.push(html`<code>${m[1]}</code>`);
    else if (m[2] != null) out.push(html`<strong>${inline(m[2])}</strong>`);
    else if (m[3] != null) out.push(html`<a href=${m[4]} target="_blank" rel="noopener noreferrer">${inline(m[3])}</a>`);
    else out.push(html`<em>${m[5]}</em>`);
    at = re.lastIndex;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** The Markdown this repository writes (headings, paragraphs, lists, tables), as nodes. */
export function Markdown({ text }) {
  const blocks = [];
  let para = [], list = [], table = [];
  const flush = () => {
    if (para.length) blocks.push(html`<p>${inline(para.join(" "))}</p>`);
    if (list.length) blocks.push(html`<ul>${list.map((t) => html`<li>${inline(t)}</li>`)}</ul>`);
    if (table.length) {
      const cells = (row) => row.replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
      const [head, , ...rows] = table;
      blocks.push(html`<div class="about-table"><table><thead><tr>${cells(head).map((c) => html`<th>${inline(c)}</th>`)}</tr></thead>
        <tbody>${rows.map((r) => html`<tr>${cells(r).map((c) => html`<td>${inline(c)}</td>`)}</tr>`)}</tbody></table></div>`);
    }
    para = []; list = []; table = [];
  };
  for (const line of text.split(/\r?\n/)) {
    const h = /^(#{1,4})\s+(.*)$/.exec(line);
    if (h) { flush(); blocks.push(h[1].length <= 2 ? html`<h3>${inline(h[2])}</h3>` : html`<h4>${inline(h[2])}</h4>`); }
    else if (/^\s*\|/.test(line)) { if (para.length || list.length) { const t = table; flush(); table = t; } table.push(line.trim()); }
    else if (/^\s*[-*]\s+/.test(line)) { if (para.length || table.length) flush(); list.push(line.replace(/^\s*[-*]\s+/, "")); }
    else if (!line.trim() || /^-{3,}$/.test(line.trim())) flush();
    else if (list.length && /^\s+\S/.test(line)) list[list.length - 1] += " " + line.trim();
    else { if (list.length || table.length) flush(); para.push(line.trim()); }
  }
  flush();
  return html`<div class="about-md">${blocks}</div>`;
}

/** CHANGELOG.md's releases, newest first: [{title, version, body}], without the agents' notes. */
export function releases(text) {
  const parts = text.split(/^## /m).slice(1).map((p) => {
    const nl = p.indexOf("\n");
    const title = (nl < 0 ? p : p.slice(0, nl)).trim();
    return { title, version: /^\d+\.\d+\.\d+/.exec(title)?.[0] || null, body: nl < 0 ? "" : p.slice(nl + 1).trim() };
  });
  return parts.filter((p) => p.version || /^unreleased/i.test(p.title));
}

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
      <p><a href=${RELEASES} target="_blank" rel="noopener noreferrer">All releases on GitHub</a></p>
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
