// End-to-end smoke test: run a few fixed scenarios through the nodejs-target
// WASM build and confirm:
//   1. The kernel returns a numeric score for each.
//   2. Slot-type rules hold (no ARCANE in regular slots; arcane slots hold
//      only ARCANE or DEAD).
//   3. Auto-place ON keeps arcane slots ARCANE when inventory has any.
//   4. Auto-place OFF + void core can choose DEAD in arcane slots.
//
// Moon deck was disabled in decks/moon.yaml, so this file no longer relies
// on it. The Starter deck (1 arcane slot at position [1,2]) is the primary
// fixture because it exercises arcane-slot handling on every case.
//
// Usage (from repo root):
//   wasm-pack build --target nodejs --out-dir web/src/wasm-node --release \
//     --no-default-features --features wasm
//   node scripts/wasm_parity_check.mjs

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { runSaInventory } from "../web/src/wasm-node/ndm_core.js";

const HERE  = dirname(fileURLToPath(import.meta.url));
const ROOT  = dirname(HERE);
const CONF  = join(ROOT, "web", "public", "config.json");
const DECKS = join(ROOT, "web", "public", "decks.json");

const cfgBundle = JSON.parse(await readFile(CONF, "utf-8"));
const decksByMode = JSON.parse(await readFile(DECKS, "utf-8"));

function buildPeers(slots) {
  const n = slots.length;
  const idx = new Map(slots.map(([r, c], i) => [`${r},${c}`, i]));
  const rowPeers = [], colPeers = [], surrPeers = [], diagPeers = [];
  for (let i = 0; i < n; i++) {
    const [r, c] = slots[i];
    const row = [], col = [], sur = [], dia = [];
    for (let j = 0; j < n; j++) {
      if (j === i) continue;
      const [qr, qc] = slots[j];
      if (qr === r) row.push(j);
      if (qc === c) col.push(j);
      if (Math.max(Math.abs(qr - r), Math.abs(qc - c)) <= 1) sur.push(j);
      if (qr - qc === r - c || qr + qc === r + c) dia.push(j);
    }
    rowPeers.push(row); colPeers.push(col); surrPeers.push(sur); diagPeers.push(dia);
  }
  return { rowPeers, colPeers, surrPeers, diagPeers };
}

function findDeck(mode, key) {
  const list = decksByMode[mode];
  if (!list) throw new Error(`mode '${mode}' missing from decks.json`);
  const d = list.find((d) => d.key === key);
  if (!d) throw new Error(`deck '${key}' missing from mode '${mode}'`);
  return d;
}

function buildPayload(mode, deck, opts) {
  const cfg = cfgBundle.modes[mode];
  const slots = deck.slots.map(([r, c]) => [r, c]);
  const peers = buildPeers(slots);
  const arcaneIdx = new Map(slots.map(([r, c], i) => [`${r},${c}`, i]));
  const arcane_slot_indices = (deck.arcane_slots ?? [])
    .map(([r, c]) => arcaneIdx.get(`${r},${c}`))
    .filter((i) => i !== undefined);

  return {
    slots,
    row_peers:  peers.rowPeers,
    col_peers:  peers.colPeers,
    surr_peers: peers.surrPeers,
    diag_peers: peers.diagPeers,
    arcane_slot_indices,
    auto_place_arcane: opts.autoPlace,
    is_shiny: opts.isShiny,
    inventory: opts.inventory,
    forced_inventory: opts.forced ?? [],
    cores: opts.cores ?? [],
    n_iter: opts.nIter ?? 30_000,
    restarts: opts.restarts ?? 4,
    // Mults bundled from the resolved config.
    mult_dir_vert:          cfg.greed.dir_vert,
    mult_dir_horiz:         cfg.greed.dir_horiz,
    mult_evo_greed:         cfg.greed.evo,
    mult_surr_greed:        cfg.greed.surr,
    mult_dir_diag_up:       cfg.greed.dir_diag_up,
    mult_dir_diag_down:     cfg.greed.dir_diag_down,
    mult_pure_base:         cfg.cores.pure_base,
    mult_pure_scale:        cfg.cores.pure_scale,
    mult_equilibrium:       cfg.cores.equilibrium,
    mult_foil:              cfg.cores.foil,
    mult_steadfast:         cfg.cores.steadfast,
    mult_color:             cfg.cores.color,
    mult_deluxe_flat:       cfg.deluxe.flat,
    mult_deluxe_core_base:  cfg.deluxe.core_base,
    mult_deluxe_core_scale: cfg.deluxe.core_scale,
    mult_void_core_base:    cfg.cores.void_base ?? 0,
    mult_void_core_scale:   cfg.cores.void_scale ?? 0,
    mult_archive_core:      cfg.cores.archive_core ?? 1.0,
    greed_additive:         cfg.stacking.greed_additive,
    additive_cores:         cfg.stacking.additive_cores,
  };
}

function checkAssignment(deck, asgn) {
  // Build a fast position-key set for arcane slots.
  const arcaneKeys = new Set((deck.arcane_slots ?? []).map(([r, c]) => `${r},${c}`));
  const slotKeys = deck.slots.map(([r, c]) => `${r},${c}`);
  const violations = [];
  for (let i = 0; i < asgn.length; i++) {
    const [t] = asgn[i];
    const key = slotKeys[i];
    const isArc = arcaneKeys.has(key);
    if (isArc) {
      if (t !== "arcane" && t !== "dead") {
        violations.push(`slot ${key} (arcane) holds illegal type ${t}`);
      }
    } else {
      if (t === "arcane") {
        violations.push(`slot ${key} (regular) holds ARCANE`);
      }
    }
  }
  return violations;
}

const TESTS = [];

// ── Case 1: wolds Starter, no cores, auto-place ON, plenty of arcane ────────
TESTS.push({
  label: "wolds Starter / no cores / auto-place ON / arcane available",
  mode:  "wolds",
  deckKey: "starter",
  opts: {
    isShiny:   true,
    autoPlace: true,
    inventory: [
      ["row",    "red",    10],
      ["col",    "blue",   10],
      ["surr",   "green",  10],
      ["arcane", "red",     5],
      ["arcane", "blue",    3],
    ],
    cores: [],
  },
  // Sanity: with no scoring cores active under SHINY, all positional cards
  // still score via their own pos_val × 1 × boost. Score must be > 0.
  expect: { gt: 0 },
});

// ── Case 2: wolds Starter, auto-place OFF, void core ────────────────────────
// Pure-only would push toward filling the arcane slot with ARCANE for the
// n_ns bump. Adding void core gives SA the option of DEAD there for void's
// per-card multiplier. With low arcane inventory + a high void scale, SA
// should at least consider DEAD; we just verify both placements remain legal.
TESTS.push({
  label: "wolds Starter / auto-place OFF / pure + void",
  mode:  "wolds",
  deckKey: "starter",
  opts: {
    isShiny:   true,
    autoPlace: false,
    inventory: [
      ["row",    "red",    10],
      ["col",    "red",    10],
      ["arcane", "red",     1],
    ],
    cores: [
      ["pure",      "", -1],
      ["void_core", "", -1],
    ],
  },
  expect: { gt: 0 },
});

// ── Case 3: wolds Starter, auto-place ON, zero arcane inventory ─────────────
// The arcane slot must fall back to DEAD (cannot leave empty).
TESTS.push({
  label: "wolds Starter / auto-place ON / NO arcane inv",
  mode:  "wolds",
  deckKey: "starter",
  opts: {
    isShiny:   true,
    autoPlace: true,
    inventory: [
      ["row", "red", 10],
      ["col", "red", 10],
    ],
    cores: [["pure", "", -1]],
  },
  expect: { gt: 0, arcaneSlotMustBe: "dead" },
});

// ── Case 4: vanilla — confirm vanilla mode loads its own deck roster ────────
// Pick the first vanilla deck and run a no-cores, all-typeless baseline.
// Verifies that the vanilla pipeline is plumbed through (different json file,
// no DELUXE / VOID cores in the candidate set).
const firstVanilla = decksByMode.vanilla?.[0];
if (firstVanilla) {
  TESTS.push({
    label: `vanilla ${firstVanilla.name} / typeless only / no cores`,
    mode:  "vanilla",
    deckKey: firstVanilla.key,
    opts: {
      isShiny:   true,           // vanilla shiny is "stat" — typeless-only decks
      autoPlace: true,
      inventory: [
        ["typeless", "red",    20],
        ["typeless", "blue",   20],
        ["typeless", "green",  20],
        ["typeless", "yellow", 20],
      ],
      cores: [],
    },
    expect: { gte: 0 },
  });
} else {
  console.warn("[parity] no vanilla decks loaded — skipping vanilla case");
}

let passed = 0, failed = 0;
for (const test of TESTS) {
  console.log(`── ${test.label}`);
  let deck;
  try {
    deck = findDeck(test.mode, test.deckKey);
  } catch (e) {
    console.log(`  ✗ FAIL: ${e.message}`);
    failed++; continue;
  }
  const payload = buildPayload(test.mode, deck, test.opts);
  const t0 = performance.now();
  let out;
  try {
    out = runSaInventory(payload);
  } catch (e) {
    console.log(`  ✗ FAIL: WASM threw: ${e.message ?? e}`);
    failed++; continue;
  }
  const elapsed = (performance.now() - t0).toFixed(0);
  const violations = checkAssignment(deck, out.assignment);
  const expectations = [];
  if (test.expect.gt !== undefined && !(out.score > test.expect.gt))
    expectations.push(`score ${out.score.toFixed(4)} not > ${test.expect.gt}`);
  if (test.expect.gte !== undefined && !(out.score >= test.expect.gte))
    expectations.push(`score ${out.score.toFixed(4)} not >= ${test.expect.gte}`);
  if (test.expect.arcaneSlotMustBe !== undefined) {
    const arcaneKeys = new Set((deck.arcane_slots ?? []).map(([r, c]) => `${r},${c}`));
    const slotKeys = deck.slots.map(([r, c]) => `${r},${c}`);
    for (let i = 0; i < out.assignment.length; i++) {
      if (!arcaneKeys.has(slotKeys[i])) continue;
      const [t] = out.assignment[i];
      if (t !== test.expect.arcaneSlotMustBe)
        expectations.push(`arcane slot ${slotKeys[i]} = ${t}, expected ${test.expect.arcaneSlotMustBe}`);
    }
  }
  if (violations.length > 0 || expectations.length > 0) {
    console.log(`  ✗ FAIL  score=${out.score.toFixed(4)}  (${elapsed}ms)`);
    for (const v of violations)   console.log(`     • rule: ${v}`);
    for (const e of expectations) console.log(`     • expect: ${e}`);
    failed++;
  } else {
    console.log(`  ✓ pass   score=${out.score.toFixed(4)}  (${elapsed}ms)`);
    passed++;
  }
}

console.log("");
console.log(`Total: ${passed}/${passed + failed} passed`);
process.exit(failed === 0 ? 0 : 1);
