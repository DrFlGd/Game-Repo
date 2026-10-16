# Platforms

The platforms Game Repo knows, with the names used for them by ES-DE (the platform's id
in the app and its folder name on export), by the DAT groups, and in folder names. The
app's own copy of this table is in the core (`platforms.rs`, Phase 1); this page is the
readable version, and the two are kept in step.

Phase 1 covers cartridge (and cartridge-like) systems. Disc systems come with Phase 2,
arcade with Phase 7, metadata providers' ids with Phase 5.

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

## Later

- Disc systems (Phase 2): PlayStation, PlayStation 2, PSP, Sega CD, Saturn, Dreamcast,
  PC Engine CD, Neo Geo CD, GameCube, Wii, 3DO, Xbox, with Redump's DAT names.
- Arcade (Phase 7): `mame`, `fbneo`, `neogeo`.
- PC (Phase 4): `pc` (no DAT; see docs/PLAN.md, 3.5).
- ScreenScraper, LaunchBox and IGDB ids per platform (Phase 5).
