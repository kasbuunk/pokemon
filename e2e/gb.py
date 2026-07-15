"""PyBoy helpers for the end-to-end suite: symbol parsing, RAM injection,
and scripting a boot from the title screen into the overworld.

How cartridge RAM injection works (PyBoy 2.x): `PyBoy(..., ram_file=<file>)`
reads the file verbatim into the MBC's RAM banks before the first frame
(`pyboy/core/cartridge/base_mbc.py: load_ram`). The format is a raw,
headerless dump of `external_ram_count * 8 KiB` banks — for Pokémon Red
(MBC3, 32 KiB SRAM) that is byte-identical to a `.sav`. Without the kwarg,
PyBoy would look for `<rom path>.ram` next to the ROM; passing the file
object explicitly keeps the tmpdir layout obvious.
"""

import os
from pathlib import Path

from pyboy import PyBoy

_SCRATCHPAD = Path(
    "/tmp/claude-0/-home-user-pokemon/f9a993e2-28ae-5efa-bcbc-1faed555fe2a/scratchpad/pokered"
)

#: wSaveFileStatus value the game sets when the save checksum matched and the
#: save was loaded (pokered engine/menus/main_menu.asm). 1 = no/invalid save.
SAVE_STATUS_VALID = 2


def rom_path():
    return Path(os.environ.get("POKERED_ROM", _SCRATCHPAD / "pokered.gbc"))


def sym_path():
    return Path(os.environ.get("POKERED_SYM", _SCRATCHPAD / "pokered.sym"))


def parse_sym(path):
    """Parse an rgbds .sym file into {label: (bank, address)}.

    Lines look like `00:d163 wPartyCount`; comments start with `;`. The
    first definition of a label wins (aliases at the same address follow).
    """
    symbols = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split(";")[0].strip()
            if not line:
                continue
            location, _, label = line.partition(" ")
            bank, _, addr = location.partition(":")
            if not (bank and addr and label):
                continue
            symbols.setdefault(label.strip(), (int(bank, 16), int(addr, 16)))
    return symbols


class BootTimeout(AssertionError):
    """The game never reached the overworld within the frame budget.

    This is the corruption signal: a save the engine rejects (checksum
    mismatch => no CONTINUE entry) can never trigger `EnterMap`.
    """


class Gen1Game:
    """A booted Pokémon Red with a symbol table for WRAM reads."""

    def __init__(self, rom, save, symbols_file):
        self.symbols = parse_sym(symbols_file)
        # Inject the .sav as cartridge RAM (see module docstring).
        # sound_emulated must stay True: with sound emulation disabled the
        # game's own audio engine (which reads APU registers) never finishes
        # its music fade-out after CONTINUE and the overworld stalls in a
        # PlaySound wait loop. Volume 0 keeps the run silent instead.
        with open(save, "rb") as ram:
            self.pyboy = PyBoy(
                str(rom),
                window="null",
                sound_emulated=True,
                sound_volume=0,
                ram_file=ram,
            )
        self.pyboy.set_emulation_speed(0)

    def close(self):
        # save=False: never write RAM back next to the ROM.
        self.pyboy.stop(save=False)

    def read(self, label, offset=0, length=1):
        """Read `length` bytes of memory at symbol `label` + `offset`."""
        _, addr = self.symbols[label]
        start = addr + offset
        return bytes(self.pyboy.memory[start : start + length])

    def screenshot(self, path):
        """Render one frame and save a PNG (for failure artifacts)."""
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        self.pyboy.tick(1, True)
        self.pyboy.screen.image.save(str(path))

    def boot_to_overworld(self, frame_budget=6000):
        """Drive intro -> title -> menu -> CONTINUE -> overworld.

        Alternates START and A presses (START skips the intro and enters
        the menu from the title screen; A selects CONTINUE and confirms
        the save-info box) while waiting for the game to execute
        `EnterMap` — the home-bank routine that runs exactly when an
        overworld map is loaded. A save the engine considers corrupt
        never offers CONTINUE, so the hook never fires and we time out.
        """
        entered = []
        bank, addr = self.symbols["EnterMap"]
        self.pyboy.hook_register(bank, addr, lambda ctx: ctx.append(True), entered)

        frame = 0
        while not entered:
            if frame >= frame_budget:
                raise BootTimeout(
                    f"did not reach the overworld (EnterMap) within "
                    f"{frame_budget} frames — the game refused the save"
                )
            # A short hold registers reliably; alternate the two buttons.
            if frame % 24 == 0:
                self.pyboy.button("start", 6)
            elif frame % 24 == 12:
                self.pyboy.button("a", 6)
            self.pyboy.tick(1, False)
            frame += 1

        status = self.read("wSaveFileStatus")[0]
        if status != SAVE_STATUS_VALID:
            raise BootTimeout(
                f"reached the overworld but wSaveFileStatus is {status}, "
                f"not {SAVE_STATUS_VALID}: the game did not load our save "
                f"(a NEW GAME was started instead)"
            )
        return frame
