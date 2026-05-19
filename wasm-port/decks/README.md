# Decks

The optimizer scans this directory at startup and loads every deck it finds.
Two formats are supported:

| Format    | Best for                                                       |
| --------- | -------------------------------------------------------------- |
| `*.yaml`  | Hand-curated decks; per-deck overrides (`enabled`, `min_regular`, `max_greed`). |
| `*.json`  | Batch import in the game-data format (one file → many decks). |

Files of both kinds may coexist. See [Collision rules](#collision-rules)
below for what happens when a YAML and a JSON entry refer to the same
deck.

## Layout grid characters

Both formats use the same 3-character grid:

| Char  | Meaning                                                            |
| :---: | ------------------------------------------------------------------ |
| `O`   | Regular slot — the optimizer places a card here.                   |
| `A`   | Arcane slot — counted toward `n_arcane`, but never placed on.      |
| `X`   | Empty / wall (any other character is also treated as empty).       |

Absolute row/column indices are arbitrary; only the relative shape matters.
Trailing blank lines are stripped, and rows can be different widths.

## YAML schema

```yaml
name: My Deck             # required
enabled: true             # optional, default true. Set false to skip.

core_slots: 2             # required. Base in-game core slots; `deckmod` from
                          # config.yaml is added at load time.

min_regular: -1           # optional, default -1 (no minimum).
max_greed:   -1           # optional, default -1 (no maximum). Both are
                          # ignored when testing.full_panel: true (the panel
                          # configs override them).

layout: |                 # required. See "Layout grid characters" above.
  XXAXAXAXX               # 3 arcanes
  XOOOOOOOX               # 7 regulars
  XOOOOOOOX               # 7 regulars
  OOOOOOOOO               # 9 regulars
```


## JSON schema

Drop a game-data dump (e.g. `wolds_decks.json`) into this
directory and every deck inside is imported. Only three fields per entry
are read:

```jsonc
{
  "values": {
    "<key>": {
      "name": "The Cake Deck",          // → Deck.name
      "layout": [
        { "value": [                    // → Deck layout
          "XXAXAXAXX",
          "XOOOOOOOX",
          "XOOOOOOOX",
          "OOOOOOOOO"
        ]}
      ],
      "socketCount": { "max": 2 }       // → core_slots
    }
  }
}
```

Other fields (`model`, `essence`, `weight`, `socketCount.min`, …) are
ignored. Entries whose `socketCount` is `null` (dungeon-only variants) are
skipped automatically. Multi-variant `layout` arrays only use the first
entry — re-export the JSON if you need a specific variant.

JSON-imported decks always use `enabled: true`, `min_regular: -1`, and
`max_greed: -1`. To override any of those, copy the entry into a YAML file
of its own.

## Collision rules

When YAML and JSON files coexist, YAMLs always load first and a JSON entry
is dropped if it refers to the same deck. "Same deck" is decided by
matching the JSON `<key>` against the YAML filename (its stem with any
leading `NN_` prefix stripped) — **not** the human-readable `name` field.

The dedup key is derived as follows:

| YAML filename            | Dedup key | Overrides JSON entry      |
| ------------------------ | --------- | ------------------------- |
| `cake.yaml`              | `cake`    | `values.cake`             |
| `01_cake.yaml`           | `cake`    | `values.cake`             |
| `42_my_custom_deck.yaml` | `my_custom_deck` | `values.my_custom_deck` |
| `moon.yaml`              | `moon`    | `values.moon` (if any)    |

So `decks/01_cake.yaml` will silently override `values.cake` from any
JSON dump in this directory, even if the YAML's `name:` field is "Cake
Deck" and the JSON's `name` is "The Cake Deck". Pick a YAML filename that
matches the upstream JSON key when you want to override; pick a different
filename (e.g. `fred.yaml`, `shard.yaml`, `moon.yaml`) when you want a
custom deck that won't collide with anything in the JSON.

JSON entries that have no matching YAML are loaded as-is. JSON entries
whose `socketCount` is `null` (dungeon-only variants like `gdungeon`,
`ldungeon`, `lost`, `odungeon`) are always skipped, regardless of YAML.

### Excluding decks globally

To skip a deck from both YAML *and* JSON without deleting any files, list
its key under `excluded_decks` in [`../config.yaml`](../config.yaml):

```yaml
excluded_decks: ["arcane", "wold", "fairy"]
```

The same key derivation as above applies — strip any leading `NN_` from
YAML filenames, and use the raw `values.<key>` for JSON.

## Tips

- **Disable instead of deleting** while iterating on YAML decks: set
  `enabled: false` to keep the file around without including it in the run.
- **Filename ordering**: files are loaded in lexicographic order, which is
  why the existing YAMLs use the `01_`, `02_`, … prefix. The prefix is
  cosmetic — only `name` appears in reports.
- **Constraint vs. panel**: per-deck `min_regular` / `max_greed` only
  matter when `testing.full_panel` is `false` in
  [`config.yaml`](../config.yaml). With the full panel on, every deck is
  run through the same `panel_configs`.
- **Core slots**: pick the in-game base value; the loader applies
  `deckmod` for you.
- **Arcane geometry doesn't affect optimization** — only the *count* of
  `A`s matters (used for PURE-core scaling). Place them wherever the
  in-game art shows them, or anywhere convenient if you don't know.
