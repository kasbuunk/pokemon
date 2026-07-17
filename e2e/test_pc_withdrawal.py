"""Scripted in-game PC withdrawal: prove the engine derives the party level
from experience, not from the box level byte (issue #29).

The boxmon.sav fixture stores CHARMANDER in the current box with a
deliberately stale level byte (80) while its experience corresponds to
level 50 — the exact regression shape behind the editor's exp-authoritative
withdrawal model (`_MoveMon` calls `CalcLevelFromExperience`; the box level
byte is cosmetic). This test drives the real menu flow — Pokémon Center PC
→ BILL'S PC → WITHDRAW MON → slot 2 — and asserts the withdrawn party mon
comes out at level 50.

Getting to a PC: the fixture spawns in the player's bedroom, whose only
warp (the staircase at (7, 1)) normally leads downstairs. Instead of baking
a second captured engine-state block for a Pokémon Center into the fixture,
the test redirects that warp's cached WRAM entry to VIRIDIAN_POKECENTER and
walks onto it: the engine then performs a *genuine* warp — `LoadMapHeader`
reloads the destination's header, warps, objects and tileset from ROM — so
everything after the first step is the real game running real map data.

Every phase synchronizes on a symbol hook (EnterMap, DisplayPCMainMenu,
BillsPC_, BillsPCWithdraw, DisplayDepositWithdrawMenu, _MoveMon), never on
blind frame counts; menu cursor moves are verified through
wCurrentMenuItem before confirming.
"""

import shutil

import pytest

import gb
from conftest import FIXTURES_DIR

FIXTURE = "boxmon.sav"

#: pokered constants/map_constants.asm
VIRIDIAN_POKECENTER = 0x29

#: In-box slot of CHARMANDER (slot 2 on screen) and its level facts from
#: the fixture generator (crates/xtask/src/e2e_fixtures.rs `boxmon()`).
CHARMANDER_SLOT = 1
STALE_BOX_LEVEL = 80
EXP_LEVEL = 50

#: Box mon records are 33 bytes at +0x16 of the box block (wBoxCount);
#: the (cosmetic) level byte sits at +3 of each record. docs/FORMAT.md.
BOX_MONS = 0x16
BOX_MON_SIZE = 33
BOX_LEVEL = 3

#: Party mon records are 44 bytes (wPartyMons); the live level byte the
#: game recomputes on withdrawal sits at +0x21.
PARTY_MON_SIZE = 44
PARTY_LEVEL = 0x21


def test_withdrawal_derives_level_from_exp(rom, sym, tmp_path, artifacts_dir):
    rom_copy = tmp_path / "pokered.gbc"
    sav_copy = tmp_path / FIXTURE
    shutil.copy(rom, rom_copy)
    shutil.copy(FIXTURES_DIR / FIXTURE, sav_copy)

    game = gb.Gen1Game(rom_copy, sav_copy, sym)
    try:
        try:
            run_withdrawal(game)
        except gb.BootTimeout:
            game.screenshot(artifacts_dir / f"{FIXTURE}.withdrawal-stuck.png")
            raise
    finally:
        game.close()


def run_withdrawal(game):
    game.boot_to_overworld()
    game.run_frames(120)  # let EnterMap finish before touching WRAM

    # The fixture's box state, straight from the WRAM block LoadSAV filled:
    # 2 mons, CHARMANDER in slot 2 with the stale level byte.
    assert game.read("wBoxCount")[0] == 2
    charmander = game.read("wBoxCount", 1 + CHARMANDER_SLOT)[0]  # species list
    stale = game.read("wBoxCount", BOX_MONS + CHARMANDER_SLOT * BOX_MON_SIZE + BOX_LEVEL)[0]
    assert stale == STALE_BOX_LEVEL, f"fixture drifted: box level byte {stale}"
    assert game.read("wPartyCount")[0] == 0

    # Redirect the bedroom staircase warp (the map's only warp, cached in
    # the save's engine-state block) to warp 0 of VIRIDIAN_POKECENTER:
    # entry layout Y, X, destination warp ID, destination map.
    game.write("wWarpEntries", [1, 7, 0, VIRIDIAN_POKECENTER])

    # Step onto the staircase at (7, 1); the engine performs the genuine
    # warp and EnterMap runs again for the Pokémon Center.
    arrivals = len(game.entermap_fired) + 1
    game.walk_to(7, 1, prefer="x", stop=lambda: len(game.entermap_fired) >= arrivals)
    game.wait_for(game.entermap_fired, count=arrivals, what="the Pokémon Center warp")
    game.run_frames(120)
    assert game.read("wCurMap")[0] == VIRIDIAN_POKECENTER

    # The PC is the hidden object at (13, 3), used from the tile below,
    # facing up (data/events/hidden_events.asm).
    game.walk_to(13, 4, prefer="y")
    game.face("up")

    main_menu = game.hook("DisplayPCMainMenu")
    bills = game.hook("BillsPC_")
    withdraw = game.hook("BillsPCWithdraw")
    submenu = game.hook("DisplayDepositWithdrawMenu")
    moved = game.hook("_MoveMon")
    bills_menu = game.hook("BillsPCMenu")

    # A on the PC → "turned on the PC" → main menu; A on item 0 (BILL'S /
    # SOMEONE'S PC) → Bill's box menu; A on item 0 (WITHDRAW MON) → the
    # box mon list. Taps between milestones are idempotent: a tap that
    # lands during drawing/text is never queued into the next menu.
    game.wait_for(main_menu, press="a", what="the PC main menu")
    game.wait_for(bills, press="a", what="BILL'S PC")
    game.wait_for(withdraw, press="a", what="the WITHDRAW MON list")

    # Move the list cursor to CHARMANDER (slot 2) with feedback through
    # wCurrentMenuItem — a tap the list wasn't ready for is retried, an
    # overshoot onto CANCEL is corrected.
    budget = 900
    while game.read("wCurrentMenuItem")[0] != CHARMANDER_SLOT:
        if budget <= 0:
            raise gb.BootTimeout("could not select CHARMANDER in the box list")
        cur = game.read("wCurrentMenuItem")[0]
        game.pyboy.button("down" if cur < CHARMANDER_SLOT else "up", 6)
        game.run_frames(24)
        budget -= 24

    game.wait_for(submenu, press="a", what="the WITHDRAW/STATS/CANCEL menu")
    # Submenu opens with the cursor on WITHDRAW; confirming runs MoveMon.
    game.wait_for(moved, press="a", what="_MoveMon")
    # Flow unwinds (mon removed from box, "taken out" text) and redraws
    # Bill's menu when everything is done.
    game.wait_for(bills_menu, count=len(bills_menu) + 1, press="a", what="the withdrawal to finish")

    assert game.read("wPartyCount")[0] == 1
    assert game.read("wPartySpecies")[0] == charmander
    assert game.read("wBoxCount")[0] == 1
    level = game.read("wPartyMons", PARTY_LEVEL)[0]
    assert level == EXP_LEVEL, (
        f"withdrawn CHARMANDER is L{level}: the game must derive the party "
        f"level from experience (L{EXP_LEVEL}), never from the stale box "
        f"level byte (L{STALE_BOX_LEVEL})"
    )
