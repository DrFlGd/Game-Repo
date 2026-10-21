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
  ["verbs", "Import, audit, organise, collect, export", "The five things the app does with your games, each on a page of its own. Import brings files into the library (Import). Audit checks what the library holds against the DATs and changes nothing (Audit, with BIOS files, Metadata and Documents under it). Organise changes files the library already holds: names, folders, formats (started from the Audit or the Library). Collect picks games for an export and moves no files (Collections). Export writes a collection to a device, a frontend or a folder."],
  ["folders-with-games", "Folders with games", "Folders of your own that hold ROMs and disc images, added on the Import page. They can be anywhere, on any drive; the app scans and watches them where they are, and changes them only when you organise."],
  ["import", "Import", "Bringing files into the library: games kept in a folder with games, games copied into one (named as their DAT says), PC games, DATs, BIOS files, documents and magazines. All of it is on the Import page (other pages' import buttons open it at their section, and files dropped on the window go there), every import has the same three ways in (copy, hardlinked on the same drive; always copy; move), and every import can be undone from Recent imports or History."],
  ["scan", "Scan", "Reading the folders with games: every file (and every file inside a ZIP, 7z, RAR or disc image) is hashed, so the app knows what it is whatever it's called. Files that haven't changed aren't read again."],
  ["dat", "DAT", "A list of games and the exact files each is made of (names, sizes and hashes), from No-Intro, Redump, TOSEC, libretro or another tool. The app reads Logiqx XML and ClrMamePro DATs, alone or zipped."],
  ["full-set", "Audit DAT", "A system's complete list of games: the DAT imported for that system, which your files are checked against. Collections are always parts of what's audited."],
  ["audit", "Audit", "Checking your files against each system's DAT: which games you have, have in part, or are missing, and which files are misnamed, bad dumps, unknown or duplicates. An audit changes nothing."],
  ["have", "Have, partly, missing", "How much of a game your folders hold: every file the DAT lists, some of them, or none. A file matches a DAT's file by its hashes, not its name."],
  ["fixdat", "List of what's missing (fixdat)", "A DAT of the games you're missing, for other tools to look for. Save a list of what's missing on the Audit page writes one."],
  ["one-per-title", "One per title (1G1R)", "Of a game's versions (regions, revisions), only the one first in your region order."],
  ["addons", "Updates and DLC", "Updates, DLC and patches for a game, from a DAT of their own (No-Intro's \"Updates and DLC\") or tagged so in its name. Each belongs to the game with the same title and region; Link to another game… on its page puts one right. They're audited with their own DAT, shown with their game, and go with it into collections and exports."],
  ["organise", "Organise", "Changing files the library already holds: copying, renaming or moving a system's games to their DAT names (Organise…), Rename to the DAT's names…, Set aside duplicates…, Convert… to another format or unpack."],
  ["change", "Change", "Anything that moves, copies, renames, converts or writes files: organising, importing, exporting and the rest. Each is shown as a preview first, nothing happens until you confirm it, every file written is read back and checked, and it can be undone from the message afterwards or from History."],
  ["set-aside", "Set aside", "What a change does instead of deleting: the file goes to a hidden .gamerepo-set-aside folder on the same drive, so Undo can bring it back. It's deleted only when you empty it (History)."],
  ["duplicates", "Duplicates", "More than one file for the same file of a DAT. Set aside duplicates… keeps the one organising would keep and sets the others aside."],
  ["collection", "Collection", "Games picked from the library for an export, made by rules (systems, regions, one per title, details), from folders, or from a DAT, and kept in the library. A collection can hold groups (say, a system's games and its translations), each made its own way. Collecting never copies files; exporting copies its games from your folders with games."],
  ["export", "Export", "Copying a collection's games (with their pictures, gamelist, manuals and BIOS files if you choose) for ES-DE (on a computer or a card), RetroArch, Pegasus, LaunchBox or Playnite, a muOS, Knulli or ROCKNIX handheld's card, or a plain folder, laid out as an export layout says. Each export is remembered: Update… brings it in step with the collection, Check reads it back."],
  ["layout", "Export layout", "Where an export puts each thing for a frontend or a device's firmware: the folder for each system, the gamelist, the pictures and their names. The built-in ones can't be changed; save a copy to make your own."],
  ["details", "Details", "What's known about a game: its title, year, genre, players, description and so on. Each source's answer is kept apart; the game shows one value for each field, from the first source in your order, unless you've chosen another source's value or written your own."],
  ["source", "Source", "Where details and pictures come from: ScreenScraper, IGDB, TheGamesDB and LaunchBox for ROM games, Steam, GOG and LaunchBox for PC games. Their keys are in Settings › Sources. (A collection's Source tab is what the collection was made from.)"],
  ["scrape", "Scrape", "Asking the sources about your games and keeping their details, pictures, video and manuals in each game's library folder. You choose what's asked for; Scrape again… asks once more."],
  ["identify", "Identify", "Choosing which game something is, by hand, from the sources' answers: for a PC game, or a game a scrape wasn't sure of. Its details and pictures come with it. For a PC game no source has, None of these (or No match on the import's list) keeps it matched to no source, under its own title."],
  ["picture-kinds", "Picture kinds and variants", "Each kind of picture (box front, screenshot, title screen, logo…) can come from several sources. One per source is kept; the one chosen is what the game shows and what an export takes."],
  ["game-library-folder", "A game's library folder", "Where a game's details, pictures and documents are kept: _library/games/<system>/<the DAT's name for it>/. It follows the game by its files' hashes, so renaming a ROM doesn't lose anything."],
  ["documents", "Documents", "Guides, FAQs, manuals, maps, posters and scans of your own, kept with their games (or by system) and searchable by their words."],
  ["magazines", "Magazines", "Issues of game magazines (PDF, CBZ or CBR), kept by magazine in the library's Magazines folder, each with its number, date and cover. A list of issues (a CSV or a DAT) says which ones you're missing."],
  ["bios", "BIOS files", "The firmware files emulators need, audited against libretro's list and imported into the library's BIOS folder. The app never downloads them or says where to get them."],
  ["arcade", "Arcade sets", "MAME's and FinalBurn Neo's games: a ZIP per machine named by its short name (sf2.zip), clones leaning on their parents, BIOS sets (neogeo.zip) and devices as ZIPs of their own, hard disks as CHDs in a folder named for the machine, and sounds in samples/. BIOS sets and devices aren't counted as games."],
  ["set-type", "Set type", "How an arcade system's ZIPs are made up: non-merged (every game's ZIP holds all it needs but its BIOS's), split (a clone's ZIP holds only what its parent's doesn't) or merged (a parent's ZIP holds its clones' too). Chosen on the Audit page or in Settings › Systems; Organise builds ZIPs that way, and an export is always non-merged."],
  ["playable", "Playable", "An arcade game you have, with the BIOS set and devices it needs (and, in a split DAT, its parent). A game that isn't says what it needs; missing sounds are listed but don't stop it."],
  ["pc-game", "PC game", "A game for Windows, kept in its own folder under PC/<Title> (<Year>)/ with a game.json saying what it is and how it starts: one installer (or archive) and one unpacked copy, with its patches, DLC and extras. Installers are never run: Unpack opens GOG's and zipped ones."],
  ["ribbons", "Ribbons", "The panels beside a page. On the left, next to the menu, a page's own list: the collections, the audited systems, the magazines ([ folds it to a strip). On the right of a game's page, its pictures (] folds it). Drag a ribbon's edge to make it wider."],
  ["inbox", "Inbox", "A folder you drop PC installers, archives, disc images and game folders into; Import… reads it and imports them into the PC folder."],
];

/** Each page in the menu, in its order: [route, title, what it's for, its main actions]. */
export const PAGES = [
  ["home", "Home", "Where your library stands: how complete each system is against its DAT, and what to do next.", [
    "Scan, at the top, reads the folders with games for what's new or changed.",
    "Next: files that need review, games whose details need a look, systems not scraped yet and missing BIOS files, each a link to the page it's done on.",
    "Systems: how complete each one is against its DAT, with Import DATs… (and Get starter DATs… while you have none) and Open the audit.",
    "Folders with games: how many files were found and when they were last scanned, Add a folder… while there are none, folders not found on this computer (Point it elsewhere…), and files no DAT takes yet. Before any library is open, Home offers Open a library….",
  ]],
  ["library", "Library", "Every game in the library, across systems, by its pictures and details.", [
    "Filter by system, genre, decade, players and more, search, and sort or group the list.",
    "Open a game's page: its details from each source (Use this picks one), its pictures by kind, its files and documents, its updates and DLC; edit a detail by hand, or drop a file on it. The Media ribbon on its right (] opens it) shows every picture from each source, larger, with Use this.",
    "Save as a collection… keeps the filters as a collection's rules.",
  ]],
  ["pc", "PC games", "Your PC games, each in its own folder with what it is and how it starts (under the Library).", [
    "Import… from the inbox, or Import files… and Import a game's folder… (each opens Import › PC games); Unpack GOG installers and archives; choose the program it starts with.",
    "On the import's list, No match keeps a game no source has (one a search took for another) as a game of its own, under the title you give it. An installer or unpacked copy that goes into a game you have replaces its own (Replaces its installer): the old one is set aside, and Undo brings it back. Patches, DLC and extras add up.",
    "Remove from the library… (in its panel, or its page's ⋯ menu) sets aside its folder and its details, pictures and documents, with the preview first and Undo after. A Steam shortcut Add to Steam… made stays until Add to Steam… is run again.",
    "Identify… finds the game on Steam, GOG or LaunchBox; each source's details and pictures are kept apart, as for every other game.",
    "Open its page: the same page as any game, with a PC game section for its program, Unpack and its folder; Add to Steam… makes the games Steam shortcuts with their artwork.",
  ]],
  ["magazines", "Magazines", "Your magazines, each issue by its cover, readable here (under the Library).", [
    "Import magazines… (Import › Magazines) from files or a folder: each file's magazine, number and date are read from its name for you to check, then imported (copied, linked or moved, with Undo).",
    "Open an issue to read it (a PDF, or a comic book a page at a time); give it a cover by dropping a picture on it; Import a list… to see what's missing, and save that as a list.",
    "An export can carry the magazines into a Magazines folder (What it carries in the Export dialog).",
  ]],
  ["import", "Import", "Everything that comes into the library, in one place.", [
    "Games into the library: From a folder… reads folders (every system's games, or one system's), matches their files against your DATs and copies or moves the games the library lacks into each system's own folder with games, named as their DAT says. It asks where a system's games go when it has no folder yet, and which system files no DAT knows belong to. One game… is for a single ROM or disc.",
    "PC games, BIOS files, documents and magazines, each with its own buttons. Files dropped on the window come here, and you're asked what they are.",
    "Recent imports: what came in lately, a day at a time, with Steps for what each did and Undo for the newest.",
    "Folders with games: games you keep where they are, scanned and audited there. DAT files: Import DATs…, Get starter DATs… and Make a DAT from a folder…, and the DATs you have.",
  ]],
  ["audit", "Audit", "Your games checked against each system's DAT: what you have, have in part, or are missing. Nothing changes.", [
    "The systems are in the ribbon on the left, with their counts; All systems is a table of every system's counts, each a link to that list.",
    "Games (have, partly there, missing) and Files (named right, misnamed, bad dumps, unknown, duplicates), each a table: a game's name opens its page, and a right-click has Convert… and Show in folder.",
    "A games list can be narrowed to a region and sorted by name, region or year; reports, and Save a list of what's missing.",
    "Organise… sorts, renames and packs a system's games; Rename to the DAT's names…, Set aside duplicates… and Convert… (to CHD, RVZ, CSO, ZIP, or unpacked) are changes too.",
    "For an arcade system (MAME, FinalBurn Neo): how many games are playable, what each needs, its BIOS sets and devices on a tab of their own, and its set type.",
    "Unknown: each file no DAT knows with the game it's likely a variant of (a translation, a hack, a prototype) and its tags, the parentheticals at the end of its name. Add puts it in the system's custom DAT as that game's variant; Edit… names it, gives it tags from the drop-down (or New tag…) and chooses its original, or none for a game of its own. Until then it stays out of the Library.",
    "DATs…: the main DAT, whose counts are the totals, and the DATs the system is audited with too (a translation group's, say): the files they know are off Unknown, their games are in the Library, and each has its counts under the totals with Show its lists.",
    "Variants: the system's custom DAT, each variant with its tags and its original; Rename (its files to their names), Remove and Undo. In the Library a variant shows under its own name with its original's details and pictures, and the Tag filter finds them.",
  ]],
  ["bios", "BIOS files", "Each system's BIOS files: which you have, which are missing, and where they are (under the Audit).", [
    "Every system the BIOS DATs list, the ones you have games for first; search for a system or a file.",
    "Get libretro's list… or Import a BIOS DAT…, then Import BIOS files… (Import › BIOS files) copies or moves the ones you have into the library's BIOS folder; an export can take them with a collection's games.",
  ]],
  ["metadata", "Metadata", "Each game's details, pictures, video and manual, and how much of it you have (under the Audit).", [
    "The systems are in the ribbon on the left. Scrape… a system, a collection or a game, choosing the sources; Change what's fetched… chooses the details and kinds of pictures.",
    "Delete metadata… or Scrape again… for a system or a game (with a preview and Undo); Import documents… from a folder.",
  ]],
  ["documents", "Documents", "Your guides, manuals and other documents, searchable and readable here (under the Audit).", [
    "Search their words, read them beside the list, filter by kind; Import documents… pairs files with games for you to check first.",
  ]],
  ["collections", "Collections", "Games picked from the library, to export or save as DATs. Collecting moves no files.", [
    "The collections are in the ribbon on the left; the one chosen shows its games as the Library does (grid, list or table), with its Source (or Rules), Size, Exported to and History.",
    "New collection…: by rules, from folders (matched by hash, so the files' names don't matter), or from a DAT read where it is; a review shows the games found before it's made. Add a group… adds a group made its own way (a system's translations, say), shown as a section of its own.",
    "Pick games (Ctrl or Shift for more, Ctrl+A for all) and Export these…, or Export… the whole collection; Update… and Check an export from Exported to. An ES-DE export can put each group in a subfolder or as a system of its own, and a Knulli or ROCKNIX export in a subfolder; every export makes a custom collection or playlist of each group where the frontend or the handheld has them (ES-DE, Knulli, RetroArch, Pegasus).",
    "Leave a game out by hand, or add one; set a size limit for a card; Save as DATs… for RomVault, clrmamepro or igir.",
    "With their updates and DLC (a rule, on by default) takes the games' updates and DLC you have along; History lists each change to what's in it, a day at a time, and Undo puts one back.",
  ]],
  ["layouts", "Export layouts", "Where an export puts things for ES-DE, RetroArch, Pegasus, LaunchBox, Playnite, a device's firmware or a folder.", [
    "See a layout as a folder tree with a collection's games, change it, and Save as new…; edit its JSON, or import one.",
  ]],
  ["history", "History", "Every change the app has made to your files, newest first, a day at a time.", [
    "Steps lists what a change did; Undo the newest; finish a change that was interrupted; empty what changes set aside, to free the space.",
  ]],
  ["settings", "Settings", "How the app works for this library and this computer.", [
    "Sources and their keys, tools (dolphin-tool for RVZ, innoextract for GOG installers; CHD, CSO and ZSO the app writes itself), how folders are scanned, systems added by hand, naming and preferred formats.",
    "The index, metadata sizes, the theme and the skin.",
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
