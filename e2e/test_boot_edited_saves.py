"""Boot every save the editor produced in a real Pokémon Red (pokered build)
under PyBoy, CONTINUE past the main menu, and verify the game loaded exactly
the values the fixture generator wrote.

Reaching the overworld at all is the primary assertion: a save the engine
rejects (bad checksum / "corrupt") never offers CONTINUE, so the boot times
out and the test fails. The WRAM comparisons then pin down every edited
field, using expectations the Rust generator emitted (raw bytes, so no Gen 1
charset/BCD reimplementation lives on the Python side).
"""

import shutil

import pytest

import gb
from conftest import FIXTURES_DIR, load_manifest

MANIFEST = load_manifest()


@pytest.mark.parametrize("entry", MANIFEST["fixtures"], ids=lambda e: e["file"])
def test_boot_fixture(entry, rom, sym, tmp_path, artifacts_dir):
    name = entry["file"]
    rom_copy = tmp_path / "pokered.gbc"
    sav_copy = tmp_path / name
    shutil.copy(rom, rom_copy)
    shutil.copy(FIXTURES_DIR / name, sav_copy)

    game = gb.Gen1Game(rom_copy, sav_copy, sym)
    try:
        try:
            game.boot_to_overworld()
        except gb.BootTimeout:
            game.screenshot(artifacts_dir / f"{name}.boot-timeout.png")
            raise

        problems = friendly_mismatches(game, entry["expected"])
        problems += wram_mismatches(game, entry["expected_wram"])
        if problems:
            game.screenshot(artifacts_dir / f"{name}.wram-mismatch.png")
            pytest.fail(
                f"{name}: game accepted the save but WRAM disagrees:\n  "
                + "\n  ".join(problems)
            )
    finally:
        game.close()


def wram_mismatches(game, expected_wram):
    """Compare every {label, offset, bytes} expectation from the manifest."""
    problems = []
    for item in expected_wram:
        want = bytes.fromhex(item["bytes"])
        got = game.read(item["label"], item["offset"], len(want))
        if got != want:
            problems.append(
                f"{item['label']}+{item['offset']}: "
                f"expected {want.hex()}, got {got.hex()}"
            )
    return problems


def friendly_mismatches(game, expected):
    """Value-level checks (readable failures; the raw bytes cover the rest).

    Party mon records are 44 bytes from wPartyMons; the level byte sits at
    +0x21 and max HP (big-endian u16) at +0x22 of each record.
    """
    problems = []

    def check(what, got, want):
        if got != want:
            problems.append(f"{what}: expected {want}, got {got}")

    check("party count (wPartyCount)", game.read("wPartyCount")[0], expected["party_count"])
    for i, (species, level) in enumerate(
        zip(expected["party_species_internal"], expected["party_levels"])
    ):
        check(f"party[{i}] species (wPartySpecies)", game.read("wPartySpecies", i)[0], species)
        check(f"party[{i}] level (wPartyMons+0x21)", game.read("wPartyMons", i * 44 + 0x21)[0], level)
    if "first_mon_max_hp" in expected:
        got = int.from_bytes(game.read("wPartyMons", 0x22, 2), "big")
        check("first mon max HP", got, expected["first_mon_max_hp"])
    check("badges byte (wObtainedBadges)", game.read("wObtainedBadges")[0], expected["badges_byte"])
    return problems
