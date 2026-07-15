# Pokémon Generation I (Red/Blue/Yellow) Save Format Reference

Ground truth for this repository. All offsets below were derived from the
**pret/pokered** disassembly at commit `1e96034092686d006e863cace09e87273051a3d8`,
whose build reproduces the retail Pokémon Red ROM byte-for-byte
(SHA-1 `ea9bcae617fdf159b045185467ae58b2e4a48b9a`, matching `roms.sha1`).
WRAM/SRAM label addresses come from the `pokered.sym` file emitted by that build
and cross-check against Bulbapedia's *Save data structure (Generation I)* and
*Pokémon data structure (Generation I)* articles. Where this document and any
other source disagree, the `.sym` file wins.

`.srm` and `.sav` are the same thing: a raw dump of the cartridge's 32 KiB
battery-backed SRAM. Expected size is exactly **0x8000 (32768) bytes**; some
emulators pad to 64 KiB or append RTC/footer bytes — everything past 0x8000
must be preserved verbatim and otherwise ignored.

## Bank layout (4 × 0x2000)

| Bank | File range | Contents |
|---|---|---|
| 0 | 0x0000–0x1FFF | 3 sprite buffers (0x188 each), 0x100 filler, Hall of Fame at **0x0598** |
| 1 | 0x2000–0x3FFF | Main save data 0x2598–0x3522, checksum 0x3523 |
| 2 | 0x4000–0x5FFF | Boxes 1–6 + checksums |
| 3 | 0x6000–0x7FFF | Boxes 7–12 + checksums |

Mapping rules used throughout:
- SRAM label → file offset: `bank * 0x2000 + (addr - 0xA000)`.
- Main-block WRAM label → file offset: `0x25A3 + (wLabel - 0xD2F7)`
  (anchor: `sMainData` = 0x25A3 is a verbatim copy of WRAM from
  `wMainDataStart` = `wPokedexOwned` = $D2F7 up to `wMainDataEnd` = $DA80).
- Party WRAM label → file offset: `0x2F2C + (wLabel - 0xD163)`
  (`sPartyData` copies `wPartyDataStart` = $D163, size 0x194).

## Checksums

All checksums use the same algorithm: 8-bit wrapping sum of the covered bytes,
then bitwise NOT. (`chk = !sum` — equivalently start at 0xFF and subtract.)

| Checksum byte | Covers |
|---|---|
| 0x3523 (`sMainDataCheckSum`) | 0x2598..=0x3522 (`sGameData`, i.e. player name through tileset byte) |
| 0x5A4C (`sBank2AllBoxesChecksum`) | 0x4000..=0x5A4B (all six box blocks of bank 2) |
| 0x5A4D–0x5A52 (`sBank2IndividualBoxChecksums`) | one per box: 0x4000+i*0x462 ..= len 0x462 |
| 0x7A4C (`sBank3AllBoxesChecksum`) | 0x6000..=0x7A4B |
| 0x7A4D–0x7A52 | per-box, boxes 7–12 |

Note: the game only trusts/loads box banks after they've been initialized
(bit 7 of 0x284C). The main checksum is what gates "the file data is
destroyed!" on boot. Sprite buffers + Hall of Fame (bank 0) are not
checksummed.

## Bank 1 — main data (sym-verified offsets)

| File offset | WRAM label | Size | Field |
|---|---|---|---|
| 0x2598 | (sPlayerName) | 11 | Player name, 0x50-terminated |
| 0x25A3 | wPokedexOwned $D2F7 | 19 | Pokédex owned bitfield (151 bits, bit0 = Bulbasaur = dex 1) |
| 0x25B6 | wPokedexSeen $D30A | 19 | Pokédex seen bitfield |
| 0x25C9 | wNumBagItems $D31D | 1 | Bag item count (max 20) |
| 0x25CA | wBagItems $D31E | 41 | 20 × [item id, qty] + 0xFF terminator |
| 0x25F3 | wPlayerMoney $D347 | 3 | Money, big-endian BCD (max 999999) |
| 0x25F6 | wRivalName $D34A | 11 | Rival name |
| 0x2601 | wOptions $D355 | 1 | Options: bits0-3 text speed (1 fast/3 med/5 slow), bit6 battle style (set=Set), bit7 battle animations (set=off) |
| 0x2602 | wObtainedBadges $D356 | 1 | Badges bitfield, bit0 = Boulder … bit7 = Earth |
| 0x2604 | wLetterPrintingDelayFlags $D358 | 1 | Letter print delay flags |
| 0x2605 | wPlayerID $D359 | 2 | Trainer ID, big-endian |
| 0x260A | wCurMap $D35E | 1 | Current map id |
| 0x260B | wCurrentTileBlockMapViewPointer $D35F | 2 | Map view pointer (little-endian WRAM pointer) |
| 0x260D | wYCoord $D361 | 1 | Player Y |
| 0x260E | wXCoord $D362 | 1 | Player X |
| 0x260F | wYBlockCoord $D363 | 1 | Y block coord |
| 0x2610 | wXBlockCoord $D364 | 1 | X block coord |
| 0x2611 | wLastMap $D365 | 1 | Last map (for Dungeon warps) |
| 0x2613 | wCurMapTileset $D367 | 1 | Current tileset |
| 0x271C | (Yellow only) | 1 | Pikachu friendship (unused byte in R/B — same layout) |
| 0x27E6 | wNumBoxItems $D53A | 1 | PC item count (max 50) |
| 0x27E7 | wBoxItems $D53B | 101 | 50 × [item id, qty] + 0xFF |
| 0x284C | wCurrentBoxNum $D5A0 | 1 | bits0-6 current box (0–11), bit7 = boxes initialized (MUST stay set) |
| 0x284E | wNumHoFTeams $D5A2 | 1 | Number of Hall of Fame teams recorded |
| 0x2850 | wPlayerCoins $D5A4 | 2 | Casino coins, big-endian BCD (max 9999) |
| 0x2852 | wToggleableObjectFlags $D5A6 | 32 | Missable/toggleable overworld object flags (256 bits) |
| 0x289C | wGameProgressFlags $D5F0 | 0xC8 | Game progress flags region |
| 0x299C | wObtainedHiddenItemsFlags $D6F0 | 14 | Hidden item pickup flags |
| 0x29AA | wObtainedHiddenCoinsFlags $D6FE | 2 | Hidden coin pickup flags |
| 0x29B7 | wTownVisitedFlag $D70B | 2 | Fly-unlocked towns bitfield (bit0 = Pallet Town) |
| 0x29B9 | wSafariSteps $D70D | 2 | Safari steps remaining |
| 0x29C1 | wRivalStarter $D715 | 1 | Rival's starter species (internal index) |
| 0x29C3 | wPlayerStarter $D717 | 1 | Player's starter species (internal index) |
| 0x29F3 | wEventFlags $D747 | 320 | Event flags (`flag_array NUM_EVENTS`, NUM_EVENTS = $A00 = 2560 bits; 507 named events allocated sparsely per map, last used bit 2522) — story milestones AND one flag per battled trainer (names in pokered `constants/event_constants.asm`) |
| 0x2CED | wPlayTimeHours $DA41 | 1 | Play time hours |
| 0x2CEE | wPlayTimeMaxed $DA42 | 1 | Play time maxed flag |
| 0x2CEF | wPlayTimeMinutes $DA43 | 1 | Minutes |
| 0x2CF0 | wPlayTimeSeconds $DA44 | 1 | Seconds |
| 0x2CF1 | wPlayTimeFrames $DA45 | 1 | Frames |
| 0x2CF4 | wDayCareInUse $DA48 | 1 | Daycare occupied (0/1) |
| 0x2CF5 | wDayCareMonName $DA49 | 11 | Daycare mon nickname |
| 0x2D00 | wDayCareMonOT $DA54 | 11 | Daycare mon OT name |
| 0x2D0B | wDayCareMon $DA5F | 33 | Daycare mon (box format) |
| 0x2D2C | (sSpriteData) | 0x200 | Overworld sprite state (opaque; preserve) |
| 0x2F2C | (sPartyData) | 0x194 | Party block (below) |
| 0x30C0 | (sCurBoxData) | 0x462 | Working copy of the current box |
| 0x3522 | (sTileAnimations) | 1 | Tile animation/tileset type byte — last checksummed byte |
| 0x3523 | (sMainDataCheckSum) | 1 | Main checksum |

Bytes not listed (gaps inside 0x2598–0x3522) are engine state that must be
preserved verbatim. The main block is a raw copy of live WRAM — everything in
it matters to the game on load.

## Party block (0x2F2C, 0x194 bytes)

```
+0x000  1    count (0–6)
+0x001  7    species internal-index list, 0xFF after last entry
+0x008  264  6 × 44-byte party mon
+0x110  66   6 × 11-byte OT name
+0x152  66   6 × 11-byte nickname
```

## Box block (each 0x462 bytes: boxes at 0x4000+i*0x462 bank 2, 0x6000+i*0x462 bank 3, current at 0x30C0)

```
+0x000  1    count (0–20)
+0x001  21   species list + 0xFF
+0x016  660  20 × 33-byte box mon
+0x2A2  220  20 × 11-byte OT name
+0x37E  220  20 × 11-byte nickname
```

The in-game "current box" lives at 0x30C0 and is written back to its bank slot
on box switch. When editing the current box, edit 0x30C0 (and optionally sync
the bank copy); when editing others, edit the bank copy + its two checksums.

## Pokémon structures (all multi-byte integers big-endian)

Party mon, 44 bytes (box mon = first 33 bytes, i.e. through offset 0x20):

| Off | Size | Field |
|---|---|---|
| 0x00 | 1 | Species (internal index) |
| 0x01 | 2 | Current HP |
| 0x03 | 1 | Level (box copy — stale in party; authoritative party level at 0x21) |
| 0x04 | 1 | Status: bits0-2 sleep turns, bit3 poison, bit4 burn, bit5 freeze, bit6 paralysis |
| 0x05 | 1 | Type 1 |
| 0x06 | 1 | Type 2 (== type 1 if monotype) |
| 0x07 | 1 | Catch rate (becomes held item on Gen 2 trade) |
| 0x08–0x0B | 4×1 | Move indexes 1–4 (0 = none) |
| 0x0C | 2 | Original trainer ID |
| 0x0E | 3 | Experience |
| 0x11 | 2 | HP stat experience |
| 0x13 | 2 | Attack stat exp |
| 0x15 | 2 | Defense stat exp |
| 0x17 | 2 | Speed stat exp |
| 0x19 | 2 | Special stat exp |
| 0x1B | 2 | DVs: byte0 = Attack<<4 \| Defense, byte1 = Speed<<4 \| Special |
| 0x1D–0x20 | 4×1 | PP: bits0-5 current PP, bits6-7 PP Ups used |
| 0x21 | 1 | Level (party only) |
| 0x22 | 2 | Max HP (party only, calculated) |
| 0x24 | 2 | Attack |
| 0x26 | 2 | Defense |
| 0x28 | 2 | Speed |
| 0x2A | 2 | Special |

- HP DV (0–15) is derived: `(atk&1)<<3 | (def&1)<<2 | (spd&1)<<1 | (spc&1)`.
- Stat formula (floor division):
  `other = ((base + dv) * 2 + min(63, ceil_sqrt(stat_exp) / 4 … )` — precisely:
  `E = floor(min(255, ceil(sqrt(min(stat_exp, 65535)))) / 4)`;
  `other = floor(((base + dv) * 2 + E) * level / 100) + 5`;
  `hp    = floor(((base + dv) * 2 + E) * level / 100) + level + 10`.
- Box→party withdraw recomputes level-dependent fields: level byte 0x21 :=
  box level (0x03) and the five stats from base stats + DVs + stat exp.

## Hall of Fame (bank 0, 0x0598)

50 teams (`HOF_TEAM_CAPACITY`) × 96 bytes (`HOF_TEAM` = 6 × 16). Each 16-byte
record: species(1), level(1), nickname(11), padding(3). Team count in main
block at 0x284E. Not checksummed.

## Text encoding (English)

0x50 terminates; fields are padded after the terminator (conventionally with
0x50 or 0x00 — preserve unknown trailing bytes on read, canonicalize with 0x50
only when writing a new value). Key points: 0x7F space, 0x80–0x99 A–Z,
0xA0–0xB9 a–z, 0xBA 'é', 0xF6–0xFF 0–9, 0xE1/0xE2 PK/MN glyphs,
0xE3 '-', 0xE6 '?', 0xE7 '!', 0xE8 '.', 0xE0 '’', 0xE4 '’r', 0xE5 '’m',
0xF1 '×', 0xF3 '/', 0xF4 ',', 0x54 POKé macro. Full table:
pokered `constants/charmap.asm`.

## Species numbering

Saves store the **internal index** (1–190; 0xBF+ and 0 are glitch). Rhydon =
0x01, Bulbasaur = 0x99, Charmander = 0xB0, Squirtle = 0xB1, Mew = 0x15. 39
gap indexes decode as MissingNo. The Pokédex bitfields, by contrast, are
indexed by **National Dex number − 1**. Mapping source of truth:
`constants/pokemon_constants.asm` + `data/pokemon/dex_order.asm`
(generated into `crates/pksave/src/gen1/data/generated/`).

## Red/Blue vs Yellow

Identical save layout (same offsets, sizes, checksums). Yellow gives meaning
to 0x271C (Pikachu friendship) and uses the starter fields differently.
Variant detection is therefore heuristic and only affects UI labeling, never
parsing.
