// Port of `_format_breakdown` in src/gui.py — produces the multi-line
// text shown in the click-to-open per-slot popup.

import type { SlotBreakdown } from "./breakdown";
import type { Position } from "./types";
import { TYPE_LABEL } from "./palette";

function titleCase(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export function formatBreakdown(pos: Position, b: SlotBreakdown): string {
  const typeName  = TYPE_LABEL[b.cardType] ?? b.cardType;
  const colorName = b.color !== null ? titleCase(b.color) : "—";
  const head = `${typeName} · ${colorName}  @ (${pos[0]},${pos[1]})`;
  const sep  = "─".repeat(Math.max(head.length, 24));

  const out: string[] = [head, sep, ""];

  // Base
  out.push("Base value:");
  out.push(`  ${b.baseExplain}`);
  out.push(`  → ${stripFloat(b.baseValue)}`);
  if (b.finalNdm === 0.0) {
    out.push("");
    out.push("(does not contribute to NDM)");
    return out.join("\n");
  }
  out.push("");

  // Applied cores
  out.push("Cores applied to this card:");
  if (b.appliedCores.length === 0) out.push("  (none)");
  for (const c of b.appliedCores) {
    const label = c.color !== null ? `${c.core_type} (${c.color})` : c.core_type;
    const tag   = c.override ? " (override)" : "";
    out.push(`  • ${label.padEnd(18)} ×${c.value.toFixed(3)}${tag}`);
  }
  out.push(`  formula: ${b.coreMultFormula}`);
  out.push(`  → core_mult = ×${b.coreMult.toFixed(3)}`);
  out.push("");

  // Excluded cores
  if (b.excludedCores.length > 0) {
    out.push("Cores excluded from this card:");
    for (const x of b.excludedCores) {
      const label = x.color !== null ? `${x.core_type} (${x.color})` : x.core_type;
      out.push(`  • ${label} — ${x.reason}`);
    }
    out.push("");
  }

  // Greed
  out.push("Boost (greed):");
  if (b.boostSources.length === 0) out.push("  (no greed targeting this slot)");
  for (const s of b.boostSources) {
    out.push(
      `  • ${(s.greedType as string).padEnd(14)} from (${s.fromPosition[0]},${s.fromPosition[1]}) → ×${s.multiplier.toFixed(3)}`,
    );
  }
  out.push(`  → boost = ×${b.boost.toFixed(3)}`);
  out.push("");

  out.push(`Final: ${stripFloat(b.baseValue)} × ${b.coreMult.toFixed(3)} × ${b.boost.toFixed(3)}`);
  out.push(`     = ${b.finalNdm.toFixed(3)}`);
  return out.join("\n");
}

// Python's `%g` formatter strips trailing zeros; mirror it for compact integers.
function stripFloat(v: number): string {
  if (Number.isInteger(v)) return v.toString();
  return v.toPrecision(6).replace(/\.?0+$/, "");
}
