# Wording

How Game Repo, Game Repo Server and Game Repo Go say things: on their pages, in dialogs and
messages, on the Help page and in the release notes. Later work follows it (docs/PLAN.md,
D144). The Help page's *Words used in the app* (`web/ui/help.js`, `TERMS`) explains the
words below to the person using the app; this file is the list for whoever writes the app.

## How it's said

- **Plain and short.** Everyday words over technical ones; one idea a sentence; the main
  thing first. A label or button is a few words; a hint is one sentence.
- **British spelling**: organise, catalogue, colour, licence (the noun), programme only for
  television (a computer *program*), travelled, centre.
- **Sentence case** everywhere: page titles, buttons, labels, column heads.
- **"You" and "your"** for the person; the app calls itself *Game Repo* (or *the app* when
  the name has just been said). *Game Repo Server* and *Game Repo Go* in full the first time
  in a message, *the server* and *Go* after.
- **Buttons say what happens**, as a verb: *Import*, *Make the collection*, *Add 3 games*,
  *Leave it*. A button that opens a dialog or asks something more ends with "…"
  (*Organise…*, *Add to a collection…*); one that acts at once doesn't.
- **Counts with their noun**, singular for one: *1 game*, *2 games*, *1 file waits*.
- **Messages say what happened, then what to do**: *Couldn't read Alpha.zip: it isn't a ZIP
  file.* Start with *Couldn't*, not *Failed to* or *Error:*; no exclamation marks.
- **A place in the app** is written as its page and section with "›": *Audit › Unknown*,
  *Settings › Sources*, *Import › Games into the library*.
- **No internal names**: no ids, JSON keys, command names or code in what the app says,
  except where the person edits them (an export layout's JSON) or a path of their own.
- **Typography**: one "…" character for an ellipsis, "·" between short facts on one line,
  "›" between a page and its section, straight apostrophes (as the code has them).

## One word for one thing

| Word | What it means | Not |
| --- | --- | --- |
| **library** | The folder Game Repo keeps everything in, and (the *Library* page) the games in it. | catalogue, database |
| **system** | A console, handheld, computer or arcade family: *Game Boy*, *PlayStation*, *Arcade (MAME)*. | platform, console (in what's shown; the code says `platform`) |
| **folder with games** | A folder of yours holding ROMs and disc images that Game Repo scans where it is. | root, ROM folder, source folder |
| **game** | One entry of a DAT, and the files that make it up. A ROM, disc image or archive is a **file** of a game. | title (except in *one per title*), entry |
| **DAT** | A list of games and the files each is made of. | dat file, catalogue |
| **main DAT** | The DAT a system's counts come from (chosen in Audit › *DATs…*). | primary DAT, audit DAT |
| **Include in Audit** | Another DAT of the system that Audit checks the files against too. | audited with it too |
| **custom DAT** | The DAT of a system's variants, which Game Repo keeps for you. | variants DAT |
| **variant** | A translation, hack, prototype or other version that the main DAT doesn't list, kept in the custom DAT with the game it's a version of (its **original**). | mod (except as a tag), custom game |
| **audited games** | The games of the DATs Audit checks a system against: what collections are made from. | the full set |
| **have / partly / missing** | How much of a game your folders hold. | owned, complete |
| **unknown file** | A file none of its system's DATs knows: listed on Audit › Unknown. | stranger, unmatched, unidentified |
| **need review** | Files Game Repo couldn't be sure of, waiting for you to say. | to put right |
| **import** | Bringing files into the library. | add (for whole folders), ingest |
| **audit** | Checking the files against the DATs, changing nothing. | verify, scan (a scan reads; an audit compares) |
| **scan** | Reading the folders with games and hashing their files. | index, refresh |
| **organise** | Changing files the library already holds: names, folders, formats. | fix, tidy, rebuild |
| **change** | Anything that writes, moves, renames or converts files. Shown as a preview first and undone from History. | operation, job, task |
| **set aside** | What a change does in place of deleting: kept in a hidden folder on the same drive until emptied. | delete, trash, recycle |
| **undo** | Putting a change back. | revert, roll back |
| **collection** | Games picked for an export; it copies no files. | playlist (except a frontend's), list |
| **group** | A part of a collection with its own way of choosing games. | section, sub-collection |
| **waiting** | A file imported into a collection that no DAT knows yet; it joins when one does. | pending, queued |
| **export** | Copying a collection's games (and their pictures and the rest) for a frontend, a device or a folder. | sync, deploy |
| **export layout** | Where an export puts each thing. | template, profile |
| **details** | What's known about a game: title, year, genre, description… | metadata (except the *Metadata* page's name and its *Delete metadata…*), info |
| **pictures** | Box fronts, screenshots, logos and the rest. *Picture kinds* are the sorts of picture. | media, images, artwork (except for PC games' *art*) |
| **source** | Where details and pictures come from (ScreenScraper, IGDB…). A collection's *Source* is what it was made from. | provider, scraper, database |
| **scrape** | Asking the sources about your games and keeping what they say. | fetch, download metadata |
| **identify** | Choosing by hand which game something is. | match (as a verb for this) |
| **documents** | Guides, manuals, maps and scans of your own, kept with their games. | docs, files, attachments |
| **magazines** | Issues of game magazines kept by magazine. | periodicals |
| **videos** | Walkthroughs, reviews, retrospectives and others about your games: files in the library, or links to the web. | clips, media (for these) |
| **text recognition** | Reading the words in a picture of a page (Windows' own, or Tesseract). | OCR (except where a tool is named), scanning |
| **BIOS files** | The firmware files emulators need. | firmware (alone), system files |
| **PC game** | A game for Windows, kept in its own folder under *PC*. | Windows game |
| **device** | A handheld, phone or computer that Game Repo Go runs on, paired with Game Repo Server. | client, target |
| **pair** | Letting a device use the server, with the six-digit code. | connect, log in, register |
| **queue** | The games waiting to download on a device. | list, basket |

## When something new is named

Add it to the table above and to the Help page's *Words used in the app*, in the same
words, in the same pull request.
