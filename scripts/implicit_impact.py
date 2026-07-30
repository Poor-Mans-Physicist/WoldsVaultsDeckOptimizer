"""Balance sheet + base-vs-implicit comparison (Optimizer 2.0).

Default: ONE spreadsheet-engine pass over the wolds roster under the current
balance regime — implicits on + the selected structural-core layouts
(`--structural-cores`); the kernel models the live archive semantics
(`base^n` additive with the other cores, wv aa5e7b39) — rendered as a
two-column, screenshot-ready HTML leaderboard sorted by NDM (plus markdown
+ console).

The HTML/markdown output carries a SECOND sheet: the "balance settings —
cycle log" (every core value, implicit, greed multiplier, and structural
set that produced the numbers). It is captured by the collect pass itself
and REGENERATED ON EVERY RUN — it must always ship alongside the
leaderboard so each balance cycle stays trackable. Never hand-edit it;
rerun the sheet instead.

`--compare`: the older mode — two passes (with / without implicits) and a
diff table.

Each pass is a real spreadsheet-engine run: same `src.main.optimize()` loop
(every candidate core set × restarts through the tagged kernel), same
production search budget (`config.yaml testing:`), one process per deck.
Decks run under their own shipped constraints (JSON decks: unconstrained).

Usage:
    uv run python scripts/implicit_impact.py               # balance sheet
    uv run python scripts/implicit_impact.py --table-only  # re-render only
    uv run python scripts/implicit_impact.py --compare     # legacy diff table

Passes are separate subprocesses because the CLI gates (implicits /
structural / archive) are resolved from argv at import time in every worker
process — flipping them in-process wouldn't reach spawned workers.
Internal: `--collect --out <json>` runs one pass and dumps per-deck results.
"""
from __future__ import annotations

import argparse
import json
import multiprocessing
import random
import subprocess
import sys
import time
from datetime import date
from pathlib import Path

_REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(_REPO))

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8")

_OUT_WITH    = _REPO / "implicit_impact_with.json"
_OUT_WITHOUT = _REPO / "implicit_impact_without.json"
_OUT_BALANCE = _REPO / "implicit_impact_balance.json"
_OUT_HTML    = _REPO / "implicit_impact.html"
_OUT_MD      = _REPO / "implicit_impact.md"
#: Also served by the dev server for easy screenshots.
_OUT_HTML_PUB = _REPO / "wasm-port" / "web" / "public" / "implicit_impact.html"


# ── collect pass (one full spreadsheet-engine run) ────────────────────────────

def _collect_worker(args):
    from src.main import optimize          # imports config (argv-driven gate)
    deck, n_iter, restarts = args
    random.seed()
    t0 = time.perf_counter()
    res = optimize(deck, n_iter=n_iter, restarts=restarts, verbose=False)
    out = {
        cc.value: {
            "score": r["score"],
            "cores": sorted(c.value for c in r.get("cores", [])),
        }
        for cc, r in res.items()
    }
    print(f"  ✓ {deck.name}  ({time.perf_counter() - t0:.1f}s)", flush=True)
    return deck.key, deck.name, out


def _settings_snapshot(config) -> dict:
    """Full balance-input dump for the settings sheet — captured by the SAME
    process (and argv gates) that ran the optimization, so the logged values
    are exactly what scored the leaderboard. Regenerated on every run; never
    hand-edit (the sheet must stay trackable per balance cycle)."""
    try:
        commit = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"], cwd=_REPO,
            capture_output=True, text=True, timeout=10,
        ).stdout.strip() or "?"
        dirty = subprocess.run(
            ["git", "status", "--porcelain"], cwd=_REPO,
            capture_output=True, text=True, timeout=10,
        ).stdout.strip()
        if dirty:
            commit += "-dirty"
    except Exception:
        commit = "?"

    cores = {
        "Pure":        {"value": f"base {config.MULT_PURE_BASE} + {config.MULT_PURE_SCALE}/non-shiny card",
                        "note": "scales with n_ns (greed + arcane + non-foil positionals)"},
        "Equilibrium": {"value": f"{config.MULT_EQUILIBRIUM}", "note": "Shiny-only flat"},
        "Steadfast":   {"value": f"{config.MULT_STEADFAST}", "note": "Shiny-only flat"},
        "Sparkling":   {"value": f"{config.MULT_SPARKLING}",
                        "note": "Shiny-only flat" + ("" if config.ALLOW_SPARKLING else " (DISABLED)")},
        "Shiny (foil)": {"value": f"{config.MULT_FOIL}",
                         "note": "flat; also flips EVO n_ns to the greed+arcane formula"},
        "Color":       {"value": f"{config.MULT_COLOR}", "note": "flat, matching-color cards"},
        "Deluxe Core": {"value": f"base {config.MULT_DELUXE_CORE_BASE} + {config.MULT_DELUXE_CORE_SCALE}/deluxe card",
                        "note": ("never boosts deluxe cards themselves"
                                 + ("" if config.ALLOW_DELUXE else " (DISABLED)"))},
        "Deluxe card": {"value": f"{config.MULT_DELUXE_FLAT}", "note": "flat base value of a deluxe card"},
        "Void":        {"value": f"base {config.MULT_VOID_CORE_BASE} + {config.MULT_VOID_CORE_SCALE}/dead card",
                        "note": "" if config.ALLOW_VOID else "(DISABLED)"},
        "Archive":     {"value": f"base {config.MULT_ARCHIVE_CORE}",
                        "note": ("per-card stack term (1+v)^n_arcane, ADDITIVE with the "
                                 "other cores (wv aa5e7b39); runic is the only whole-card "
                                 "multiplier" + ("" if config.ALLOW_ARCHIVE else " (DISABLED)"))},
    }
    greeds = {
        "dir vert": config.MULT_DIR_GREED_VERT,
        "dir horiz": config.MULT_DIR_GREED_HORIZ,
        "dir diag up": config.MULT_DIR_GREED_DIAG_UP,
        "dir diag down": config.MULT_DIR_GREED_DIAG_DOWN,
        "evo greed": config.MULT_EVO_GREED,
        "surr greed": config.MULT_SURR_GREED,
    }

    imp_path = _REPO / "decks" / "wolds_implicits.json"
    implicits_raw: dict = {}
    implicits_src = ""
    if imp_path.is_file():
        blob = json.loads(imp_path.read_text(encoding="utf-8")) or {}
        implicits_raw = blob.get("implicits") or {}
        implicits_src = blob.get("_source", "")

    structural: dict = {}
    if config.STRUCTURAL_INCLUDE:
        for deck in config.DECKS:
            if deck.key in config.STRUCTURAL_IMPLICITS or deck.key in ("wold", "fairy", "mystery"):
                forced = config.STRUCTURAL_IMPLICITS.get(deck.key) or []
                structural[deck.key] = {
                    "name": deck.name,
                    "slots": len(deck.slots),
                    "arcane": len(deck.arcane_slots),
                    "core_slots_after": deck.core_slots,
                    "forced_implicits": forced,
                }

    return {
        "commit": commit,
        "mode": config.MODE,
        "engine": str(config._CFG.get("engine", "tagged")),
        "stacking": {"additive_cores": config.ADDITIVE_CORES,
                     "greed_additive": config.GREED_ADDITIVE},
        "implicits_enabled": config.IMPLICITS_ENABLED,
        "structural_enabled": config.STRUCTURAL_INCLUDE,
        "cores": cores,
        "greeds": greeds,
        "implicits": implicits_raw,
        "implicits_source": implicits_src,
        "structural": structural,
    }


def collect(out_path: Path) -> None:
    from src import config
    from src.config import DECKS

    n_iter   = int(config._CFG["testing"]["n_iter"])
    restarts = int(config._CFG["testing"]["restarts"])
    n_procs  = min(len(DECKS), multiprocessing.cpu_count())
    label = " ".join([
        "WITH implicits" if config.IMPLICITS_ENABLED else "WITHOUT implicits",
        "+structural" if config.STRUCTURAL_INCLUDE else "",
    ]).strip()
    print(f"[implicit_impact] pass: {label} — {len(DECKS)} decks × "
          f"{n_iter}×{restarts} across {n_procs} processes")

    args = [(deck, n_iter, restarts) for deck in DECKS]
    with multiprocessing.Pool(processes=n_procs) as pool:
        raw = pool.map(_collect_worker, args)

    payload = {
        "implicits_enabled": config.IMPLICITS_ENABLED,
        "structural": config.STRUCTURAL_INCLUDE,
        "mode": config.MODE,
        "n_iter": n_iter,
        "restarts": restarts,
        "settings": _settings_snapshot(config),
        "decks": {key: {"name": name, "classes": classes} for key, name, classes in raw},
    }
    out_path.write_text(json.dumps(payload, indent=1), encoding="utf-8")
    print(f"[implicit_impact] wrote {out_path.name}")


# ── table rendering ───────────────────────────────────────────────────────────

def _best(classes: dict) -> tuple[float, str]:
    """(best score, winning class letter) across shiny/evo."""
    best_s, best_c = float("-inf"), "?"
    for cname, r in classes.items():
        if r["score"] > best_s:
            best_s, best_c = r["score"], cname[0].upper()
    return best_s, best_c


def _implicit_meta() -> dict:
    path = _REPO / "decks" / "wolds_implicits.json"
    if not path.is_file():
        return {}
    with path.open("r", encoding="utf-8") as fh:
        return (json.load(fh) or {}).get("implicits") or {}


def build_table() -> None:
    with_ = json.loads(_OUT_WITH.read_text(encoding="utf-8"))
    wout  = json.loads(_OUT_WITHOUT.read_text(encoding="utf-8"))
    meta  = _implicit_meta()

    rows = []
    for key, w in with_["decks"].items():
        b = wout["decks"].get(key)
        if b is None:
            continue
        w_score, w_class = _best(w["classes"])
        b_score, b_class = _best(b["classes"])
        m = meta.get(key) or {}
        kind = m.get("kind", "")
        if kind == "mystery":
            note = "player-rolled pair (not in CLI runs)"
        elif kind in ("gameplay", ""):
            note = m.get("name") or "—"
        else:
            note = m.get("name") or kind
        delta = (w_score / b_score - 1.0) * 100.0 if b_score > 0 else 0.0
        rows.append({
            "key": key, "name": w["name"], "implicit": note,
            "desc": (m.get("desc") or "").strip(),
            "kind": kind,
            "base": b_score, "base_class": b_class,
            "with": w_score, "with_class": w_class,
            "delta": delta,
        })
    rows.sort(key=lambda r: r["delta"], reverse=True)

    stamp = date.today().isoformat()
    budget = f"{with_['n_iter']:,}×{with_['restarts']}"

    # — markdown —
    md = [
        f"# Wold's Vaults — deck implicit balance impact ({stamp})",
        "",
        f"Optimizer 2.0 spreadsheet engine, two full runs (with / without deck "
        f"implicits), best of Shiny/Evo per deck, every candidate core set, "
        f"search budget {budget}, deck-default constraints, best-roll cores.",
        "",
        "| Deck | Implicit | Base NDM | With implicit | Δ |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for r in rows:
        md.append(
            f"| {r['name']} | {r['implicit']} | {r['base']:,.1f} ({r['base_class']}) "
            f"| {r['with']:,.1f} ({r['with_class']}) | {r['delta']:+.1f}% |"
        )
    _OUT_MD.write_text("\n".join(md) + "\n", encoding="utf-8")

    # — screenshot-ready HTML —
    def _row_html(r: dict) -> str:
        d = r["delta"]
        # Sub-half-percent deltas are SA noise (identical model both passes,
        # e.g. Mystery) — render as flat rather than implying a real change.
        cls = "up" if d > 0.5 else ("down" if d < -0.5 else "flat")
        dtxt = f"{d:+.1f}%" if abs(d) > 0.5 else "—"
        return (
            f"<tr><td class='deck'>{r['name']}"
            f"<span class='imp'>{r['implicit']}</span></td>"
            f"<td class='num'>{r['base']:,.1f}<span class='cls'>{r['base_class']}</span></td>"
            f"<td class='num'>{r['with']:,.1f}<span class='cls'>{r['with_class']}</span></td>"
            f"<td class='num {cls}'>{dtxt}</td></tr>"
        )

    html = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>WV deck implicit balance impact</title>
<style>
  body {{ background:#0F1115; color:#E5E7EB; font-family:'Segoe UI',system-ui,sans-serif;
         margin:0; padding:28px; }}
  .wrap {{ max-width:860px; margin:0 auto; }}
  h1 {{ font-size:20px; margin:0 0 4px; }}
  .sub {{ color:#9CA3AF; font-size:12px; margin-bottom:16px; line-height:1.5; }}
  table {{ border-collapse:collapse; width:100%; font-size:13px; }}
  th {{ text-align:left; font-size:11px; text-transform:uppercase; letter-spacing:.06em;
       color:#9CA3AF; padding:6px 10px; border-bottom:2px solid #374151; }}
  th.num, td.num {{ text-align:right; font-variant-numeric:tabular-nums; }}
  td {{ padding:5px 10px; border-bottom:1px solid #1F2937; }}
  tr:nth-child(even) td {{ background:#141821; }}
  .deck {{ font-weight:600; }}
  .imp {{ display:block; font-weight:400; font-size:11px; color:#8B93A7; }}
  .cls {{ font-size:9px; color:#6B7280; margin-left:4px; vertical-align:super; }}
  .up   {{ color:#6EE7B7; font-weight:600; }}
  .down {{ color:#FCA5A5; font-weight:600; }}
  .flat {{ color:#6B7280; }}
  .foot {{ color:#6B7280; font-size:11px; margin-top:12px; }}
</style></head><body><div class="wrap">
<h1>Wold's Vaults — deck implicit balance impact</h1>
<div class="sub">Optimizer&nbsp;2.0 spreadsheet engine · two full runs (with&nbsp;/ without deck implicits)
· best of Shiny/Evo per deck (marked <sup>S</sup>/<sup>E</sup>) · every candidate core set ·
search budget {budget} · deck-default constraints · best-roll cores · {stamp}</div>
<table>
<tr><th>Deck · implicit</th><th class="num">Base NDM</th><th class="num">With implicit</th><th class="num">Δ</th></tr>
{"".join(_row_html(r) for r in rows)}
</table>
<div class="foot">Mystery runs implicit-less in the spreadsheet (its pair is picked in-app);
gameplay-only implicits (Villager / Extended / Arcane / Relic) have no NDM effect by design.
“—” = within SA noise (&lt;0.5%). DeckFAST · github.io/woldsvaultsdeckoptimizer</div>
</div></body></html>
"""
    _OUT_HTML.write_text(html, encoding="utf-8")

    # — console —
    print(f"\n{'Deck':<22} {'Base':>10} {'With':>10} {'Δ':>8}   Implicit")
    print("─" * 78)
    for r in rows:
        print(f"{r['name']:<22} {r['base']:>10,.1f} {r['with']:>10,.1f} "
              f"{r['delta']:>+7.1f}%   {r['implicit']}")
    print(f"\n[implicit_impact] wrote {_OUT_MD.name} + {_OUT_HTML.name}")


def _settings_md(s: dict, stamp: str) -> list[str]:
    """Sheet 2 (markdown): the balance-input log for this cycle."""
    if not s:
        return []
    out = [
        "",
        "---",
        "",
        f"## Balance settings — cycle log ({stamp})",
        "",
        f"Captured from the live config by the run itself (DeckFAST commit "
        f"`{s.get('commit', '?')}`, mode `{s.get('mode')}`, engine "
        f"`{s.get('engine')}`). **This log is regenerated on every sheet run — "
        f"it must always ship alongside the leaderboard so each balance cycle "
        f"stays trackable. Never hand-edit it.**",
        "",
        "### Cores",
        "",
        "| Core | Value | Notes |",
        "| --- | --- | --- |",
    ]
    for name, c in s.get("cores", {}).items():
        out.append(f"| {name} | {c['value']} | {c['note']} |")
    out += [
        "",
        "### Greed multipliers & stacking",
        "",
        "| Greed | Mult |",
        "| --- | ---: |",
    ]
    for k, v in s.get("greeds", {}).items():
        out.append(f"| {k} | ×{v} |")
    st = s.get("stacking", {})
    out += [
        "",
        f"Stacking: cores {'additive' if st.get('additive_cores') else 'multiplicative'}, "
        f"greeds {'additive' if st.get('greed_additive') else 'multiplicative'}; "
        f"implicits {'ON' if s.get('implicits_enabled') else 'OFF'}; "
        f"structural builds {'ON' if s.get('structural_enabled') else 'OFF'}.",
        "",
        "### Deck implicits",
        "",
        "| Deck key | Implicit | Kind | Value | Effect |",
        "| --- | --- | --- | ---: | --- |",
    ]
    for key in sorted(s.get("implicits", {})):
        m = s["implicits"][key]
        out.append(f"| {key} | {m.get('name', '—')} | {m.get('kind', '')} "
                   f"| {m.get('value', '')} | {(m.get('desc') or '').strip()} |")
    if s.get("implicits_source"):
        out += ["", f"Implicit source: {s['implicits_source']}"]
    if s.get("structural"):
        out += [
            "",
            "### Structural balance builds",
            "",
            "| Deck | Slots | Arcane | Core budget after | Forced implicits |",
            "| --- | ---: | ---: | ---: | --- |",
        ]
        for key, d in s["structural"].items():
            forced = ", ".join(d["forced_implicits"]) or "—"
            out.append(f"| {d['name']} | {d['slots']} | {d['arcane']} "
                       f"| {d['core_slots_after']} | {forced} |")
    return out


def _settings_html(s: dict, stamp: str) -> str:
    """Sheet 2 (HTML): the balance-input log rendered under the leaderboard."""
    if not s:
        return ""
    core_rows = "".join(
        f"<tr><td class='deck'>{n}</td><td>{c['value']}</td>"
        f"<td class='note'>{c['note']}</td></tr>"
        for n, c in s.get("cores", {}).items()
    )
    greed_rows = "".join(
        f"<tr><td class='deck'>{k}</td><td class='num plain'>×{v}</td></tr>"
        for k, v in s.get("greeds", {}).items()
    )
    imp_rows = "".join(
        f"<tr><td class='deck'>{k}</td><td>{m.get('name', '—')}</td>"
        f"<td>{m.get('kind', '')}</td><td class='num plain'>{m.get('value', '')}</td>"
        f"<td class='note'>{(m.get('desc') or '').strip()}</td></tr>"
        for k, m in sorted(s.get("implicits", {}).items())
    )
    struct_rows = "".join(
        f"<tr><td class='deck'>{d['name']}</td><td class='num plain'>{d['slots']}</td>"
        f"<td class='num plain'>{d['arcane']}</td>"
        f"<td class='num plain'>{d['core_slots_after']}</td>"
        f"<td>{', '.join(d['forced_implicits']) or '—'}</td></tr>"
        for d in s.get("structural", {}).values()
    )
    st = s.get("stacking", {})
    struct_table = ("" if not struct_rows else f"""
<h3>Structural balance builds</h3>
<table class="set"><tr><th>Deck</th><th class="num">Slots</th><th class="num">Arcane</th>
<th class="num">Core budget after</th><th>Forced implicits</th></tr>{struct_rows}</table>""")
    return f"""
<div class="sect">
<h1>Balance settings — cycle log</h1>
<div class="sub"><strong>Regenerated from the live config on every sheet run</strong> — this log must
always ship alongside the leaderboard above so each balance cycle stays trackable (never hand-edit).
DeckFAST commit <code>{s.get('commit', '?')}</code> · mode <code>{s.get('mode')}</code> ·
engine <code>{s.get('engine')}</code> · cores {'additive' if st.get('additive_cores') else 'multiplicative'} ·
greeds {'additive' if st.get('greed_additive') else 'multiplicative'} ·
implicits {'ON' if s.get('implicits_enabled') else 'OFF'} ·
structural builds {'ON' if s.get('structural_enabled') else 'OFF'} · {stamp}</div>
<div class="setcols">
<div>
<h3>Cores</h3>
<table class="set"><tr><th>Core</th><th>Value</th><th>Notes</th></tr>{core_rows}</table>
<h3>Greed multipliers</h3>
<table class="set greed"><tr><th>Greed</th><th class="num">Mult</th></tr>{greed_rows}</table>
{struct_table}
</div>
<div>
<h3>Deck implicits</h3>
<table class="set"><tr><th>Deck</th><th>Implicit</th><th>Kind</th><th class="num">Value</th>
<th>Effect</th></tr>{imp_rows}</table>
</div>
</div>
<div class="foot">Sources: <code>config.yaml</code> · <code>decks/wolds_implicits.json</code> ·
<code>decks/structural_layouts.json</code>. Implicit provenance: {s.get('implicits_source') or '—'}</div>
</div>"""


def build_sheet() -> None:
    """Single-run balance leaderboard: two side-by-side columns sorted by
    NDM (highest first), no comparisons — for Discord screenshots. A second
    sheet below logs every balance input (cores / implicits / greeds /
    structural sets) for the cycle — regenerated on every run so balance
    data stays trackable."""
    data = json.loads(_OUT_BALANCE.read_text(encoding="utf-8"))
    meta = _implicit_meta()

    rows = []
    for key, d in data["decks"].items():
        score, cls = _best(d["classes"])
        m = meta.get(key) or {}
        kind = m.get("kind", "")
        if kind == "mystery":
            imp = "runic + bishop pair (structural build)"
        elif kind in ("gameplay", ""):
            imp = m.get("name") or "—"
        else:
            imp = m.get("name") or kind
        rows.append({"key": key, "name": d["name"], "implicit": imp,
                     "score": score, "cls": cls})
    rows.sort(key=lambda r: r["score"], reverse=True)

    stamp  = date.today().isoformat()
    budget = f"{data['n_iter']:,}×{data['restarts']}"
    struct_names = ", ".join(
        data["decks"][k]["name"] for k in ("wold", "fairy", "mystery")
        if k in data["decks"]
    )
    settings = data.get("settings") or {}
    if not settings:
        print("[implicit_impact] WARN: no settings snapshot in the balance JSON "
              "(old collect run?) — the cycle-log sheet will be missing. Re-run "
              "without --table-only to regenerate it.", file=sys.stderr)

    # — markdown —
    md = [
        f"# Wold's Vaults — deck NDM balance sheet ({stamp})",
        "",
        f"Optimizer 2.0 spreadsheet engine, current balance: deck implicits "
        f"on; greater structural-core builds for {struct_names} (core budget "
        f"reduced accordingly); live archive semantics (1+v)^n additive with "
        f"the other cores; best of Shiny/Evo per deck; every candidate core "
        f"set; search budget {budget}; best-roll cores.",
        "",
        "| # | Deck | Implicit | NDM |",
        "| ---: | --- | --- | ---: |",
    ]
    for i, r in enumerate(rows, 1):
        md.append(f"| {i} | {r['name']} | {r['implicit']} | {r['score']:,.1f} ({r['cls']}) |")
    md += _settings_md(settings, stamp)
    _OUT_MD.write_text("\n".join(md) + "\n", encoding="utf-8")

    # — screenshot-ready HTML: two side-by-side columns —
    def _row_html(i: int, r: dict) -> str:
        return (
            f"<tr><td class='rank'>{i}</td>"
            f"<td class='deck'>{r['name']}<span class='imp'>{r['implicit']}</span></td>"
            f"<td class='num'>{r['score']:,.1f}<span class='cls'>{r['cls']}</span></td></tr>"
        )

    half = (len(rows) + 1) // 2
    def _col(chunk: list, start: int) -> str:
        body = "".join(_row_html(start + j + 1, r) for j, r in enumerate(chunk))
        return (f"<table><tr><th class='rank'>#</th><th>Deck · implicit</th>"
                f"<th class='num'>NDM</th></tr>{body}</table>")

    html = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>WV deck NDM balance sheet</title>
<style>
  body {{ background:#0F1115; color:#E5E7EB; font-family:'Segoe UI',system-ui,sans-serif;
         margin:0; padding:26px; }}
  .wrap {{ max-width:1060px; margin:0 auto; }}
  h1 {{ font-size:20px; margin:0 0 4px; }}
  .sub {{ color:#9CA3AF; font-size:12px; margin-bottom:14px; line-height:1.5; }}
  .cols {{ display:flex; gap:22px; align-items:flex-start; }}
  table {{ border-collapse:collapse; flex:1; font-size:13px; }}
  th {{ text-align:left; font-size:11px; text-transform:uppercase; letter-spacing:.06em;
       color:#9CA3AF; padding:6px 10px; border-bottom:2px solid #374151; }}
  th.num, td.num {{ text-align:right; font-variant-numeric:tabular-nums; }}
  th.rank, td.rank {{ text-align:right; color:#6B7280; width:26px; padding-right:4px; }}
  td {{ padding:5px 10px; border-bottom:1px solid #1F2937; }}
  tr:nth-child(even) td {{ background:#141821; }}
  .deck {{ font-weight:600; }}
  .imp {{ display:block; font-weight:400; font-size:11px; color:#8B93A7; }}
  .cls {{ font-size:9px; color:#6B7280; margin-left:4px; vertical-align:super; }}
  td.num {{ color:#93C5FD; font-weight:600; }}
  td.num.plain {{ color:#E5E7EB; font-weight:400; }}
  .foot {{ color:#6B7280; font-size:11px; margin-top:12px; line-height:1.5; }}
  /* Sheet 2 — balance-settings cycle log. */
  .sect {{ margin-top:40px; padding-top:22px; border-top:2px solid #374151; }}
  .sect h3 {{ font-size:12px; text-transform:uppercase; letter-spacing:.05em;
             color:#9CA3AF; margin:16px 0 6px; }}
  .setcols {{ display:flex; gap:26px; align-items:flex-start; }}
  .setcols > div {{ flex:1; min-width:0; }}
  table.set {{ width:100%; font-size:12px; }}
  table.set td {{ padding:4px 8px; }}
  table.set.greed {{ width:auto; min-width:220px; }}
  .note {{ color:#8B93A7; font-size:11px; }}
  code {{ background:#141821; padding:1px 5px; border-radius:3px; font-size:11px; }}
</style></head><body><div class="wrap">
<h1>Wold's Vaults — deck NDM balance sheet</h1>
<div class="sub">Optimizer&nbsp;2.0 spreadsheet engine · current balance: deck implicits on
(incl. the latest implicit retunes) · greater structural-core builds for {struct_names}
(core budget reduced accordingly) · <strong>live Archive semantics</strong> — (1+v)<sup>n</sup>,
additive with the other cores (no longer a whole-card multiplier) ·
best of Shiny/Evo per deck (marked <sup>S</sup>/<sup>E</sup>) · every candidate core set ·
search budget {budget} · best-roll cores · {stamp}</div>
<div class="cols">
{_col(rows[:half], 0)}
{_col(rows[half:], half)}
</div>
<div class="foot">Mystery is scored with its chosen runic&nbsp;+&nbsp;bishop implicit pair on the
structural (greater Construction) build. Gameplay-only implicits (Villager / Extended / Arcane / Relic)
have no NDM effect by design. DeckFAST · github.io/WoldsVaultsDeckOptimizer</div>
{_settings_html(settings, stamp)}
</div></body></html>
"""
    _OUT_HTML.write_text(html, encoding="utf-8")
    try:
        _OUT_HTML_PUB.write_text(html, encoding="utf-8")
    except OSError as e:
        print(f"[implicit_impact] WARN: couldn't copy sheet to {_OUT_HTML_PUB}: {e}",
              file=sys.stderr)

    # — console —
    print(f"\n{'#':>3} {'Deck':<22} {'NDM':>10}   Implicit")
    print("─" * 70)
    for i, r in enumerate(rows, 1):
        print(f"{i:>3} {r['name']:<22} {r['score']:>10,.1f}   {r['implicit']}")
    print(f"\n[implicit_impact] wrote {_OUT_MD.name} + {_OUT_HTML.name} (+ public copy)")


# ── orchestrator ──────────────────────────────────────────────────────────────

def _run_pass(extra: list[str], out_path: Path) -> None:
    cmd = [sys.executable, str(Path(__file__).resolve()),
           "--collect", "--out", str(out_path)] + extra
    for attempt in (1, 2):
        r = subprocess.run(cmd, cwd=_REPO)
        if r.returncode == 0:
            return
        print(f"[implicit_impact] pass failed (attempt {attempt}, rc={r.returncode})"
              + (" — retrying" if attempt == 1 else ""), file=sys.stderr)
        time.sleep(3)
    raise SystemExit(f"pass failed twice: {' '.join(cmd)}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--collect", action="store_true", help="internal: run one pass")
    ap.add_argument("--out", type=Path, help="internal: JSON output for --collect")
    ap.add_argument("--table-only", action="store_true",
                    help="skip the runs; rebuild the sheet/table from existing JSONs")
    ap.add_argument("--compare", action="store_true",
                    help="legacy mode: two passes (with/without implicits) + diff table")
    # config-gate flags (--no-implicits / --structural-cores) are consumed by
    # src.config at import; accept & ignore.
    args, _ = ap.parse_known_args()

    if args.collect:
        collect(args.out)
        return

    if args.compare:
        if not args.table_only:
            t0 = time.perf_counter()
            _run_pass([], _OUT_WITH)
            _run_pass(["--no-implicits"], _OUT_WITHOUT)
            print(f"[implicit_impact] both passes done in {time.perf_counter() - t0:.0f}s")
        build_table()
        return

    # Default: single balance-regime pass → two-column leaderboard.
    if not args.table_only:
        t0 = time.perf_counter()
        _run_pass(["--structural-cores"], _OUT_BALANCE)
        print(f"[implicit_impact] balance pass done in {time.perf_counter() - t0:.0f}s")
    build_sheet()


if __name__ == "__main__":
    main()
