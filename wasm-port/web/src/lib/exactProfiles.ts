// Exact-mode inventory profiles (spec §9.4): named saved inventories in
// localStorage + export/import to file. A profile is just a labelled list of
// ExactStack records.

import type { ExactStack } from "./types";

const LS_KEY = "deckfast.exactProfiles.v1";

export interface ExactProfile {
  name:   string;
  stacks: ExactStack[];
  savedAt: number;
}

function loadAll(): Record<string, ExactProfile> {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return {};
    return JSON.parse(raw) as Record<string, ExactProfile>;
  } catch (e) {
    console.error("[exactProfiles] failed to parse localStorage — starting empty:", e);
    return {};
  }
}

function persistAll(all: Record<string, ExactProfile>): void {
  localStorage.setItem(LS_KEY, JSON.stringify(all));
}

export function listProfiles(): ExactProfile[] {
  return Object.values(loadAll()).sort((a, b) => a.name.localeCompare(b.name));
}

export function saveProfile(name: string, stacks: ExactStack[]): void {
  const all = loadAll();
  all[name] = {
    name,
    stacks: stacks.map((s) => ({ ...s, groups: [...s.groups] })),
    savedAt: Date.now(),
  };
  persistAll(all);
}

export function loadProfile(name: string): ExactStack[] | null {
  const p = loadAll()[name];
  if (!p) return null;
  return p.stacks.map((s) => ({ ...s, groups: [...s.groups] }));
}

export function deleteProfile(name: string): void {
  const all = loadAll();
  delete all[name];
  persistAll(all);
}

/** Serialize one profile for file export. */
export function exportProfile(name: string, stacks: ExactStack[]): string {
  return JSON.stringify({ deckfastExactProfile: 1, name, stacks }, null, 2);
}

/** Parse an imported profile file. Throws with a readable message. */
export function importProfile(text: string): { name: string; stacks: ExactStack[] } {
  const data = JSON.parse(text);
  if (!data || data.deckfastExactProfile !== 1 || !Array.isArray(data.stacks)) {
    throw new Error("Not a DeckFAST exact-inventory profile file.");
  }
  return { name: String(data.name ?? "imported"), stacks: data.stacks as ExactStack[] };
}

/** Stack identity for ×N stacking in the panel + builder dedup. */
export function stackIdentity(s: ExactStack): string {
  return `${s.t}|${s.color}|${s.scaleColor}|${[...s.groups].sort().join(",")}`;
}
