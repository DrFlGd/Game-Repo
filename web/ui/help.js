// Help (docs/PLAN.md, D54): the words the app uses, each said once and used the same way
// on every page, then what each page in the menu is for and what its main actions do.
// The page test checks every page in the menu has a section here.
import { html, useState } from "../lib/html.js";
import { CommandBar } from "./layout.js";
import { routeHash } from "./context.js";
import { Icon } from "./icons.js";

/** The words, in the order they build on each other: [id, word, what it means]. */
export const TERMS = [
  ["library", "Library", "The folder Game Repo keeps everything in. Its _library folder holds your DATs, collections, exports, the history of changes and each game's details, pictures and documents, as ordinary files that make sense without the app. Move it, or open it on another computer, and it's all there."],
  ["folders-with-games", "Folders with games", "The folders you've added that hold your ROMs and disc images (Add a folder… on Home, or Settings › Folders with games). They can be anywhere, on any drive; the app reads and watches them, and changes them only when you run a change."],
  ["scan", "Scan", "Reading the folders with games: every file (and every file inside a ZIP, 7z, RAR or disc image) is hashed, so the app knows what it is whatever it's called. Files that haven't changed aren't read again."],
  ["dat", "DAT", "A list of games and the exact files each is made of (names, sizes and hashes), from No-Intro, Redump, TOSEC, libretro or another tool. The app reads Logiqx XML and ClrMamePro DATs, alone or zipped."],
  ["full-set", "Full set (audit DAT)", "A system's complete list of games: the DAT imported on the Audit page for that system, which your files are checked against. Collections are always parts of the full sets."],
  ["have", "Have, partly, missing", "How much of a game your folders hold: every file the DAT lists, some of them, or none. A file matches a DAT's file by its hashes, not its name."],
  ["one-per-title", "One game per title (1G1R)", "Of a game's versions (regions, revisions), only the one first in your region order."],
  ["change", "Change", "Anything that moves, copies, renames, converts or writes files: Organise, Rename these, Convert, Export, Import and the rest. Each is shown as a preview first, nothing happens until you press Run, every file written is read back and checked, and it can be undone from the message afterwards or from Settings › History."],
  ["set-aside", "Set aside", "What a change does instead of deleting: the file goes to a hidden .gamerepo-set-aside folder on the same drive, so Undo can bring it back. It's deleted only when you empty it (Settings › History)."],
  ["duplicates", "Duplicates", "More than one file for the same file of a DAT. Set aside duplicates keeps the one organising would keep and sets the others aside."],
  ["collection", "Collection", "A part of your full sets, made by rules (systems, regions, one per title, details), from folders you already keep, or from a DAT, and kept in the library. Exporting it copies its games from your folders with games."],
  ["export", "Export", "Copying a collection's games (with their pictures, gamelist, manuals and BIOS files if you choose) to ES-DE, a handheld's card or a folder, laid out as an export layout says. Each export is remembered: Update… brings it in step with the collection, Check reads it back."],
  ["layout", "Export layout", "Where an export puts each thing for a frontend or a device's firmware: the folder for each system, the gamelist, the pictures and their names. The built-in ones can't be changed; save a copy to make your own."],
  ["details", "Details", "What's known about a game: its title, year, genre, players, description and so on. Each source's answer is kept apart; the game shows one value for each field, from the first source in your order, unless you've chosen another source's value or written your own."],
  ["source", "Source", "Where details and pictures come from: ScreenScraper, IGDB, TheGamesDB and LaunchBox for ROM games, Steam, GOG and LaunchBox for PC games. Their keys are in Settings › Sources."],
  ["scrape", "Scrape", "Asking the sources about your games and keeping their details, pictures, video and manuals in each game's library folder. You choose what's asked for; Scrape again… asks once more."],
  ["picture-kinds", "Picture kinds and variants", "Each kind of picture (box, screenshot, title screen, logo…) can come from several sources. One per source is kept; the one chosen is what the game shows and what an export takes."],
  ["game-library-folder", "A game's library folder", "Where a game's details, pictures and documents are kept: _library/games/<system>/<the DAT's name for it>/. It follows the game by its files' hashes, so renaming a ROM doesn't lose anything."],
  ["documents", "Documents", "Guides, FAQs, manuals, maps, posters and scans of your own, kept with their games (or by system) and searchable by their words."],
  ["bios", "BIOS files", "The firmware files emulators need, audited against libretro's list and kept in the library's BIOS folder. The app never downloads them or says where to get them."],
  ["arcade", "Arcade sets", "MAME's and FinalBurn Neo's games: a ZIP per machine named by its short name (sf2.zip), clones leaning on their parents, BIOS sets (neogeo.zip) and devices as ZIPs of their own, hard disks as CHDs in a folder named for the machine, and sounds in samples/. BIOS sets and devices aren't counted as games."],
  ["set-type", "Set type", "How an arcade system's ZIPs are made up: non-merged (every game's ZIP holds all it needs but its BIOS's), split (a clone's ZIP holds only what its parent's doesn't) or merged (a parent's ZIP holds its clones' too). Chosen on the Audit page or in Settings › Systems; Organise builds ZIPs that way, and an export is always non-merged."],
  ["playable", "Playable", "An arcade game you have, with the BIOS set and devices it needs (and, in a split DAT, its parent). A game that isn't says what it needs; missing sounds are listed but don't stop it."],
  ["pc-game", "PC game", "A game for Windows, kept in its own folder under PC/<Title> (<Year>)/ with a game.json saying what it is and how it starts. Installers are never run: Unpack opens GOG's and zipped ones."],
  ["inbox", "Inbox", "A folder you drop PC installers, archives, disc images and game folders into; Import… reads it and brings them into the PC folder."],
];

/** Each page in the menu: [route, title, what it's for, its main actions]. */
export const PAGES = [
  ["home", "Home", "Where your library stands: how complete each system is against its full set, and what to do next.", [
    "Open a library…, Add a folder… with games, Import DATs… or Get starter DATs…: the first steps, in order.",
  ]],
  ["library", "Library", "Every game in the library, across systems, by its pictures and details.", [
    "Filter by system, genre, decade, players and more, search, and sort or group the list.",
    "Open a game's page: its details from each source (Use this picks one), its pictures by kind, its files and documents; edit a detail by hand, or drop a file on it.",
    "Save as a collection… keeps the filters as a collection's rules.",
  ]],
  ["audit", "Audit", "Your folders with games checked against each system's full set: what you have, have in part, or are missing.", [
    "Import DATs… or Get starter DATs… for each system; your folders with games are scanned by themselves.",
    "The lists of games you have, have in part and are missing, with reports and Export fixdat for what's missing.",
    "Organise… sorts, renames and packs a system's games; Rename these, Set aside duplicates, Convert… (to CHD, RVZ, CSO, ZIP) and Extract are changes too.",
    "Make a DAT from a set… you trust; Add a game… for one ROM or disc from anywhere.",
    "For an arcade system (MAME, FinalBurn Neo): how many games are playable, what each needs, its BIOS sets and devices on a tab of their own, and its set type.",
  ]],
  ["bios", "BIOS files", "Each system's BIOS files: which you have, which are missing, and where they are.", [
    "Get libretro's list… or Import a BIOS DAT…, then Gather… copies the ones you have into the library's BIOS folder; an export can take them with a collection's games.",
  ]],
  ["collections", "Collections", "Parts of your full sets, to export or save as DATs.", [
    "New collection…: by rules, from folders (matched by hash, so the files' names don't matter), or from a DAT read where it is; a review shows the games found before it's made.",
    "Tick games or whole systems and Export these…, or Export… the whole collection; Update… and Check an export from its tab.",
    "Leave a game out by hand, or add one; set a size limit for a card; Save as DATs… for RomVault, clrmamepro or igir.",
  ]],
  ["layouts", "Export layouts", "Where an export puts things for ES-DE, a device's firmware or a folder.", [
    "See a layout as a folder tree with a collection's games, change it, and Save as new…; edit its JSON, or bring one in.",
  ]],
  ["pc", "PC games", "Your PC games, each in its own folder with what it is and how it starts.", [
    "Import… from the inbox, or Import files… and Import a game's folder…; Unpack GOG installers and archives; choose the program it starts with.",
    "Identify finds the game on Steam, GOG or LaunchBox; Add to Steam makes it a Steam shortcut with its artwork.",
  ]],
  ["metadata", "Metadata", "Each game's details, pictures, video and manual, and how much of it you have.", [
    "Scrape… a system, a collection or a game, choosing the sources and the kinds of pictures.",
    "Delete metadata… or Scrape again… for a system or a game (with a preview and Undo); Import documents… from a folder.",
  ]],
  ["documents", "Documents", "Your guides, manuals and other documents, searchable and readable here.", [
    "Search their words, read them beside the list, filter by kind; Import documents… pairs files with games for you to check first.",
  ]],
  ["settings", "Settings", "How the app works for this library and this computer.", [
    "Sources and their keys, tools (chdman, dolphin-tool, innoextract), systems added by hand, naming and preferred formats.",
    "History: every change, with Undo, and emptying what was set aside; the index, metadata sizes, the theme and the skin.",
  ]],
  ["about", "About", "The app's version, its licence and where its data comes from.", []],
];

export function Help() {
  const [q, setQ] = useState("");
  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const hit = (...texts) => !words.length || words.every((w) => texts.join(" ").toLowerCase().includes(w));
  const terms = TERMS.filter(([, word, what]) => hit(word, what));
  const pages = PAGES.filter(([, title, what, does]) => hit(title, what, ...does));
  return html`<${CommandBar} id="help-head" title="Help"
      primary=${html`<input type="search" id="help-search" class="compact" placeholder="Search the help" aria-label="Search the help" value=${q} onInput=${(e) => setQ(e.target.value)} />`} />
    <div class="pages help-page" id="help-page">
      <p class="page-lead">${Icon.help(18)} The words Game Repo uses, and what each page is for. Every change to your files is shown first and can be undone.</p>
      <section class="ls-section" id="help-terms">
        <h2>Words used in the app</h2>
        ${terms.length ? html`<dl class="help-terms">${terms.map(([id, word, what]) => html`<div key=${id} id=${`term-${id}`} data-term=${id}><dt>${word}</dt><dd>${what}</dd></div>`)}</dl>`
          : html`<p class="muted">No word matches.</p>`}
      </section>
      <section class="ls-section" id="help-pages">
        <h2>The pages</h2>
        ${pages.map(([route, title, what, does]) => html`<div class="help-section" key=${route} id=${`help-${route}`} data-route=${route}>
          <h3><a href=${routeHash(route)}>${title}</a></h3>
          <p>${what}</p>
          ${does.length ? html`<ul>${does.map((d, i) => html`<li key=${i}>${d}</li>`)}</ul>` : null}
        </div>`)}
        ${!pages.length ? html`<p class="muted">No page matches.</p>` : null}
      </section>
    </div>`;
}
