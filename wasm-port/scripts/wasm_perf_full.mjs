// Full-flow timing: enumerate candidates like optimizeInventory does, run
// the WASM kernel for each, and time the whole loop. Approximates what the
// browser actually does per Run click (minus the worker/structured-clone
// overhead and the final breakdown re-score).

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { runSaInventory } from "../web/src/wasm-node/ndm_core.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const cfgBundle = JSON.parse(await readFile(join(ROOT, "web", "public", "config.json"), "utf-8"));
const decks     = JSON.parse(await readFile(join(ROOT, "web", "public", "decks.json"), "utf-8"));
const cfg       = cfgBundle.modes.wolds;

// Snake deck (large, has arcane slots).
const deck = decks.wolds.find((d) => d.key === "snake");
const slots = deck.slots.map(([r, c]) => [r, c]);
const n = slots.length;

// Build peers.
const row = [], col = [], sur = [], dia = [];
for (let i = 0; i < n; i++) {
  const [r, c] = slots[i];
  const ro = [], co = [], su = [], di = [];
  for (let j = 0; j < n; j++) {
    if (j === i) continue;
    const [qr, qc] = slots[j];
    if (qr === r) ro.push(j);
    if (qc === c) co.push(j);
    if (Math.max(Math.abs(qr - r), Math.abs(qc - c)) <= 1) su.push(j);
    if (qr - qc === r - c || qr + qc === r + c) di.push(j);
  }
  row.push(ro); col.push(co); sur.push(su); dia.push(di);
}
const arcaneIdx = new Map(slots.map(([r, c], i) => [`${r},${c}`, i]));
const arcane_slot_indices = (deck.arcane_slots ?? [])
  .map(([r, c]) => arcaneIdx.get(`${r},${c}`))
  .filter((i) => i !== undefined);

// Realistic "Unlimited (100×)" inventory.
const TYPES = [
  "row", "col", "surr", "diag", "deluxe", "typeless", "arcane",
  "dir_greed_up", "dir_greed_down", "dir_greed_left", "dir_greed_right",
  "dir_greed_ne", "dir_greed_nw", "dir_greed_se", "dir_greed_sw",
  "evo_greed", "surr_greed",
];
const COLORS = ["red", "green", "blue", "yellow"];
const inventory = [];
for (const t of TYPES) for (const c of COLORS) inventory.push([t, c, 100]);

const basePayload = {
  slots,
  row_peers: row, col_peers: col, surr_peers: sur, diag_peers: dia,
  arcane_slot_indices,
  auto_place_arcane: true,
  is_shiny: false,        // EVO
  inventory,
  forced_inventory: [],
  n_iter: 60_000,
  restarts: 12,
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

// Inline candidate enumeration matching cores.ts logic. EVO with all cores
// enabled (PURE/EQUI/STEAD/FOIL/COLOR/DELUXE_CORE/VOID).
// We won't bother enumerating COLOR per-color in this perf test — just count
// what cores.ts would yield with a representative configuration.
const SCENARIOS = [
  {
    label: "minimal: PURE only (1 candidate)",
    cores: [["pure", "", -1]],
    candidates: [[["pure", "", -1]]],
  },
  {
    label: "modest: PURE + FOIL + DELUXE (3 candidates roughly)",
    cores: [["pure","",-1],["foil","",-1],["deluxe_core","",-1]],
    candidates: [
      [["foil","",-1]],
      [["pure","",-1],["foil","",-1]],
      [["foil","",-1],["deluxe_core","",-1]],
      [["pure","",-1],["foil","",-1],["deluxe_core","",-1]],
    ],
  },
  {
    label: "heavy: PURE+EQUI+STEAD+FOIL+COLOR+DELUXE+VOID, EVO no-FOIL group only",
    cores: "all",
    // EVO group A var pool = [DELUXE, VOID] → 4 subsets × 5 color choices = 20
    candidates: (() => {
      const list = [];
      const varCombos = [
        [],
        [["deluxe_core","",-1]],
        [["void_core","",-1]],
        [["deluxe_core","",-1],["void_core","",-1]],
      ];
      for (const colorPick of [null, ["color","red",-1],["color","green",-1],["color","blue",-1],["color","yellow",-1]]) {
        for (const v of varCombos) {
          const combo = [...v];
          if (colorPick) combo.push(colorPick);
          // Add Pure as filler if it's not already.
          if (!combo.some((c) => c[0] === "pure")) combo.push(["pure","",-1]);
          list.push(combo);
        }
      }
      return list;
    })(),
  },
];

for (const s of SCENARIOS) {
  // Warm up.
  runSaInventory({ ...basePayload, cores: s.candidates[0] });
  const t0 = performance.now();
  let bestScore = -1;
  for (const cores of s.candidates) {
    const out = runSaInventory({ ...basePayload, cores });
    if (out.score > bestScore) bestScore = out.score;
  }
  const ms = performance.now() - t0;
  console.log(`${s.label}`);
  console.log(`  ${s.candidates.length} candidates, ${ms.toFixed(0)}ms total (${(ms/s.candidates.length).toFixed(0)}ms/candidate)  bestScore=${bestScore.toFixed(2)}`);
}
