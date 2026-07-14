// Game-card modifier loader + classifier (port of src/modifiers.py).
// Reads the verbatim per-mode `public/modifiers_<mode>.json` dump and filters
// down to the previewable stat cards. Family is one of
// "shiny" | "evo" | "deluxe" | "typeless".

const PERCENT_SUFFIXES = ["_percent", "_percentile"];

// Attributes that are percent-style but don't end in _percent / _percentile.
// Mirror the _PERCENT_EXTRA set in src/modifiers.py — extend together.
const PERCENT_EXTRA = new Set<string>([
  "the_vault:damage_increase",
  "the_vault:cooldown_reduction",
  "the_vault:critical_hit_mitigation",
  "the_vault:healing_effectiveness",
  "the_vault:knockback_resistance",
  "the_vault:lucky_hit_chance",
  "the_vault:resistance",
  "the_vault:soul_chance",
  "the_vault:trap_disarming",
  "the_vault:item_quantity",
  "the_vault:item_rarity",
  "the_vault:mana_regen",
  "the_vault:movement_speed",
  "the_vault:area_of_effect",
  "the_vault:copiously",
]);

// Order matters — first match wins (Deluxe before Shiny so deluxe cards that
// also carry stat-family tags end up in "deluxe" exclusively).
const FAMILY_ORDER: ReadonlyArray<[string, CardFamily]> = [
  ["Deluxe",    "deluxe"],
  ["Shiny",     "shiny"],
  ["Evolution", "evo"],
  ["Stat",      "typeless"],
];

export type CardFamily = "shiny" | "evo" | "deluxe" | "typeless";
export const FAMILIES: ReadonlyArray<CardFamily> = ["shiny", "evo", "deluxe", "typeless"];

export interface CardTier { tier: number; value: number; }

// The 9 freeform category tags (mirror of types.ts CATEGORY_GROUPS —
// duplicated here so this module stays dependency-free).
const CATEGORY_TAGS = new Set<string>([
  "Offensive", "Defensive", "Physical", "Magical", "Utility",
  "Resource", "Knack", "Temporal", "Essence",
]);

export interface CardEntry {
  key:            string;
  name:           string;
  attribute:      string;
  attributeShort: string;
  family:         CardFamily;
  isPercent:      boolean;
  tiers:          CardTier[];
  displayAttribute: string;
  /** The card's category tags (Offensive, Utility, …) from the game data —
   *  drives the tag-aware slot filter in the Preview chooser. */
  categories:     string[];
}

function classifyFamily(groups: string[]): CardFamily | null {
  for (const [tag, fam] of FAMILY_ORDER) if (groups.includes(tag)) return fam;
  return null;
}

function isPercent(attribute: string): boolean {
  if (PERCENT_EXTRA.has(attribute)) return true;
  return PERCENT_SUFFIXES.some((s) => attribute.endsWith(s));
}

function stripAttrPrefix(attribute: string): string {
  const i = attribute.indexOf(":");
  return i === -1 ? attribute : attribute.slice(i + 1);
}

function humanize(snake: string): string {
  return snake
    .split("_")
    .filter((p) => p.length > 0)
    .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
    .join(" ");
}

// One card catalog per mode — wolds and vanilla ship different rosters
// (modifiers_wolds.json / modifiers_vanilla.json from build_data.py).
const _cacheByMode = new Map<string, Map<string, CardEntry>>();

/** Fetch & parse the mode's card dump once, classify into CardEntry, cache. */
export async function loadModifiers(baseUrl: string, mode: string): Promise<Map<string, CardEntry>> {
  const cached = _cacheByMode.get(mode);
  if (cached) return cached;

  // `no-cache` like config/decks: revalidate so card retunes aren't stale.
  const res = await fetch(`${baseUrl}modifiers_${mode}.json`, { cache: "no-cache" });
  if (!res.ok) {
    console.warn(`[modifiers] modifiers_${mode}.json fetch failed (${res.status}) — preview will be empty.`);
    const empty = new Map<string, CardEntry>();
    _cacheByMode.set(mode, empty);
    return empty;
  }
  const raw = await res.json();
  const values = raw?.values ?? {};
  const out = new Map<string, CardEntry>();

  for (const [key, entry] of Object.entries<any>(values)) {
    if (!entry || typeof entry !== "object" || entry.type !== "gear") continue;
    const groups: string[] = Array.isArray(entry.groups) ? entry.groups : [];
    const family = classifyFamily(groups);
    if (family === null) continue;

    const attribute = entry.attribute;
    if (typeof attribute !== "string" || !attribute) continue;

    const pool = entry.pool;
    if (!Array.isArray(pool) || pool.length === 0) continue;
    const tiers: CardTier[] = [];
    for (const p of pool) {
      const tier  = Number(p?.tier);
      const value = Number(p?.min);
      if (Number.isFinite(tier) && Number.isFinite(value)) tiers.push({ tier, value });
    }
    if (tiers.length === 0) continue;
    tiers.sort((a, b) => a.tier - b.tier);

    const nameBlock = entry.name;
    const displayName = typeof nameBlock === "object" && nameBlock?.text
      ? String(nameBlock.text) : String(nameBlock ?? key);

    const attributeShort = stripAttrPrefix(attribute);
    out.set(key, {
      key, name: displayName,
      attribute, attributeShort,
      family, isPercent: isPercent(attribute),
      tiers,
      displayAttribute: humanize(attributeShort),
      categories: groups.filter((g) => CATEGORY_TAGS.has(g)),
    });
  }

  _cacheByMode.set(mode, out);
  return out;
}

export function getCard(map: Map<string, CardEntry>, key: string): CardEntry | undefined {
  return map.get(key);
}

export function cardsByFamily(map: Map<string, CardEntry>, family: CardFamily): CardEntry[] {
  return [...map.values()]
    .filter((c) => c.family === family)
    .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
}
