// Structural cores (WEB ONLY).
//
// The Construction Core lets the player add up to 3 new regular slots to the
// deck grid; the Arcane Core lets the player convert up to 3 existing regular
// slots into arcane slots. Both cost one of the deck's core-slot budget each.
// They mutate the deck layout in the UI before the SA kernel sees it; the
// kernel itself has no idea these cores exist.
//
// This module owns:
//   • the StructuralCores state shape
//   • effectiveDeck()              — derives the optimizer-facing Deck
//   • constructionCandidates()     — positions where a new tile may be placed
//   • canRemoveConstructionTile()  — connectivity check for right-click removal
//   • pruneConvertedAfterChanges() — keep convertedSlots in sync with reality
//
// Connectivity rule: every construction-added tile must remain reachable from
// an original deck slot via 8-direction adjacency, walking only through other
// originals or other added tiles. Removing a tile is allowed iff the remaining
// added tiles all stay reachable. This implicitly forbids the player from
// orphaning a "branch" they built on top of an earlier addition.

import type { Position } from "./types";
import type { Deck, RawDeck } from "./deck";
import { buildDeck } from "./deck";

export interface StructuralCores {
  /** Construction Core toggled on in the cores picker. */
  constructionEnabled: boolean;
  /** Arcane Core toggled on in the cores picker. */
  arcaneCoreEnabled:   boolean;
  /** Positions added by Construction Core (≤3, or ≤5 with Greater). Order is
   *  insertion order — not load-bearing for connectivity (removal is allowed
   *  on any tile whose absence doesn't orphan another addition), but kept
   *  stable so the UI list is calm. */
  addedSlots:     Position[];
  /** Positions originally regular that have been converted to arcane (≤3, or
   *  ≤5 with Greater). Members may be in either deck.slots (native) or
   *  addedSlots (construction). */
  convertedSlots: Position[];
  /** "Greater" structural cores — experimental community variant that bumps
   *  both caps from 3 → 5. Not in the modpack yet; the UI flags it. */
  greaterStructural: boolean;
}

export const MAX_CONSTRUCTION_BASE    = 3;
export const MAX_ARCANE_CONVERT_BASE  = 3;
export const MAX_CONSTRUCTION_GREATER = 5;
export const MAX_ARCANE_GREATER       = 5;

/** Effective Construction Core cap given the Greater toggle. */
export function maxConstruction(sc: StructuralCores): number {
  return sc.greaterStructural ? MAX_CONSTRUCTION_GREATER : MAX_CONSTRUCTION_BASE;
}
/** Effective Arcane Core cap given the Greater toggle. */
export function maxArcaneConvert(sc: StructuralCores): number {
  return sc.greaterStructural ? MAX_ARCANE_GREATER : MAX_ARCANE_CONVERT_BASE;
}

// Hard cap on a deck's footprint — the bounding box of all placeable slots
// (`O` ∪ `A`, native + construction-added) must fit inside this rectangle.
// 9 wide × 6 tall is the engine-side limit; the Construction Core cannot
// push the bbox past it.
export const MAX_GRID_WIDTH  = 9;
export const MAX_GRID_HEIGHT = 6;

export function emptyStructural(): StructuralCores {
  return {
    constructionEnabled: false,
    arcaneCoreEnabled:   false,
    addedSlots:     [],
    convertedSlots: [],
    greaterStructural: false,
  };
}

const posKey = (p: Position) => `${p[0]},${p[1]}`;
const samePos = (a: Position, b: Position) => a[0] === b[0] && a[1] === b[1];

/** How many of the player's core slots the structural cores currently occupy. */
export function structuralCoreCost(sc: StructuralCores): number {
  return (sc.constructionEnabled ? 1 : 0) + (sc.arcaneCoreEnabled ? 1 : 0);
}

/** Build the optimizer-facing Deck with construction additions + arcane
 *  conversions applied. Peers are recomputed because new slots have new peers. */
export function effectiveDeck(base: Deck, sc: StructuralCores, deckmod: number): Deck {
  // Don't pay the cost when nothing changed.
  if (sc.addedSlots.length === 0 && sc.convertedSlots.length === 0) return base;

  const slots: Position[] = [
    ...base.slots.map(([r, c]) => [r, c] as Position),
    ...sc.addedSlots.map(([r, c]) => [r, c] as Position),
  ];

  // Arcane set = native arcane ∪ converted positions that exist in `slots`.
  const slotSet = new Set(slots.map(posKey));
  const arcaneSet = new Set(base.arcaneSlots.map(posKey));
  for (const p of sc.convertedSlots) {
    if (slotSet.has(posKey(p))) arcaneSet.add(posKey(p));
  }
  const arcaneSlots: Position[] = slots.filter((p) => arcaneSet.has(posKey(p)));

  const raw: RawDeck = {
    key:             base.key,
    name:            base.name,
    slots:           slots.map(([r, c]) => [r, c] as [number, number]),
    arcane_slots:    arcaneSlots.map(([r, c]) => [r, c] as [number, number]),
    base_core_slots: base.base_core_slots,
    min_regular:     base.min_regular,
    max_greed:       base.max_greed,
    // The deck's implicit survives layout mutation — dropping it here made
    // every structural-core run silently lose the implicit.
    implicit:        base.implicit,
  };
  return buildDeck(raw, deckmod);
}

/** Bounding box of the union of original + added slots. */
function slotsBbox(base: Deck, sc: StructuralCores): {
  minR: number; maxR: number; minC: number; maxC: number;
} {
  const all: Position[] = [
    ...base.slots, ...sc.addedSlots,
  ];
  let minR = Infinity, maxR = -Infinity, minC = Infinity, maxC = -Infinity;
  for (const [r, c] of all) {
    if (r < minR) minR = r;
    if (r > maxR) maxR = r;
    if (c < minC) minC = c;
    if (c > maxC) maxC = c;
  }
  return { minR, maxR, minC, maxC };
}

/** All cells adjacent (8-direction) to any existing slot (original or added)
 *  that aren't themselves already slots AND don't push the slot bbox past
 *  MAX_GRID_WIDTH × MAX_GRID_HEIGHT. Returns [] once the placement cap is
 *  reached so the UI won't surface candidates. */
export function constructionCandidates(base: Deck, sc: StructuralCores): Position[] {
  if (!sc.constructionEnabled) return [];
  if (sc.addedSlots.length >= maxConstruction(sc)) return [];

  const bb = slotsBbox(base, sc);

  const occupied = new Set<string>([
    ...base.slots.map(posKey),
    ...sc.addedSlots.map(posKey),
  ]);
  const candidates = new Set<string>();
  for (const k of occupied) {
    const [r, c] = k.split(",").map(Number);
    for (let dr = -1; dr <= 1; dr++) {
      for (let dc = -1; dc <= 1; dc++) {
        if (dr === 0 && dc === 0) continue;
        const nr = r + dr;
        const nc = c + dc;
        const nk = `${nr},${nc}`;
        if (occupied.has(nk)) continue;
        // Reject anything that would push the slot bbox past the 9×6 cap.
        // A candidate that lies INSIDE the current bbox is always fine
        // regardless of these checks (the union with current bbox doesn't
        // grow), and anything outside has to satisfy the width/height limit.
        const newW = Math.max(bb.maxC, nc) - Math.min(bb.minC, nc) + 1;
        const newH = Math.max(bb.maxR, nr) - Math.min(bb.minR, nr) + 1;
        if (newW > MAX_GRID_WIDTH)  continue;
        if (newH > MAX_GRID_HEIGHT) continue;
        candidates.add(nk);
      }
    }
  }
  return [...candidates].map((k) => {
    const [r, c] = k.split(",").map(Number);
    return [r, c] as Position;
  });
}

/** Right-click removal is only valid if every OTHER added tile stays reachable
 *  from an original deck slot through 8-adjacency. Originals are roots, the
 *  search walks through originals + remaining additions. */
export function canRemoveConstructionTile(target: Position, base: Deck, sc: StructuralCores): boolean {
  const remaining = sc.addedSlots.filter((p) => !samePos(p, target));
  if (remaining.length === 0) return true;

  const universe = new Set<string>([
    ...base.slots.map(posKey),
    ...remaining.map(posKey),
  ]);
  // BFS from every original. Anything reachable is "connected to structure."
  const visited = new Set<string>();
  const queue: Position[] = [...base.slots];
  for (const p of queue) visited.add(posKey(p));
  while (queue.length) {
    const [r, c] = queue.shift()!;
    for (let dr = -1; dr <= 1; dr++) {
      for (let dc = -1; dc <= 1; dc++) {
        if (dr === 0 && dc === 0) continue;
        const np: Position = [r + dr, c + dc];
        const nk = posKey(np);
        if (!universe.has(nk) || visited.has(nk)) continue;
        visited.add(nk);
        queue.push(np);
      }
    }
  }
  // Every remaining addition must be reachable.
  return remaining.every((p) => visited.has(posKey(p)));
}

/** After Construction Core changes (deselect / remove), some `convertedSlots`
 *  entries may no longer reference any slot at all. Drop them. */
export function pruneConvertedSlots(base: Deck, sc: StructuralCores): Position[] {
  const valid = new Set<string>([
    ...base.slots.map(posKey),
    ...sc.addedSlots.map(posKey),
  ]);
  return sc.convertedSlots.filter((p) => valid.has(posKey(p)));
}

/** Test whether a position is among `sc.addedSlots`. */
export function isAddedSlot(p: Position, sc: StructuralCores): boolean {
  return sc.addedSlots.some((q) => samePos(q, p));
}

/** Test whether a position is among `sc.convertedSlots`. */
export function isConvertedSlot(p: Position, sc: StructuralCores): boolean {
  return sc.convertedSlots.some((q) => samePos(q, p));
}
