# DAT formats

What Game Repo reads from each kind of DAT, and how it uses it. DATs are imported
through *Settings › DAT files* or `gamerepo-cli dat import <file>`; a copy is kept in
`_library/dats/<platform>/` and the index is loaded from it (docs/PLAN.md, "Phase 1
design").

## Logiqx XML (No-Intro, Redump, TOSEC, FBNeo)

```xml
<datafile>
  <header>
    <name>Nintendo - Super Nintendo Entertainment System</name>
    <description>Nintendo - Super Nintendo Entertainment System</description>
    <version>20261001-123456</version>
    <homepage>No-Intro</homepage>
  </header>
  <game name="Chrono Trigger (USA)" id="0123" cloneofid="0122">
    <description>Chrono Trigger (USA)</description>
    <rom name="Chrono Trigger (USA).sfc" size="4194304" crc="2d206bf7" md5="…" sha1="…" status="verified"/>
  </game>
</datafile>
```

| Read | Where | Used for |
| --- | --- | --- |
| DAT name | `header/name` | Platform detection (after dropping suffixes such as `(Headered)`, `(Parent-Clone)`, `(Decrypted)` and dates), the DAT's identity across versions |
| Version | `header/version` (else `header/date`) | Telling versions apart; the newest is current by default |
| Group | `header/homepage`, `header/author`, `header/url` | Shown on the DATs list |
| Games | `game` (also `machine`) `name`, `description` | DAT games |
| Parent/clone | `cloneof` (name), or No-Intro's `cloneofid` → `id` | 1G1R groups |
| ROMs | `rom` `name`, `size`, `crc`, `md5`, `sha1`, `status` | Matching. `status="nodump"`: not counted as missing; `baddump`: matched and flagged |
| Disks | `disk` | Ignored until discs (Phase 2) and arcade CHDs (Phase 7) |
| Other | `category`, `release`, `sample`, `biosset`, `driver` | Ignored for now; `category` may feed collection rules in Phase 3 |

Read with `quick-xml` as a stream, so a DAT of any size takes little memory. Unknown
elements and attributes are ignored.

## ClrMamePro text

```
clrmamepro (
	name "Nintendo - Game Boy"
	description "Nintendo - Game Boy"
	version 20261001-000000
)

game (
	name "Tetris (World) (Rev 1)"
	description "Tetris (World) (Rev 1)"
	cloneof "Tetris (World)"
	rom ( name "Tetris (World) (Rev 1).gb" size 32768 crc 46df91ad md5 … sha1 … )
)
```

The same fields as Logiqx, read by a small tokenizer (words, quoted strings, brackets).
`resource ( … )` blocks are read like `game`. A `rom` line without a hash it needs is kept
but can only be matched by what it has.

## Zipped DATs

No-Intro's downloads are ZIPs holding one DAT (sometimes several, one per platform). An
imported `.zip` is opened and each `.dat`/`.xml` inside is imported as above.

## Names parsed into fields

At import, each game's name is split by the No-Intro naming convention into a title and
tags, so pages and collection rules don't need regular expressions:

| Field | From | Example |
| --- | --- | --- |
| title | the name before the first `(` | `Chrono Trigger` |
| regions | the first bracket of region names | `USA`, `Europe`, `Japan`, `World`, `USA, Europe` |
| languages | a bracket of language codes | `En,Fr,De` |
| revision | `(Rev N)` | `1` |
| version | `(vN.N)` | `1.1` |
| flags | `(Beta)`, `(Proto)`, `(Demo)`, `(Sample)`, `(Unl)`, `(Pirate)`, `(Aftermarket)`, `[BIOS]`, `(Kiosk)`, `(Virtual Console)`… | `beta` |

TOSEC's convention (`Title (Year)(Publisher)(Region)[flags]`) is parsed the same way
where it can be; anything not understood stays in the name.

## Not yet

- MAME `-listxml` (machines, ROM sets, merges, device and BIOS links, disks): Phase 7.
- Redump's cue sheets beside its DATs: Phase 2, with disc systems.
- Header definition XMLs (No-Intro's `headers/*.xml`): not needed; the header rules are
  built in (docs/PLAN.md, "Phase 1 design").
