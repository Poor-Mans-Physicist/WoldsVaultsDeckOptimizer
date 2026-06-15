// Raw WASM kernel timing — runs a realistic workload and reports per-restart
// throughput so we can compare against pre-port numbers.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { runSaInventory } from "../web/src/wasm-node/ndm_core.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const CONF  = join(ROOT, "web", "public", "config.json");
const DECKS = join(ROOT, "web", "public", "decks.json");

const cfgBundle = JSON.parse(await readFile(CONF, "utf-8"));
const decks     = JSON.parse(await readFile(DECKS, "utf-8"));
const cfg       = cfgBundle.modes.wolds;

// Snake deck — larger than Starter, has arcane slots, exercises everything.
const deck = decks.wolds.find((d) => d.key === "snake");
if (!deck) throw new Error("snake deck missing");

const slots = deck.slots.map(([r, c]) => [r, c]);
const n = slots.length;
console.log(`Deck: ${deck.name}  slots=${n}  arcane=${deck.arcane_slots.length}`);

// Peer arrays.
function peers() {
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
  return { row, col, sur, dia };
}
const ps = peers();
const arcaneIdx = new Map(slots.map(([r, c], i) => [`${r},${c}`, i]));
const arcane_slot_indices = (deck.arcane_slots ?? [])
  .map(([r, c]) => arcaneIdx.get(`${r},${c}`))
  .filter((i) => i !== undefined);

// Inventory: unlimited (100×) of every type+color.
const TYPES = [
  "row", "col", "surr", "diag", "deluxe", "typeless", "arcane",
  "dir_greed_up", "dir_greed_down", "dir_greed_left", "dir_greed_right",
  "dir_greed_ne", "dir_greed_nw", "dir_greed_se", "dir_greed_sw",
  "evo_greed", "surr_greed",
];
const COLORS = ["red", "green", "blue", "yellow"];
const inventory = [];
for (const t of TYPES) for (const c of COLORS) inventory.push([t, c, 100]);

function payload(cores, n_iter, restarts) {
  return {
    slots,
    row_peers: ps.row, col_peers: ps.col,
    surr_peers: ps.sur, diag_peers: ps.dia,
    arcane_slot_indices,
    auto_place_arcane: true,
    is_shiny: false,                // EVO — exercises all the variable cores
    inventory,
    forced_inventory: [],
    cores,
    n_iter,
    restarts,
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

// Three core combos representative of typical user runs.
const COMBOS = [
  { label: "no cores",                    cores: [] },
  { label: "PURE",                        cores: [["pure", "", -1]] },
  { label: "PURE + FOIL + DELUXE_CORE",   cores: [["pure", "", -1], ["foil", "", -1], ["deluxe_core", "", -1]] },
];

const N_ITER   = 60_000;
const RESTARTS = 12;

console.log(`n_iter=${N_ITER}  restarts=${RESTARTS}  (total per-combo: ${N_ITER * RESTARTS})`);
console.log("");

for (const { label, cores } of COMBOS) {
  const p = payload(cores, N_ITER, RESTARTS);
  // Warm-up call (typically faster after; matches in-app behavior).
  runSaInventory(p);
  const t0 = performance.now();
  const out = runSaInventory(p);
  const ms = performance.now() - t0;
  const iters_per_sec = (N_ITER * RESTARTS / (ms / 1000)).toFixed(0);
  console.log(`${label.padEnd(36)}  ${ms.toFixed(0)}ms  ${iters_per_sec} iter/s  score=${out.score.toFixed(2)}`);
}
