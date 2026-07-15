"""Shared pytest fixtures: input file locations and the failure-artifact dir.

The suite needs three inputs:
- POKERED_ROM: a pokered-built Pokémon Red ROM (byte-identical to retail),
- POKERED_SYM: the matching rgbds symbol file,
- save fixtures + manifest in ./fixtures (from
  `cargo run -p xtask -- make-e2e-fixtures --out e2e/fixtures`).
"""

import json
from pathlib import Path

import pytest

import gb

E2E_DIR = Path(__file__).resolve().parent
FIXTURES_DIR = E2E_DIR / "fixtures"
ARTIFACTS_DIR = E2E_DIR / "artifacts"


def load_manifest():
    manifest = FIXTURES_DIR / "fixtures.json"
    if not manifest.is_file():
        raise FileNotFoundError(
            f"{manifest} missing — run "
            f"`cargo run -p xtask -- make-e2e-fixtures --out e2e/fixtures` first"
        )
    with open(manifest, encoding="utf-8") as f:
        return json.load(f)


@pytest.fixture(scope="session")
def rom():
    path = gb.rom_path()
    if not path.is_file():
        pytest.fail(f"ROM not found: {path} (set POKERED_ROM)", pytrace=False)
    return path


@pytest.fixture(scope="session")
def sym():
    path = gb.sym_path()
    if not path.is_file():
        pytest.fail(f"symbol file not found: {path} (set POKERED_SYM)", pytrace=False)
    return path


@pytest.fixture(scope="session")
def artifacts_dir():
    ARTIFACTS_DIR.mkdir(parents=True, exist_ok=True)
    return ARTIFACTS_DIR
