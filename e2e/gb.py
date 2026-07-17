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

#: `wSpritePlayerStateData1FacingDirection` values (pokered
#: constants/sprite_data_constants.asm: SPRITE_FACING_*).
FACING = {"down": 0x0, "up": 0x4, "left": 0x8, "right": 0xC}


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

    def write(self, label, data, offset=0):
        """Write `data` bytes into memory at symbol `label` + `offset`."""
        _, addr = self.symbols[label]
        for i, byte in enumerate(data):
            self.pyboy.memory[addr + offset + i] = byte

    def hook(self, label):
        """Register a PC hook on a ROM symbol; returns the list that gets
        a `True` appended every time the routine starts executing."""
        fired = []
        bank, addr = self.symbols[label]
        self.pyboy.hook_register(bank, addr, lambda ctx: ctx.append(True), fired)
        return fired

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
        entered = self.hook("EnterMap")
        # Kept for callers that need to synchronize on later map loads
        # (every warp executes EnterMap again).
        self.entermap_fired = entered

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

    def run_frames(self, n):
        """Advance the emulation n frames without rendering."""
        for _ in range(n):
            self.pyboy.tick(1, False)

    def is_alive(self, frames=240):
        """Whether the game engine is still healthy: run `frames` frames
        and require the LCD to stay on and the in-game play-time clock
        (which only ticks in a working overworld loop) to advance. A game
        that took a wild jump into the rst $38 crash loop fails both.
        """
        before = self.read("wPlayTimeMinutes", 0, 3)  # minutes, seconds, frames
        self.run_frames(frames)
        after = self.read("wPlayTimeMinutes", 0, 3)
        lcd_on = self.pyboy.memory[0xFF40] & 0x80 != 0
        return lcd_on and after != before

    # -- overworld / menu scripting --------------------------------------

    def player_pos(self):
        """(x, y) of the player in map tiles."""
        return self.read("wXCoord")[0], self.read("wYCoord")[0]

    def wait_for(self, fired, count=1, frame_budget=900, press=None, every=30, what=""):
        """Tick until the hook list `fired` holds >= `count` entries,
        optionally tapping `press` every `every` frames (menus and text
        boxes poll for fresh presses, so short taps with gaps register
        reliably and never double-fire)."""
        frame = 0
        while len(fired) < count:
            if frame >= frame_budget:
                raise BootTimeout(
                    f"timed out after {frame_budget} frames waiting for {what}"
                )
            if press is not None and frame % every == 0:
                self.pyboy.button(press, 6)
            self.pyboy.tick(1, False)
            frame += 1

    def walk_to(self, x, y, frame_budget=6000, prefer="y", stop=None):
        """Walk the player to tile (x, y) by coordinate feedback: press
        toward the target one step at a time and re-read the position, so
        a step swallowed by a collision (furniture, a wandering NPC) is
        simply retried. After a few blocked steps the axis order flips to
        route around static obstacles. `stop` aborts early (e.g. when a
        warp fired and the coordinates now belong to another map)."""
        stuck = 0
        while frame_budget > 0:
            if stop is not None and stop():
                return
            cur = self.player_pos()
            if cur == (x, y):
                return
            moves = []
            if cur[1] != y:
                moves.append("up" if y < cur[1] else "down")
            if cur[0] != x:
                moves.append("left" if x < cur[0] else "right")
            if prefer == "x":
                moves.reverse()
            if stuck >= 5 and len(moves) > 1:
                moves.reverse()
            # A held direction moves one tile per 16 frames; 18 held + a
            # short settle covers turn-and-step reliably.
            self.pyboy.button(moves[0], 18)
            self.run_frames(24)
            frame_budget -= 24
            stuck = stuck + 1 if self.player_pos() == cur else 0
        raise BootTimeout(f"could not walk to ({x}, {y}): stuck at {self.player_pos()}")

    def face(self, direction, frame_budget=240):
        """Face `direction` without leaving the current tile. Gen 1 has no
        turn-in-place input: a press always attempts a step, so this only
        terminates when the step is blocked (facing changes, coordinates
        do not) — use it against walls/furniture only."""
        want = FACING[direction]
        start = self.player_pos()
        while frame_budget > 0:
            if (
                self.read("wSpritePlayerStateData1FacingDirection")[0] == want
                and self.player_pos() == start
            ):
                return
            self.pyboy.button(direction, 4)
            self.run_frames(20)
            frame_budget -= 20
        raise BootTimeout(f"could not face {direction} at {start}")
