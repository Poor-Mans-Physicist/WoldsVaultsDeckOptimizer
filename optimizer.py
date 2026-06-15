"""CLI entry point for the Wold's Vaults Deck Optimizer.

Thin wrapper around ``src.main.main`` exposed as the ``optimize`` console
script so the optimizer can be invoked through ``uv run --extra rust optimize``
instead of pointing at a script path.

Card-type key used in grid displays:
    R = Row    C = Col    S = Surr    X = Diag    D = Deluxe    T = Typeless
    ^ = DirGreed(up)    v = DirGreed(down)
    < = DirGreed(left)  > = DirGreed(right)
    ↗ ↖ ↘ ↙ = diagonal DirGreed
    e = EvoGreed    o = SurrGreed    . = filler greed
    · = empty slot
"""

from __future__ import annotations

import sys
from pathlib import Path

# Ensure the bundled ``src`` package is importable when the wrapper is
# launched from a different working directory.
_PROJECT_ROOT = Path(__file__).resolve().parent
if str(_PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(_PROJECT_ROOT))


def main() -> None:
    """Run the optimizer (Rust kernel — mandatory after the consolidation refactor)."""
    from src.main import main as _main
    _main()
