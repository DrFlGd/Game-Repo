# Platforms

The platforms Game Repo knows, with the names used for them by ES-DE (the platform's id
in the app and its folder name on export), by the DAT groups, and in folder names. The
app's own copy of this table is in the core (`platforms.rs`, Phase 1); this page is the
readable version, and the two are kept in step.

Phase 1 covers cartridge (and cartridge-like) systems, Phase 2b the disc systems
(below the cartridges), arcade comes with Phase 7, metadata providers' ids with Phase 5.

**To check during Phase 1:** each DAT name against the current No-Intro release, and each
id against ES-DE's `es_systems.xml`. Anything found wrong here is fixed in both places.
TOSEC's DAT names (`Nintendo Game Boy - Games - [GB] (TOSEC-…)`) aren't in the table
yet: the import asks for their platform once and remembers it.

## How a DAT finds its platform

The DAT's `header/name` is compared with the *DAT names* column after removing the
group's suffixes: anything in brackets at the end that is a date or one of `Headered`,
`Headerless`, `Parent-Clone`, `Decrypted`, `Encrypted`, `BigEndian`, `ByteSwapped`,
`LNX`, `LYX`, `J64`, `ROM`, `Private`. No match: the import asks, and remembers.

## How a file without a match finds its platform

1. The platform set on its folder with games, if any.
2. A folder on its path whose name is one of the *Folder names* (case, spaces and
   punctuation ignored), or the platform's id or DAT name.
3. Its extension, when only one platform uses it (marked **bold** below).
4. Otherwise it's listed under *No platform* on Home.

## Cartridge systems (Phase 1)

| Id (ES-DE) | Platform | DAT names (No-Intro) | Folder names | Extensions | Header |
| --- | --- | --- | --- | --- | --- |
| `nes` | Nintendo Entertainment System | Nintendo - Nintendo Entertainment System | NES, Famicom, FC | **.nes**, **.unf**, **.unif** | iNES / NES 2.0, 16 B |
| `fds` | Famicom Disk System | Nintendo - Family Computer Disk System | FDS, Famicom Disk System | **.fds** | fwNES, 16 B |
| `snes` | Super Nintendo | Nintendo - Super Nintendo Entertainment System | SNES, Super Nintendo, Super Famicom, SFC | **.sfc**, **.smc**, **.swc**, **.fig** | copier, 512 B |
| `n64` | Nintendo 64 | Nintendo - Nintendo 64 | N64, Nintendo 64 | **.z64**, **.v64**, **.n64** | byte order normalised to big-endian |
| `gb` | Game Boy | Nintendo - Game Boy | GB, Game Boy | **.gb** | |
| `gbc` | Game Boy Color | Nintendo - Game Boy Color | GBC, Game Boy Color | **.gbc** | |
| `gba` | Game Boy Advance | Nintendo - Game Boy Advance | GBA, Game Boy Advance | **.gba** | |
| `nds` | Nintendo DS | Nintendo - Nintendo DS | NDS, DS, Nintendo DS | **.nds** | |
| `virtualboy` | Virtual Boy | Nintendo - Virtual Boy | Virtual Boy, VB | **.vb** | |
| `pokemini` | Pokémon Mini | Nintendo - Pokemon Mini | Pokemon Mini | **.min** | |
| `genesis` | Sega Genesis / Mega Drive | Sega - Mega Drive - Genesis | Genesis, Mega Drive, Megadrive, MD | **.md**, **.gen**, .bin, .smd | |
| `mastersystem` | Sega Master System | Sega - Master System - Mark III | Master System, SMS, Mark III | **.sms** | |
| `gamegear` | Sega Game Gear | Sega - Game Gear | Game Gear, GG | **.gg** | |
| `sega32x` | Sega 32X | Sega - 32X | 32X, Sega 32X | **.32x** | |
| `sg-1000` | Sega SG-1000 | Sega - SG-1000 | SG-1000, SG1000 | **.sg** | |
| `atari2600` | Atari 2600 | Atari - 2600 | Atari 2600, 2600, VCS | **.a26**, .bin | |
| `atari5200` | Atari 5200 | Atari - 5200 | Atari 5200, 5200 | **.a52**, .bin | |
| `atari7800` | Atari 7800 | Atari - 7800 | Atari 7800, 7800 | **.a78**, .bin | A78, 128 B |
| `atarilynx` | Atari Lynx | Atari - Lynx | Lynx, Atari Lynx | **.lnx**, **.lyx** | LNX, 64 B |
| `atarijaguar` | Atari Jaguar | Atari - Jaguar | Jaguar, Atari Jaguar | **.j64**, **.jag** | |
| `pcengine` | PC Engine / TurboGrafx-16 | NEC - PC Engine - TurboGrafx-16 | PC Engine, TurboGrafx-16, TG16, PCE | **.pce** | |
| `ngp` | Neo Geo Pocket | SNK - Neo Geo Pocket | Neo Geo Pocket, NGP | **.ngp** | |
| `ngpc` | Neo Geo Pocket Color | SNK - Neo Geo Pocket Color | Neo Geo Pocket Color, NGPC | **.ngc** | |
| `wonderswan` | WonderSwan | Bandai - WonderSwan | WonderSwan, WS | **.ws** | |
| `wonderswancolor` | WonderSwan Color | Bandai - WonderSwan Color | WonderSwan Color, WSC | **.wsc** | |
| `colecovision` | ColecoVision | Coleco - ColecoVision | ColecoVision, Coleco | **.col** | |
| `intellivision` | Intellivision | Mattel - Intellivision | Intellivision | **.int** | |
| `vectrex` | Vectrex | GCE - Vectrex | Vectrex | **.vec** | |

`.bin` and `.rom` belong to several platforms, so they never decide one on their own.

## Disc systems (Phase 2b)

Audited against Redump's DATs. A CHD, CSO, ZSO or RVZ is read as the files Redump lists
(a CD CHD as its cue sheet's tracks, the others as one ISO), and *Kept as* is the format
Convert… offers first (Settings › Preferred formats changes it where there's a choice).
Where Redump has renamed a DAT, both names are listed; libretro's copies use the second.

| Id | Name | DAT names (Redump's, then libretro's copy) | Folder names | Extensions | Kept as |
| --- | --- | --- | --- | --- | --- |
| `psx` | PlayStation | Sony - PlayStation | PSX, PS1, PlayStation, PlayStation 1, Sony PlayStation | .cue, .bin, **.img**, .chd, **.m3u** | CHD |
| `ps2` | PlayStation 2 | Sony - PlayStation 2 | PS2, PlayStation 2, Sony PlayStation 2 | .iso, .cue, .bin, .chd | CHD |
| `psp` | PlayStation Portable | Sony - PlayStation Portable | PSP, PlayStation Portable, Sony PlayStation Portable | .iso, **.cso**, **.zso**, .chd | CSO (or ZSO, CHD) |
| `saturn` | Sega Saturn | Sega - Saturn | Saturn, Sega Saturn | .cue, .bin, .iso, .chd | CHD |
| `segacd` | Sega CD / Mega-CD | Sega - Mega CD & Sega CD; Sega - Mega-CD - Sega CD | Sega CD, Mega CD, Mega-CD, SegaCD, MegaCD | .cue, .bin, .iso, .chd | CHD |
| `dreamcast` | Sega Dreamcast | Sega - Dreamcast | Dreamcast, DC, Sega Dreamcast | **.gdi**, **.cdi**, .cue, .bin, .chd | CHD |
| `pcenginecd` | PC Engine CD / TurboGrafx-CD | NEC - PC Engine CD & TurboGrafx CD; NEC - PC Engine CD - TurboGrafx-CD | PC Engine CD, TurboGrafx-CD, TurboGrafx CD, PCE-CD, TG-CD | .cue, .bin, .chd | CHD |
| `neogeocd` | Neo Geo CD | SNK - Neo Geo CD | Neo Geo CD, NeoGeo CD, NGCD | .cue, .bin, .chd | CHD |
| `3do` | 3DO | Panasonic - 3DO Interactive Multiplayer; The 3DO Company - 3DO | 3DO, Panasonic 3DO | .cue, .bin, .iso, .chd | CHD |
| `pcfx` | PC-FX | NEC - PC-FX & PC-FXGA; NEC - PC-FX | PC-FX, PCFX | .cue, .bin, .chd | CHD |
| `atarijaguarcd` | Atari Jaguar CD | Atari - Jaguar CD Interactive Multimedia System; Atari - Jaguar CD | Jaguar CD, Atari Jaguar CD | .cue, .bin, .chd | CHD |
| `cdimono1` | Philips CD-i | Philips - CD-i | CD-i, CDi, Philips CD-i | .cue, .bin, .iso, .chd | CHD |
| `gc` | Nintendo GameCube | Nintendo - GameCube | GameCube, GC, NGC, Nintendo GameCube | .iso, **.gcm**, .rvz, .gcz, **.ciso** | RVZ |
| `wii` | Nintendo Wii | Nintendo - Wii | Wii, Nintendo Wii | .iso, **.wbfs**, .rvz, **.wia**, .gcz | RVZ |
| `xbox` | Xbox | Microsoft - Xbox | Xbox, Microsoft Xbox | .iso, **.xiso** | audited only |
| `mame` | Arcade (MAME) | MAME (any DAT whose name starts with MAME, and MAME's own `-listxml`) | MAME, Arcade | .zip, .7z, .chd | arcade (Phase 7): ZIPs by set type |
| `fbneo` | Arcade (FinalBurn Neo) | FinalBurn Neo - Arcade Games, FBNeo - Arcade Games (and FBNeo's arcade and Neo Geo DATs) | FBNeo, FinalBurn Neo, FBA | .zip, .7z | arcade (Phase 7): ZIPs by set type |

`.cue`, `.bin`, `.iso` and `.chd` belong to several disc platforms, so a disc set is
found by its DAT match or its folder name.

## Systems added by hand (Phase 3, D22)

A system the table doesn't have is added in *Settings › Systems* (or as *A new system…*
when a DAT's system is asked for): its name, its id (the folder exports use, ES-DE's
system name when it has one), its DAT names, folder names and extensions, and what its
games are on (cartridges, CDs, GameCube/Wii discs, PSP discs, or audit only), which
sets the formats Convert… offers. They're kept in the library's `library.json` under
`systems` and join this table when the library opens, so DATs, folders and files find
them as they find the built-in ones. A DAT name is kept as its base name (no version or
`(Parent-Clone)`), so every version of the DAT is found.

## Later

- Arcade (Phase 7): `mame`, `fbneo`, `neogeo`.
- PC (Phase 4): `pc` (no DAT; see docs/PLAN.md, 3.5).
- ScreenScraper, LaunchBox and IGDB ids per platform (Phase 5).
