"""CLI entry points for the Wold's Vaults Deck Optimizer.

These are thin wrappers around ``src.main.main`` exposed as console scripts so
the optimizer can be invoked through ``uv run optimize`` /
``uv run optimize-py`` instead of pointing at a script path.

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

# Ensure the bundled ``src`` package is importable when the wrappers are
# launched from a different working directory.
_PROJECT_ROOT = Path(__file__).resolve().parent
if str(_PROJECT_ROOT) not in sys.path:
    sys.path.insert(0, str(_PROJECT_ROOT))


def main() -> None:
    """Run the optimizer using the Rust core when available."""
    from src.main import main as _main
    _main()


def main_no_rust() -> None:
    """Run the optimizer in pure-Python mode (Rust core disabled)."""
    # Setting the module to ``None`` in ``sys.modules`` causes any subsequent
    # ``import ndm_core`` to raise ``ImportError``, so the simulation module's
    # try/except fallback path is used. Must run before ``src.main`` (which
    # transitively imports ``src.simulate``) is loaded.
    sys.modules["ndm_core"] = None  # type: ignore[assignment]
    from src.main import main as _main
    _main()
