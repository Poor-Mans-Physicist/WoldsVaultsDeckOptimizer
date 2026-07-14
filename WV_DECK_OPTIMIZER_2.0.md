# WV Deck Optimizer 2.0 — Design Spec

**Status:** Draft for review. Author brain-dump captured + annotated by Claude (Opus 4.8), 2026-07-13.
**Next steps:** Fable reviews → shores up implementation details → test build on a new branch → live compare against current optimizer → push/replace only once the new engine reproduces the current one (on vanilla, no-implicit configs) within tolerance.

> Reading order: §1 is the one idea the whole rewrite hangs on. §2 is the data model. §3–8 are the feature areas. §9 is UI. §10 is performance. **§11 (Open Decisions) is the list of things that still need a human/Fable ruling — read it before implementing.**

---

## 1. Core architectural principle: one kernel, three configurations

The three "modes" (**Max**, **Targeted**, **Exact**) are *not* three optimizers. They are one tag-aware simulated-annealing kernel run with three different **supply models** and **implicit-evaluation strategies**. Keeping this a single kernel is what preserves the current Rust performance and keeps the code maintainable.

| Mode | Supply model | Deck implicit evaluated as | Search space | Relative cost |
|------|--------------|----------------------------|--------------|---------------|
| **Max** | Infinite supply of every card type | Exact, like today: assign the minimal necessary category tag(s) (free — they're inert) then count exactly; foil = ported run-level logic | positional type + greed placement (color-blind unless Complex) | Fastest |
| **Targeted** | Infinite supply, but per-tag **caps** (0 = ban, N = at most N) | Blanket for uncapped tags; per-card assignment for capped implicit-relevant tags | above + tag assignment within caps | Medium |
| **Exact** | Finite multiset the player built (a curated inventory profile) | Per-card, from the real tags on each placed card | placement of a fixed card multiset | Slowest |

**The reduction that makes this cheap (author-confirmed):** the Bucket-B category tags (§2.2) are **NDM-inert**. A card's *base* NDM depends only on **positional type + color (in the color-aware path) + foil** (foil via its Pure-core interaction). Tags like Offensive/Defensive/Utility/Physical/Magical/etc. do nothing to a card's score *except*:
1. as an **input to a deck implicit** (e.g. Belt reads Utility), or
2. as a **constraint** in Targeted (a cap), or
3. the two special cases **foil** (blocks Pure-core contribution) and **wild** (a 0-NDM card that counts as a universal positional-match neighbor — §2.2).

So the optimizer never has to "search over" inert tags. It only searches positional type + color (+ the handful of tags an active implicit/cap actually cares about). This is the key to keeping Targeted/Exact fast.

**Validation gate (hard requirement):** Max, run on a **vanilla** deck with **no implicit**, must reproduce the *current* optimizer's numbers within floating tolerance. This gates the whole rewrite (test-compare before push). See §10 and §11-A for the unification-vs-separate-lineage decision this forces.

---

## 2. Card & tag data model

### 2.1 A card is now
```
Card = {
    type:        CardType     // positional (row/col/surr/diag), greed (4 dirs only — see §2.3),
                              //   deluxe, typeless, arcane
    card_color:  Color        // red / green / blue / yellow   (a card's own color)
    scale_color: Color        // the color its bonus keys off (see §7 Complex Cards).
                              //   == card_color unless Complex Cards is on.
    groups:      TagMask      // bitmask of inert/implicit tags (offensive, defensive, ...)
    foil:        bool         // §5 — an explicit tag now, with rules
}
```
Represent `groups` as a `u32` bitmask in Rust (one bit per tag). `foil` can live in the mask too or as its own bit; keep it addressable because it drives Pure-core math.

### 2.2 Tag taxonomy — **AUTHORITATIVE (extracted from live card data)**
Pulled from `config/the_vault/card/modifiers.json` + `woldsvaults/.../deck_mods/new_cores.json`. **17 distinct groups.** A card carries a *set* of groups — one or more **class markers** plus zero or more **category tags** (they genuinely co-occur, e.g. `['Defensive','Offensive','Stat']`, `['Magical','Offensive','Temporal']`, `['Deluxe','Essence','Stat','Utility']`).

**Bucket A — class / structural markers** (derived from the card's type / run mode / overlay; the optimizer already knows these — they are **not** freely assigned or notched):
- `Stat` — gives a raw stat. **Shiny-only (author-confirmed):** appears on shiny stat cards, **never on normal evolution cards** (the sole Evolution+Stat combo in the data is the Wild card, which counts as everything). Consequence: `treasure` (+100% Stat) and `mutant` do **nothing on evo runs** — only shiny runs. Also not universal even among shinies (Utility/Resource/Knack cards can lack it).
- `Greed` — greed card (appears alone; no stat).
- `Deluxe` — deluxe overlay (enhanced card; drives the deluxe core). Overlay, like Foil.
- `Shiny` / `Evolution` — the two card classes (= run mode).
- `Arcane` — arcane card (arcane slots). Note arcane cards still carry a category, e.g. `['Arcane','Offensive']`.
- `Foil` — overlay; special (Pure-core interaction). See §5.

**Bucket B — category "special" tags** (the freeform tags implicits read; **NDM-inert except via a deck implicit**; these are what get assigned in Max, capped in Targeted, built in Exact, notched, and edited in the click popup):
- `Offensive`, `Defensive`, `Physical`, `Magical`, `Utility`, `Resource`, `Knack`, `Temporal`, `Essence`.

**`Wild` — special card, NOT inert (author-confirmed).** The Wild card contributes **0 NDM itself** but **counts as any group — and, in our color-keyed positional model, any color — for positional-scaling purposes**. It's a universal wildcard *neighbor*: any positional card counts a Wild card as a match, regardless of that card's color/group. Behaviorally it's ARCANE-like (0 self-score, boosts neighbors' positional counts) but a universal match rather than a fixed color. **This makes Wild a second NDM-relevant exception alongside Foil — model it as a special card type, not an inert tag.** (In the data it's one card carrying ~9 groups at once, incl. the only Evolution+Stat combo.)

Author-confirmed: **the Bucket B category tags have no NDM effect except through a deck implicit. The two exceptions are Foil** (blocks Pure-core contribution) **and Wild** (0-NDM universal positional-match card). Stated fact, not assumption.

### 2.3 Card-type vocabulary changes
- **Disable as selectable / non-real:** `SURR_GREED`, `EVO_GREED`, and all four diagonal greeds (`DIR_GREED_NE/NW/SE/SW`). Rationale: not obtainable as real cards. **Real greed = the 4 orthogonal directions only** (`UP/DOWN/LEFT/RIGHT`). NOTE: this changes current seeding defaults (today `SURR_GREED` is the default greed and `SURR` the default positional in `lib.rs`); the initial-fill logic must be re-pointed. (`FILLER_GREED` was already display-only.)
- **`DEAD` is not a selectable card type.** "Dead" = empty slot, which is the natural default state of any slot on the deck. It is **always permitted** by the optimizer and **cannot be restricted or capped** in any mode. It never appears as a buildable/selectable card.
- Positional (`ROW/COL/SURR/DIAG`), `DELUXE`, `TYPELESS`, `ARCANE` unchanged in meaning.

---

## 3. The three modes (behavior)

### 3.1 Max — theoretical ceiling (also the spreadsheet optimizer)
Max works **exactly like today's optimizer** (same positional counting, same Pure `n_ns` math, same greed logic, exact — *not* a blanket flat bump), with one addition: it knows each deck's implicit and **assigns the minimal necessary category tags** to make that implicit fire.

- **Unlimited supply of every card type/color.** Tag assignment is free because category tags are NDM-inert (§2.2): to reduce the search space, Max simply gives cards whichever category tag(s) an active implicit rewards, and **defaults to zero special tags when none help** (e.g. Rook is boosted by positional frequency, not by any category tag → 0 tags).
  - If a slot's card needs a tag to earn the implicit bonus, the read-out records that tag as **required** for that slot (any real card placed there must carry it; extra tags are harmless). This drives the Max side-panel read-out and tells the player what cards to actually build.
  - Example: Belt (+200% Utility) → every card is marked Utility and gets `+2.0` in the additive core sum. Because Utility is inert and free, "assign it to all" is exact and optimal — it is *not* an approximation.
- **Positional-logic implicits** (Cake, Rook/Pillager/Bishop, Merchant, Skull, Puzzle, Snake, Shadow, Mutant, Runic) run their real placement logic, loaded once at start (if/else on the active implicit), under the favorable-tag/color assumption but with **exact position counting**.
- **Foil is not a per-card choice Max makes.** It is set by the run config (shiny ⇒ foil; evo ⇒ foil iff Foil core) via the existing foil logic, ported into the tag system, plus the §6 final pass. So the Foil-gated implicits (gilded/ornate/living) need **no special Max handling**: their required color is assigned free, and whether their cards are foil follows from the run — if it's not a foil run, the implicit simply yields nothing.
- Color is **not** modeled in positional scaling here (Max stays color-blind like today, matching the validation gate); color-keyed implicits are satisfied by the free favorable-color assignment. Color only enters the search under **Complex Cards** (§7).
- This is the mode the batch **spreadsheet pipeline** uses. Vanilla runs Max with no implicit.

### 3.2 Targeted — Max + tag limits
- Same objective (maximize NDM), but the player can constrain tags:
  - **Ban:** set a tag's max to **0** (e.g. exclude all Offensive; or ban a whole color; or ban a direction/positional type).
  - **Limit:** set a tag's max to **N** (e.g. "at most 2 Offensive", "at most 4 Column").
- **Multi-tag counting:** a card counts toward the cap of *every* tag it has. With caps {Red ≤ 2, Offensive ≤ 2}, legal decks include 2 red + 2 offensive as separate cards, OR 2 cards that are both red *and* offensive, etc.
- Uncapped tags behave as in Max (assigned freely wherever an implicit rewards them). Capped tags that an implicit reads become **assignment variables** (the optimizer decides which slots carry them, within the cap, to maximize NDM).
- Caps apply uniformly to positional types, colors, and groups. `DEAD` is exempt (§2.3).

### 3.3 Exact — real cards
- Player builds the **exact** cards they own: color, type, direction, positional kind, and tags — for both greed and regular cards. Example: "3× (red, offensive, column) + 2× (green, left-greed)."
- The optimizer places from that finite multiset only. No blanket assumptions; real per-card tags drive the implicit.
- **Inventory builder + profiles:** cards are created in a selector/builder (not a side panel of toggles), added to a saved bucket, and **stack** (duplicates show `×2`, and can be batch-created). Multiple named inventory **profiles** can be curated and swapped without re-entering cards (persist to localStorage; export/import to file). See §9.4.

---

## 4. Deck-implicit integration per mode

Implicits should use the **baked-in type system wherever possible** (Max/Targeted) and the non-baked-in per-card path when not (mainly Exact). Mapping of the 28 implicits (from the audit) to how each mode treats them:

| Implicit class | Decks | Max / Targeted (baked) | Exact (per-card) |
|---|---|---|---|
| Conditional flat efficiency (`GlobalDeckModifier`) | treasure, idona, tenos, velara, wendarr, champion, anvil, belt, fairy, cactus, gilded*, ornate*, living* | Flat additive core-base bump on every card (assume condition met). `+value` into the additive core sum. | Per-card: apply `+value` only to cards actually matching group(s)+color. |
| Frequency multiplier | rook, pillager, bishop | Sim-logic hook: multiply matching positional type's neighbor count by `round(value)`; factor into best-positional choice. | Same hook, gated on the real card's positional type. |
| Adjacency per-group | merchant (col/Resource), skull (surr/Knack) | Assume all neighbors carry the group → `+value ×(neighbor count)`. | Count real matching-group neighbors. |
| Color-mismatch adjacency | puzzle | Assume best-case color arrangement (max mismatches). | Count real different-color adjacencies. |
| Row-position | cake | Per-card `+value ×(row distance from bottom)`, all cards. | Same (position-only; tag-independent). |
| Chain (same-color connected) | snake | Assume all same color → chain = whole deck; `+value ×(n−1)` per card. One labeling pass. | Real connected-same-color components. |
| Empty-slot | shadow | `+value ×(empty count)` per card (already have empty-slot count). | Same. |
| Unique-groups | mutant | Stat cards `+value ×(unique groups in deck)`. Under Max, assume max diversity. | Real unique-group count. |
| Arcane-slot boost | villager | Cards in arcane slots `×(1+value)`. | Same. |
| Multiplicative mirror | runic | `×value` when a card's horizontal mirror is same color; multiply, not add. Assume mirror match under Max. | Real mirror-color check. |
| Gameplay-only (NDM-inert) | arcane (ability lvl), extended (duration), relic (crate tier) | **Ignored for NDM** — display note only. | Same. |
| Special: Mystery | mystery | Rolls **two** random implicits — evaluate both; see §11-E for the conflicting-assumption case. | Two real implicits on the built deck. |

`*` gilded/ornate/living require the **Foil** tag. Foil is set by the run config (§5), not chosen per-card by Max, so these need no special Max handling: the required *color* is assigned for free, and if the run isn't a foil run the implicit simply yields nothing. (Earlier drafts wrongly called this a Max "tradeoff" — it isn't.)

---

## 5. Foil, redone as a tag

Foil is currently implicit and invisibly wired (it drives the Foil "shiny" core and blocks Pure-core contribution via `n_ns`). It becomes an explicit tag with these rules:

- **Foil is a tag.** Notch color = **bright white** (the *only* non-positional/non-color tag with a mandated notch color; author picks the rest — §9.5).
- **Shiny cards are ALWAYS foil** and it cannot be removed. The Exact builder must not allow creating a non-foil shiny card.
- **Evo cards are foil only if the Foil core is selected** (that's the whole point of the Foil core).
- **Foil blocks Pure-core contribution** (a foil card is excluded from Pure's `n_ns`). This is the existing behavior; the refactor just makes the exclusion **per-card** (driven by the tag) instead of the current global `is_shiny || foil_active` shortcut.
- **Targeted, foil banned (cap 0):**
  - Every placed evo card is non-foil regardless of the Foil core (and realistically the Foil core shouldn't be picked then).
  - On a **shiny** deck, banning foil yields an **all-dead (empty) deck** — because shiny ⇒ foil is mandatory, so nothing is placeable. (Valid but degenerate; UI should warn — §11-F.)
- **Vanilla exception:** vanilla's typeless stat cards do **not** have to be foil (vanilla "shiny" is positional-off flat typeless). The must-be-foil rule is WV-only.

**Unifying insight:** making foil per-card *is* the same change that powers the §6 final pass — once `n_ns` counts *actual* non-foil cards, non-foil evo cards are valued correctly.

---

## 6. Non-foil-evo final cleanup pass

When a **Foil core** is present (evo run), the SA assumes every placed evo card is foil (so it doesn't count toward Pure's `n_ns`). This simplification keeps the search space small but leaves value on the table: a **non-foil** evo card still contributes to the Pure core.

- **After** the SA has fully settled the layout, do a single final pass: replace any **greed card that is not pointing at a valid stat-bearing card** (a wasted greed) with a **non-foil evo card**. The non-foil evo adds to Pure's `n_ns`; it was skipped during search only because of the all-evo-are-foil simplification.
- This is a cheap deterministic post-step, not part of the annealing loop. It must recompute the final score after substitution.
- Keeping it as a final pass (rather than modeling foil/non-foil evo as separate placeable types during SA) is the performance-conscious choice — it avoids doubling the evo option space. Exact mode, which models foil per-card anyway, does not need this pass.

---

## 7. Complex Cards (scale color ≠ card color)

In the real game a card can scale off / boost a color different from its own — e.g. a **red greed card that boosts GREEN cards** to the left, or a **blue evo card that scales off RED row cards**. This is an optional layer:

- **Toggle** in the optimizer panel: **Complex Cards** (default OFF). Hover explanation + explicit **warning that it significantly slows the optimizer** (it multiplies the per-card option space by up to `card_color × scale_color`).
- **ON:**
  - Exact: the card builder exposes both `card_color` and `scale_color`.
  - Max: the optimizer may try cards whose `scale_color ≠ card_color`.
- **OFF:**
  - `scale_color` is forced `== card_color` everywhere (current behavior).
  - Exact can't create mismatched cards; any already in the inventory are **greyed out and ignored** at optimize time.
- **Semantics to pin down (§11-D):** today greed is **color-agnostic** (it boosts whatever scorable card is in its direction, ignoring color). Under Complex, does a greed card boost *only* its `scale_color`? And does non-Complex preserve today's color-agnostic greed (required for the validation gate)? Proposed default: non-Complex ⇒ greed stays color-agnostic and positional counts own color (current numbers preserved); Complex ⇒ greed boosts only `scale_color`, positional counts `scale_color` neighbors.

---

## 8. Vanilla mode

- **No deck-specific implicits** in vanilla. Everything else applies: the SA-params-panel redo (§9.2), the inventory-panel rebuild (§9.3–9.4), notches, hover/click popups, complex-cards toggle — just with no implicit wiring.
- Vanilla keeps its own roster (`vh_decks.json`) and its multiplicative-core / positional-off / no-deluxe / no-void settings (existing `modes.vanilla` overrides).
- Vanilla typeless stat cards are **not** forced foil (§5).

---

## 9. UI redesign

### 9.1 Overview
Two panels change: the **optimizer-settings** box (replaces SA-params) and the **side inventory panel** (mode-dependent). Plus per-card notches, hover/click popups, and a rebound breakdown gesture. Construction/arcane core placement UI is **unchanged**.

### 9.2 Optimizer-settings box (replaces the SA-params box)
- **Mode slider:** Max / Targeted / Exact. Each selection swaps the optimizer's default settings + the side-panel layout.
- **Depth slider:** Fast / Default / Deep → fixed SA params:
  - Fast = 50 000 iterations, 6 restarts
  - Default = 75 000 iterations, 12 restarts
  - Deep = 125 000 iterations, 24 restarts
  - (Replaces the raw iterations/restarts inputs.)
- **Complex Cards toggle** (§7) with hover explanation + slowdown warning.

### 9.3 Side panel — per mode
- **Max:** read-out of the **placed** cards, **stat-bearing cards listed first**.
- **Targeted:** the list of tags that can be capped, ordered **colors/positional types first, then the freeform tags** (Utility, Essence, …). Each row has a cap input (blank/∞ = unlimited, 0 = ban, N = limit).
- **Exact:** a **+ Add Card** button opens the builder popup (§9.4); the panel shows the current inventory as stacked entries (`×N`), with save/load (profile) controls.

### 9.4 Exact card builder + inventory profiles
- Builder popup: pick color → type → positional/direction → tags (and `scale_color` if Complex is on). Add to inventory; support **batch add** (e.g. "add 5 of this").
- Inventory list: stacks identical cards (`×N`); allows editing counts / removing.
- **Profiles:** named saved inventories; switch between them without re-entering cards. Persist locally; export/import to file.

### 9.5 Per-card notches (tag indicators)
- Every placed card shows small **colored notches** for the tags it carries.
- Notched tags = the **Bucket B category tags** (§2.2) + Foil. Class markers (Stat/Deluxe/Shiny/Evolution/Arcane/Greed) are conveyed by the tile's type rendering, not category notches (Deluxe/Foil are overlays — Foil always notched; Deluxe optionally).
- **Foil = bright white** (mandated). Author picks the rest; must be **distinctive and sensible**. Proposed starting palette (adjust freely):
  - Offensive = crimson `#D7263D` · Defensive = steel blue `#3A6EA5` · Physical = bronze `#B07D3B` · Magical = violet `#7B2FBE` · Utility = teal `#189A8A` · Resource = emerald `#2FA84F` · Knack = amber `#E8A020` · Temporal = sky `#5BC8E8` · Essence = magenta `#D6469E` · Stat = gold `#E8C33B` · Wild = chartreuse `#9BCF3B` · **Foil = white `#FFFFFF`**.
  - (Card *color* red/green/blue/yellow is shown by the tile itself, not a notch, to avoid clashing.)

### 9.6 Interactions
- **Hover** a placed card → popup beside the cursor listing its tags, each in a **bubble colored to its notch color**.
- **Click** a placed card → popup to **add/remove non-restrictive tags** (the stat-derived ones: Essence, Foil*, …) — **not** type, color, or positional. On change, **immediately recompute that card's NDM and the whole deck's NDM** (a score-only sim pass, not a re-anneal) so the player sees the live effect.
  - These edits are an **ephemeral what-if overlay**: they do **not** modify the Exact inventory or the Targeted tag selection, and are **discarded when the simulation is re-run**.
  - `*` Foil is editable only where legal: cannot be removed from a shiny card (§5); the popup must enforce foil rules.
- **Shift + left-click** a placed card → the existing **slot breakdown** view (rebound from plain click).

---

## 10. Performance plan

### 10.1 Kernel hygiene
- **Reuse the inventory kernel's discipline:** caller-allocated scratch buffers (zero-alloc hot path), dense Vec-indexed counters (not hashmaps), rayon-parallel restarts. The abstract `lib.rs` kernel currently allocates ~8 heap objects per `simulate()` and uses hashmap row/col counts — porting Max onto the scratch-buffered path can make it *faster* than today even with tags added.
- **Resolve flexibility at setup, not in the loop.** Parse the active deck implicit(s) + caps into a compact tagged-union once before annealing; the per-card kernel does a small `match` over that with bitmask ops. **Never** interpret a generic rule tree per-card per-iteration.
- **Closed-form implicit bonuses** (§4) keep Max's added cost near zero (a flat addend for conditional-efficiency implicits).
- **Per-call cost budget:** with `n ≈ 30–50` slots, a tag-aware full `simulate()` adds ~1.1–1.4× per-call for a typical single-implicit deck (mostly bitmask checks folded into existing peer scans). Snake must be one labeling pass, not per-card (else O(n²)). Exact is slowest because it can't blanket-assume.

### 10.2 Delta (incremental) evaluation — O(n) → O(affected slots)
Both kernels today call the full `simulate()` (re-partition, re-count, re-accumulate, ~8 allocs — all O(n)) on **every** SA move. Replace that with an incremental evaluator so a single-slot move costs **O(|affected set|)** instead of O(n).

**Decompose the score.** For additive cores, per-card contribution is `contrib_i = w_i · (1 + baseline_sum + gate_terms_i)` with `w_i = base_i · greed_i`, so:
```
NDM = archive_mult · [ (1 + baseline_sum)·A  +  B ]
  A   = Σ_i w_i                                   (scorable slots)
  B   = color_addend·W_color + deluxe_addend·W_deluxe + void_addend·W_void
  W_x = Σ_{i : gate x applies} w_i
```
Maintain `A`, `W_color`, `W_deluxe`, `W_void` (hence `B`) and the global counts (`n_ns`, `n_deluxe`, `n_dead`, `n_arcane`, `row_color[]`, `col_color[]`, empty-slot count) **incrementally** across moves. Then:

1. **Global-aggregate change** (a move flips a card's `n_ns`/`n_dead`/`n_deluxe`/`n_arcane` membership): recompute `baseline_sum` / addends / `archive_mult` (O(1)) and recombine `NDM` from the maintained sums (O(1)). **No per-card loop** — this is the big win, since `n_ns` (Pure) otherwise touches every card.
2. **Local change** (slot p's card changes): rebuild only the **affected set** `S` = p itself + the positional cards whose neighbor-count changed because p's fill/color changed (p's row/col/diag/surr line-mates of the old & new color) + p's greed target (directional greed → ≤1). For each `i ∈ S`: subtract its old `(w_i, gated w_i)` from the running sums, recompute, add back. Cost **O(|S|)**, bounded by deck geometry (row+col+diag+surr span), not total n.

A pair-swap = two single-slot deltas. Keep a full-`simulate()` **oracle** for tests and an occasional re-sync (every K accepted moves) to bound float drift — the delta must match full-sim exactly.

**Implicit-awareness (critical — the delta must know the active implicit):**
- **snake** (color chain): a color change can alter a whole connected same-color component → `S` includes that component (bounded flood-fill).
- **runic** (mirror): p's mirror slot joins `S`.
- **merchant / skull / puzzle** (neighbor group/color reads): the neighbors that read p are already p's peers — already in `S`.
- **mutant** (unique groups) / **shadow** (empty slots): **global aggregates** → handle via the O(1)-recombine path (case 1), like `n_ns`.
- **cake** (row position) / **rook·pillager·bishop** (frequency): position/type-only, no color ripple → no extra `S`.
- **wild** cards (§2.2): a universal positional match → when p becomes/ceases-to-be Wild, every positional neighbor of p is affected (treat like a color change that matches all colors).

**Sequencing & risk.** Build the correct full-`simulate()` engine first and pass the §12 validation gate with it; add the delta layer as **Phase 2**, shipped *behind the oracle check*, because a subtle delta bug silently corrupts scores rather than crashing. Expected win: per-move cost drops from ~n to the perimeter of a slot's influence, plus the elimination of per-move re-partition/alloc and the O(1) handling of the global Pure/void/deluxe/archive aggregates — this is what buys back the tag/implicit cost and then some (more on larger decks).

---

## 11. Open decisions (need a ruling before/at implementation)

- **A. Kernel unification vs. Max lineage.** Ideal: one kernel (inventory-style) for all three modes. But Max must reproduce current numbers (validation gate). Action: empirically check whether the inventory kernel fed a monochrome "ideal" inventory reproduces the abstract `lib.rs` numbers (color-blind positional counts, color-agnostic greed, experimental exponent, FILLER_GREED, void/archive). If yes → unify. If no → keep Max on the abstract lineage and share only the tag machinery. **This is the first thing to settle.**
- **B. Targeted search-space semantics.** Confirm the model in §3.2/§1: inert tags are pure constraints; only implicit-relevant capped tags become assignment variables; uncapped ⇒ blanket. Confirm caps apply to positional types + colors + groups alike, with multi-tag counting.
- **C. Foil implicits in Max — RESOLVED.** Foil is set by run config, not chosen per-card by Max (§3.1/§5). gilded/ornate/living need no special handling: color assigned free, foil follows the run, ported foil logic keeps Pure `n_ns` exact. No "tradeoff." (Kept here only to mark the earlier draft's error as corrected.)
- **D. Complex/greed color semantics.** Pin down greed color behavior on/off Complex (see §7). Requirement: Complex-OFF must reproduce current numbers.
- **E. Mystery deck conflicting assumptions.** Its two random implicits can want opposite favorable configs under Max (e.g. Snake wants all-same-color, Puzzle wants all-different). Decide the resolution: evaluate both and take the better single assumption, or model the interaction. (Low priority — Mystery is one deck.)
- **F. Degenerate configs.** Targeted banning foil on a shiny deck ⇒ empty deck; other cap combinations can be infeasible (e.g. min-regular vs. bans). Decide UI behavior: warn, disable, or return-empty-with-explanation.
- **G. Authoritative tag list — RESOLVED.** Extracted: 17 groups, categorized in §2.2.
- **H. "Wasted greed" definition for §6.** Define precisely "greed not pointing to a valid stat-bearing card" for all 4 directions (and confirm it's only the orthogonal greeds that survive per §2.3).
- **I. Live tag-edit scope.** Confirm the exact set of "non-restrictive" (editable) tags for the click popup, and that foil-rule enforcement lives there.
- **J. `Wild` — RESOLVED.** Confirmed: Wild is its own special card — **0 NDM, counts as any group/color for positional-scaling purposes** (universal wildcard neighbor, ARCANE-like but any-color). Modeled as a special card type and the second NDM exception alongside Foil (§2.2, §10.2).

---

## 12. Validation & rollout

1. New branch off the current optimizer.
2. Build the tag-aware **full-`simulate()`** kernel + three modes + UI (no delta yet).
3. **Live compare** against the current optimizer on GitHub: Max/vanilla/no-implicit must match current output within tolerance; spot-check WV decks.
4. Only once matched → push and make the new one live.
5. Deck-specific implicits ship in tandem, gated so vanilla never receives them.
6. **Phase 2 — delta evaluation (§10.2):** add the incremental evaluator behind the full-`simulate()` oracle; ship only once it matches the oracle exactly across a fuzz sweep of decks/cores/implicits.
