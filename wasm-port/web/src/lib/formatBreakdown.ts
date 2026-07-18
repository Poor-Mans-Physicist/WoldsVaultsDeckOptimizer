// Produces the multi-line text shown in the Shift+click per-slot popup.
// Consumes the Optimizer 2.0 tagged breakdown (implicits, tags, mirror).

import type { TaggedSlotBreakdown as SlotBreakdown } from "./taggedBreakdown";
import type { Position } from "./types";
import { TYPE_LABEL } from "./palette";
import { coreKeyDisplay } from "./coreOptions";

function titleCase(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export function formatBreakdown(pos: Position, b: SlotBreakdown): string {
  const typeName  = TYPE_LABEL[b.cardType] ?? b.cardType;
  const colorName = b.color !== null ? titleCase(b.color) : "—";
  const scaleNote = b.scaleColor !== null && b.scaleColor !== b.color
    ? ` (scales ${b.scaleColor})` : "";
  const head = `${typeName} · ${colorName}${scaleNote}  @ (${pos[0]},${pos[1]})`;
  const sep  = "─".repeat(Math.max(head.length, 24));

  const out: string[] = [head, sep];
  // Stat is run-derived — hidden on every surface, incl. here.
  const shownTags = b.groups.filter((g) => g !== "Stat");
  if (shownTags.length > 0) out.push(`Tags: ${shownTags.join(", ")}`);
  out.push("");

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
    const key   = coreKeyDisplay(c.core_type);
    const label = c.color !== null ? `${key} (${c.color})` : key;
    const tag   = c.override ? " (override)" : "";
    out.push(`  • ${label.padEnd(18)} ×${c.value.toFixed(3)}${tag}`);
  }
  if (b.implicitParts.length > 0) {
    out.push("Deck implicit (additive into core_mult):");
    for (const p of b.implicitParts) {
      out.push(`  • ${p.label}  → +${p.addend.toFixed(3)}`);
    }
  }
  out.push(`  formula: ${b.coreMultFormula}`);
  out.push(`  → core_mult = ×${b.coreMult.toFixed(3)}`);
  out.push("");

  // Excluded cores
  if (b.excludedCores.length > 0) {
    out.push("Cores excluded from this card:");
    for (const x of b.excludedCores) {
      const key   = coreKeyDisplay(x.core_type);
      const label = x.color !== null ? `${key} (${x.color})` : key;
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

  // Archive core — applied OUTSIDE the per-card core_mult stack, so shown as
  // its own factor. Only render when archive is actually contributing.
  const showArchive = b.archiveMult !== 1.0;
  if (showArchive) {
    out.push("Archive core (outside core stack):");
    out.push(`  • ${b.archiveArcaneCount} arcane placed → ${b.archiveBase.toFixed(3)}^(2.1·√${b.archiveArcaneCount}) = ×${b.archiveMult.toFixed(3)}`);
    out.push("");
  }
  // Runic mirror — multiplicative on the whole card value.
  const showMirror = b.mirrorFactor !== 1.0;
  if (showMirror) {
    out.push(`Runic mirror: same-color mirror slot → ×${b.mirrorFactor.toFixed(3)}`);
    out.push("");
  }
  const factors = [
    stripFloat(b.baseValue),
    b.coreMult.toFixed(3),
    b.boost.toFixed(3),
    ...(showArchive ? [b.archiveMult.toFixed(3)] : []),
    ...(showMirror ? [b.mirrorFactor.toFixed(3)] : []),
  ];
  out.push(`Final: ${factors.join(" × ")}`);
  out.push(`     = ${b.finalNdm.toFixed(3)}`);
  return out.join("\n");
}

// Python's `%g` formatter strips trailing zeros; mirror it for compact integers.
function stripFloat(v: number): string {
  if (Number.isInteger(v)) return v.toString();
  return v.toPrecision(6).replace(/\.?0+$/, "");
}
