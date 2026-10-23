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
  ["import", "Import", "Bringing files into the library: games kept in a folder with games, games copied into one (named as their DAT says), PC games, DATs, BIOS files, documents, magazines and videos. All of it is on the Import page (other pages' import buttons open it at their section, and files dropped on the window go there), every import has the same three ways in (copy, hardlinked on the same drive; always copy; move), and every import can be undone from Recent imports or History."],
  ["scan", "Scan", "Reading the folders with games: every file (and every file inside a ZIP, 7z, RAR or disc image) is hashed, so the app knows what it is whatever it's called. Files that haven't changed aren't read again."],
  ["system", "System", "A console, handheld, computer or arcade family: Game Boy, PlayStation, Arcade (MAME). Settings › Systems adds one the app doesn't know."],
  ["dat", "DAT", "A list of games and the exact files each is made of (names, sizes and hashes), from No-Intro, Redump, TOSEC, libretro or another tool. The app reads Logiqx XML and ClrMamePro DATs, alone or zipped."],
  ["game", "Game", "One game a DAT lists, and the files that make it up. A ROM, disc image or archive is a file of a game."],
  ["main-dat", "Main DAT", "The DAT whose counts are a system's totals on the Audit page, when the system has more than one. Chosen in Audit › DATs…, or with Make it the main DAT in Import › DAT files."],
  ["include-in-audit", "Include in Audit", "Ticked in Audit › DATs… for another DAT of a system (a translation group's, say): Audit checks the files against it too, the files it knows are off Unknown, and its games are in the Library and collections beside the main DAT's."],
  ["full-set", "Audited games", "The games of the DATs Audit checks a system against: its main DAT, those included in Audit and its custom DAT. Collections are made from them."],
  ["audit", "Audit", "Checking your files against each system's DATs: which games you have, have in part, or are missing, and which files are misnamed, bad dumps, unknown or duplicates. An audit changes nothing."],
  ["have", "Have, partly, missing", "How much of a game your folders hold: every file the DAT lists, some of them, or none. A file matches a DAT's file by its hashes, not its name."],
  ["unknown", "Unknown files", "Files none of their system's DATs knows, listed on Audit › Unknown, each with the game it's likely a variant of. Until one is added to the custom DAT, it stays out of the Library."],
  ["need-review", "Need review", "Files Game Repo couldn't be sure of, waiting for you to say: misnamed files, bad dumps and files no DAT knows. Home's Next links to them on the Audit page."],
  ["variants", "Custom DAT and variants", "A variant is a translation, hack, prototype or other version that the main DAT doesn't list. Game Repo keeps a system's variants in its custom DAT, each with the game it's a version of (its original), so they're audited, shown in the Library and collected like any other game."],
  ["fixdat", "List of what's missing (fixdat)", "A DAT of the games you're missing, for other tools to look for. Save a list of what's missing on the Audit page writes one."],
  ["one-per-title", "One per title (1G1R)", "Of a game's versions (regions, revisions), only the one first in your region order."],
  ["addons", "Updates and DLC", "Updates, DLC and patches for a game, from a DAT of their own (No-Intro's \"Updates and DLC\") or named so: an (Update), [UPD], (DLC) or (Patch) tag, or a Switch, 3DS or Wii U title ID. Each belongs to the game with its title ID, else the one with the same title and region, else the game whose title its name starts with; Link to another game… puts one right. Files no DAT lists are suggested for their game on Audit › Unknown. They're audited with their own DAT, shown with their game (or as games of their own: untick Updates and DLC with their game in the Library), and go with it into collections and exports."],
  ["organise", "Organise", "Changing files the library already holds: copying, renaming or moving a system's games to their DAT names (Organise…), Rename to the DAT's names…, Set aside duplicates…, Convert… to another format or unpack."],
  ["change", "Change", "Anything that moves, copies, renames, converts or writes files: organising, importing, exporting and the rest. Each is shown as a preview first, nothing happens until you confirm it, every file written is read back and checked, and it can be undone from the message afterwards or from History."],
  ["set-aside", "Set aside", "What a change does instead of deleting: the file goes to a hidden .gamerepo-set-aside folder on the same drive, so Undo can bring it back. It's deleted only when you empty it, or delete it on its own (History › Set aside)."],
  ["undo", "Undo", "Putting a change back, from the message after it or from History: the newest change first."],
  ["duplicates", "Duplicates", "More than one file for the same file of a DAT. Set aside duplicates… keeps the one organising would keep and sets the others aside."],
  ["collection", "Collection", "Games picked from the library for an export, made by rules (systems, regions, one per title, details), from folders, or from a DAT, and kept in the library. A collection can hold groups (say, a system's games and its translations), each made its own way. Collecting never copies files; exporting copies its games from your folders with games."],
  ["group", "Group", "A part of a collection with its own way of choosing games: say, a system's translations beside its games. An export can put each group in a subfolder or make it a system of its own."],
  ["waiting", "Waiting", "A file imported into a collection that no DAT knows yet. It waits on Audit › Unknown and joins the collection once a DAT knows it, such as the system's custom DAT."],
  ["export", "Export", "Copying a collection's games (with their pictures, gamelist, manuals and BIOS files if you choose) for ES-DE (on a computer or a card), RetroArch, Pegasus, LaunchBox or Playnite, a muOS, Knulli or ROCKNIX handheld's card, or a plain folder, laid out as an export layout says. Each export is remembered: Update… brings it in step with the collection, Check reads it back."],
  ["layout", "Export layout", "Where an export puts each thing for a frontend or a device's firmware: the folder for each system, the gamelist, the pictures and their names. The built-in ones can't be changed; save a copy to make your own."],
  ["details", "Details", "What's known about a game: its title, year, genre, players, description and so on. Each source's answer is kept apart; the game shows one value for each field, from the first source in your order, unless you've chosen another source's value or written your own."],
  ["source", "Source", "Where details and pictures come from: ScreenScraper, IGDB, TheGamesDB and LaunchBox for every system, and Steam and GOG for PC games too. Their keys are in Settings › Sources. (A collection's Source tab is what the collection was made from.)"],
  ["scrape", "Scrape", "Asking the sources about your games and keeping their details, pictures, video and manuals in each game's library folder. You choose what's asked for; Scrape again… asks once more."],
  ["identify", "Identify", "Choosing which game something is, by hand, from the sources' answers: for a PC game, or a game a scrape wasn't sure of. Its details and pictures come with it. For a PC game no source has, None of these (or No match on the import's list) keeps it matched to no source, under its own title."],
  ["picture-kinds", "Pictures and picture kinds", "Box fronts, screenshots, logos and the rest. Each kind of picture (box front, screenshot, title screen, logo…) can come from several sources; one from each source can be kept, and the one chosen is what the game shows and what an export takes."],
  ["game-library-folder", "A game's library folder", "Where a game's details, pictures and documents are kept: _library/games/<system>/<the DAT's name for it>/. It follows the game by its files' hashes, so renaming a ROM doesn't lose anything."],
  ["documents", "Documents", "Guides, FAQs, manuals, maps, posters and scans of your own, kept with their games (or by system) and searchable by their words."],
  ["magazines", "Magazines", "Issues of game magazines (PDF, CBZ or CBR), kept by magazine in the library's Magazines folder, each with its number, date and cover. A list of issues (a CSV or a DAT) says which ones you're missing."],
  ["videos", "Videos", "Walkthroughs, reviews, retrospectives and other videos about your games: files kept by kind in the library's Videos folder, or links to videos on the web (opened in your browser). Each lists the games it's about, and a game's page lists its videos."],
  ["backlog", "Backlog", "The games you mean to play, in the order you'll play them, beside the ones you've completed (under Collections). One to a library, kept in _library/backlog.json; it takes PC games too."],
  ["text-recognition", "Text recognition", "Reading the words in a picture of a page: a scanned magazine's, or a comic book's. Game Repo uses Windows' own (in the languages Windows has), or Tesseract when it's found (Settings › Tools); a PDF's own text needs none. What it reads is kept with the issue, searched on the Documents page, and looked through for the titles of your games."],
  ["bios", "BIOS files", "The firmware files emulators need, audited against libretro's list and imported into the library's BIOS folder. The app never downloads them or says where to get them."],
  ["arcade", "Arcade sets", "MAME's and FinalBurn Neo's games: a ZIP per machine named by its short name (sf2.zip), clones leaning on their parents, BIOS sets (neogeo.zip) and devices as ZIPs of their own, hard disks as CHDs in a folder named for the machine, and sounds in samples/. BIOS sets and devices aren't counted as games."],
  ["set-type", "Set type", "How an arcade system's ZIPs are made up: non-merged (every game's ZIP holds all it needs but its BIOS's), split (a clone's ZIP holds only what its parent's doesn't) or merged (a parent's ZIP holds its clones' too). Chosen on the Audit page or in Settings › Systems; Organise builds ZIPs that way, and an export is always non-merged."],
  ["playable", "Playable", "An arcade game you have, with the BIOS set and devices it needs (and, in a split DAT, its parent). A game that isn't says what it needs; missing sounds are listed but don't stop it."],
  ["pc-game", "PC game", "A game for Windows, kept in its own folder under PC/<Title> (<Year>)/ with a game.json saying what it is and how it starts: one installer (or archive) and one unpacked copy, with its patches, DLC and extras. Installers are never run: Unpack opens GOG's and zipped ones."],
  ["ribbons", "Ribbons", "The panels beside a page. On the left, next to the menu, a page's own list: the collections, the audited systems ([ folds it to a strip). On the right of a game's page, its pictures (] folds it). Drag a ribbon's edge to make it wider."],
  ["inbox", "Inbox", "A folder you drop PC installers, archives, disc images and game folders into; Import… reads it and imports them into the PC folder."],
  ["device", "Device and pairing", "A handheld, phone or computer that Game Repo Go runs on. It's paired with Game Repo Server once, with the six-digit code Pair a device… shows in Settings › Serve to devices; then it browses the library and downloads games into ES-DE's folders. The games waiting to download are its queue."],
];

/** Each page in the menu, in its order: [route, title, what it's for, its main actions]. */
export const PAGES = [
  ["home", "Home", "Where your library stands: how complete each system is against its DAT, and what to do next.", [
    "Scan, at the top, reads the folders with games for what's new or changed.",
    "Next: files that need review, games whose details need a look, systems not scraped yet and missing BIOS files, each a link to the page it's done on.",
    "Systems: how complete each one is against its DAT, with Import DATs… (and Get starter DATs… while you have none), Add a system… and Open the audit. A system shows once it has a DAT.",
    "Folders with games: how many files were found and when they were last scanned, Add a folder… while there are none, folders not found on this computer (Point it elsewhere…), and files no DAT takes yet. Before any library is open, Open a library… is all Home offers.",
  ]],
  ["library", "Library", "Every game in the library, across systems, by its pictures and details.", [
    "Filter by system, genre, decade, players and more, search, and sort or group the list. The Library keeps where you were (its filters, view and order) while you're on other pages.",
    "Open a game's page (Open, a double click or Enter): ‹ Previous and Next › go through the list it was opened from, the Library's or a collection's; Back comes back to that game in the list. A game's page shows its details from each source (Use this picks one), its pictures by kind, its files and documents, its updates and DLC; edit a detail by hand, or drop a file on it. The ribbon on its right (] opens it) shows every picture from each source, larger, with Use this.",
    "Save as a collection… keeps the filters as a collection's rules.",
  ]],
  ["pc", "PC games", "Your PC games, each in its own folder with what it is and how it starts (under the Library).", [
    "Laid out as the Library is: filters down the left (year, genre, developer, publisher, store, players, what it has, whether its program is set), Grid, List or Table, Sort and Group by; the one picked shows in the panel on the right, a double click opens its page, and a right click has its menu.",
    "Import… from the inbox, or Import files… and Import a game's folder… (each opens Import › PC games); Unpack GOG installers and archives; choose the program it starts with.",
    "On the import's list, No match keeps a game no source has (one a search took for another) as a game of its own, under the title you give it. An installer or unpacked copy that goes into a game you have replaces its own (Replaces its installer): the old one is set aside, and Undo brings it back. Patches, DLC and extras add up.",
    "Remove from the library… (in its panel, or its page's ⋯ menu) sets aside its folder and its details, pictures and documents, with the preview first and Undo after. A Steam shortcut Add to Steam… made stays until Add to Steam… is run again.",
    "Identify… finds the game at the sources (ScreenScraper, Steam, GOG, IGDB, TheGamesDB, LaunchBox), every one in the order Settings › Sources gives, or just the one picked beside the year; each source's details and pictures are kept apart, as for every other game.",
    "Find details… (in the ⋯ menu) asks the sources about every PC game not matched to one yet, as an import does: a game a source has under the same title (or another of its titles) takes its details and pictures; the rest are listed with Identify… beside each. DOS and Windows 3.x games are searched as well as Windows ones.",
    "Open its page: the same page as any game, with a PC game section for its program, Unpack and its folder; Add to Steam… makes the games Steam shortcuts with their artwork.",
  ]],
  ["magazines", "Magazines", "Your magazines, each issue by its cover, laid out as the Library is and readable here (under the Library).", [
    "Every issue is a tile: filter by magazine, publisher, tag, year, system, whether its text is read and what it has; group by magazine, publisher, tag, year or decade; Grid, List or Table. A magazine's heading has a menu: Rename…, Publisher…, Read its text, Save a list of what's missing, Show its folder.",
    "Pick issues (Ctrl or Shift and a click) to Move to another magazine… (a change with Undo) or add and remove a tag; one picked shows in the panel, where its number, date, tags and games are changed.",
    "Import magazines… (Import › Magazines) from files or a folder: each file's magazine, number and date are read from its name for you to check, then imported (copied, linked or moved, with Undo).",
    "Open an issue to read it (a PDF, or a comic book a page at a time); give it a cover by dropping a picture on it; Lists… (in ⋯) imports a list of a magazine's issues: Missing issues too shows those you don't have as faded tiles, and Save a list of what's missing saves them.",
    "Read their text reads each issue page by page (its own text, or by text recognition), once for each file: the Documents page then finds an issue by its words, and the viewer's Text shows a page's text with them marked.",
    "Games, on an issue, lists the games whose titles are in its text, with their pages: ticked when sure, and listed on their game's page (In magazines). Tick or untick them, or add one by its title and page.",
    "An export can carry the magazines into a Magazines folder (What it carries in the Export dialog).",
  ]],
  ["videos", "Videos", "Videos about your games: walkthroughs, reviews, retrospectives and others (under the Library).", [
    "Import videos… (Import › Videos) from files or a folder: each one's title, kind and date are read from its name, and the games whose titles are in its title found, for you to check; then they're imported (copied, linked or moved, with Undo) into Videos/, a folder for each kind.",
    "Add a link… keeps a video on the web (YouTube and the like) by its address; it opens in your browser.",
    "Videos are laid out as the Library is: filter by kind, series, creator, tag, system, game or year; group and sort them; Grid, List or Table. Watch one here with a double click (a file the app can't play opens in its own player).",
    "Pick videos (Ctrl or Shift and a click) to set their series, creator or kind, add or remove a tag, or add a game, together; one picked shows in the panel, where its title, kind, series, creator, date, tags, games and picture are changed. Remove… sets a file aside (History can undo it).",
    "A game's page lists its videos (Videos). Exports and devices don't take videos.",
  ]],
  ["import", "Import", "Everything that comes into the library, in one place.", [
    "Games into the library: From a folder… reads folders (every system's games, or one system's), matches their files against your DATs and copies or moves the games the library lacks into each system's own folder with games, named as their DAT says. It asks where a system's games go when it has no folder yet, and which system files no DAT knows belong to. Also put these games in a collection makes a collection (or a new group of one) of every game read, the library's own too; files no DAT knows wait on it until you add them to a DAT on Audit › Unknown. One game… is for a single ROM or disc.",
    "PC games, BIOS files, documents, magazines and videos, each with its own buttons. Files dropped on the window come here, and you're asked what they are.",
    "Recent imports: what came in lately, a day at a time, with Steps for what each did and Undo for the newest.",
    "Folders with games: games you keep where they are, scanned and audited there. DAT files: Import DATs…, Get starter DATs…, Get Redump's DATs… (Redump's own, every track of each disc, for the disc systems), Add a system… and Make a DAT from a folder…, and the DATs you have.",
  ]],
  ["audit", "Audit", "Your games checked against each system's DATs: what you have, have in part, or are missing. Nothing changes.", [
    "The systems are in the ribbon on the left, with their counts; All systems is a table of every system's counts, each a link to that list. A system shows once it has a DAT: Add a system… (the + in the ribbon, or ⋯) gets one for a system without, PlayStation 3, 4 and 5, Xbox 360 or Switch among them, or for one Game Repo doesn't know. Its DAT is downloaded (libretro's copy, or Redump's own for a disc system), imported from a file you have, or made from a folder of its games.",
    "Games (have, partly there, missing) and Files (named right, misnamed, bad dumps, unknown, duplicates), each a table: a game's name opens its page, and a right-click has Convert… and Show in folder.",
    "A games list can be narrowed to a region and sorted by name, region or year; reports, and Save a list of what's missing.",
    "Organise… sorts, renames and packs a system's games; Rename to the DAT's names…, Set aside duplicates… and Convert… (to CHD, RVZ, CSO, ZIP, or unpacked) are changes too.",
    "For an arcade system (MAME, FinalBurn Neo): how many games are playable, what each needs, its BIOS sets and devices on a tab of their own, and its set type.",
    "Unknown: each file no DAT knows with the game it's likely a variant of (a translation, a hack, a prototype) and its tags, the parentheticals at the end of its name. Add puts it in the system's custom DAT as that game's variant; Edit… names it, gives it tags from the drop-down (or New tag…) and chooses its original, or none for a game of its own. Until then it stays out of the Library.",
    "DATs…: the main DAT, whose counts are the totals, and the others ticked Include in Audit (a translation group's, say): the files they know are off Unknown, their games are in the Library, and each has its counts under the totals with Show its lists.",
    "Variants: the system's custom DAT, each variant with its tags and its original; Rename (its files to their names), Remove and Undo. In the Library a variant shows under its own name with its original's details and pictures, and the Tag filter finds them.",
  ]],
  ["bios", "BIOS files", "Each system's BIOS files: which you have, which are missing, and where they are (under the Audit).", [
    "Every system the BIOS DATs list, the ones you have games for first; search for a system or a file.",
    "Get libretro's list… or Import a BIOS DAT…, then Import BIOS files… (Import › BIOS files) copies or moves the ones you have into the library's BIOS folder; an export can take them with a collection's games.",
  ]],
  ["metadata", "Metadata", "Each game's details, pictures, video and manual, and how much of it you have (under the Audit).", [
    "The systems are in the ribbon on the left. Scrape… a system, a collection or a game, choosing the sources; Change what's scraped… chooses the details and kinds of pictures.",
    "Needs a look lists the games a scrape wasn't sure of: pick the right one in the game's panel, or Identify… it by hand. The panel also attaches your documents.",
    "Delete metadata… or Scrape again… for a system or a game (with a preview and Undo); Import documents… from a folder.",
  ]],
  ["documents", "Documents", "Your guides, manuals and other documents, searchable and readable here (under the Audit).", [
    "Search their words, filter by kind, system, game, where they're from and whether their words are read, grouped by game, kind or system, as a list or a grid of their first pages (small groups side by side); the one picked is read in the panel (a double click opens it larger). Import documents… pairs files with games for you to check first.",
  ]],
  ["collections", "Collections", "Games picked from the library, to export or save as DATs. Collecting moves no files.", [
    "The collections are in the ribbon on the left; the one chosen shows its games as the Library does (grid, list or table), with its Source (or Rules), Size, Exported to and History. Under it in the ribbon are its groups and its systems: a system shows just its games, and clicking it again (or the collection) shows them all.",
    "New collection…: by rules, from folders (matched by hash, so the files' names don't matter), from a DAT read where it is (a review shows the games found before it's made), or empty (games are added later with Add to a collection…). Add a group… adds a group made its own way (a system's translations, say), shown as a section of its own.",
    "Pick games (Ctrl or Shift for more, Ctrl+A for all) and Export these…, or Export… the whole collection; Update… and Check an export from Exported to. An ES-DE export can put each group in a subfolder or as a system of its own, and a Knulli or ROCKNIX export in a subfolder; every export makes a custom collection or playlist of each group where the frontend or the handheld has them (ES-DE, Knulli, RetroArch, Pegasus).",
    "Remove a game from it by hand, or add one; set a size limit for a card; Save as DATs… for RomVault, clrmamepro or igir.",
    "With their updates and DLC (a rule, on by default) takes the games' updates and DLC you have along; History lists each change to what's in it, a day at a time, and Undo puts one back.",
  ]],
  ["backlog", "Backlog", "The games you mean to play, ranked, beside the ones you've completed (under Collections, and at the top of their ribbon).", [
    "Add games… finds games by their titles (yours, or every game in the DATs); in the Library, pick games and choose Add to the backlog, or use the button on a game's page. PC games can go in too.",
    "The first game in To play is the one to play next. Drag a game to its place, or use its menu (Move to the top, Move up, Move down, Move to the bottom, Move to place…), or Alt+↑ and Alt+↓.",
    "In progress marks a game you're playing (as many as you like): it's highlighted, and its page says since when. Completed moves it to the list beside, the newest first, with the date; drag it back, or Back to the backlog, to play it again.",
    "Every change has Undo (Ctrl+Z too). A game opened from either list (a double click or Enter) has Previous and Next through it.",
  ]],
  ["layouts", "Export layouts", "Where an export puts things for ES-DE, RetroArch, Pegasus, LaunchBox, Playnite, a device's firmware or a folder.", [
    "See a layout as a folder tree with a collection's games, change it, and Save as new…; edit its JSON, or import one.",
  ]],
  ["history", "History", "Every change the app has made to your files, newest first, a day at a time.", [
    "Steps lists what a change did; Undo the newest; finish a change that was interrupted; empty what changes set aside, to free the space.",
    "Set aside (a tab, and a button on each change that holds files) lists every file set aside, with its game, where it was and its size: tick the ones you don't want and Delete… them for good. Undo then leaves the steps they came from as they are.",
    "A library two computers share (on a network drive) is changed by one at a time: a change started on one waits while Game Repo on the other is changing it, saying which computer, and History shows a change running there as Running on that computer. If that computer stopped part way, its change shows as Interrupted here ten minutes later, to finish or put back.",
  ]],
  ["settings", "Settings", "How the app works for this library and this computer.", [
    "The library's name and folder, how the folders with games are scanned, systems added by hand, naming, preferred formats, and where PC games and their inbox are.",
    "Sources and their keys, which source each detail comes from and the space details and pictures take, Steam's folder, and tools (dolphin-tool for RVZ, innoextract for GOG installers; CHD, CSO and ZSO the app writes itself).",
    "Serve to devices: Game Repo Server, which serves the library to Game Repo Go on your phones and handhelds, with Pair a device… and the devices paired. Updates, the index on this computer, the skin and the theme.",
  ]],
  ["about", "About", "The app's version and updates, what's changed and what's coming, how a library is laid out, and the licences and credits.", []],
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
