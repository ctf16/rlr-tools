# CLAUDE.md

## Project Overview

rlr-tools is a Rust project for Rocket League replay verification and analysis. It parses `.replay` binary files, with planned features for cryptographic verification and bot detection.

## Tech Stack

- **Rust** (2024 edition)
- **boxcars** — Rocket League replay parser (parses network data)
- **serde_json** — JSON serialization

## Project Structure

```
src/
  main.rs        — Entry point; interactive replay selector
  parser.rs      — Replay parsing with caching (parsed JSON stored in parsed_games/)
  demystify.rs   — Human-readable summaries from parsed JSON (overview, players, stats)
assets/
  replays/       — Sample .replay files for testing
parsed_games/    — Cached JSON output from parsed replays (gitignored)
```

## Building & Running

```sh
cargo build
cargo run        # Parses assets/replays/good/rumble.replay
```

## Current Functionality

- Parse `.replay` files into full JSON (including network frame data) via `boxcars::ParserBuilder`
- Cache parsed results as `parsed_games/<name>.json` to avoid redundant parsing
- `parser::run_cached()` — parse with cache; `parser::run()` — parse without cache

## Parsed JSON Structure

The cached JSON files (produced by boxcars) have this top-level structure:

```
{
  "header_size", "header_crc",
  "major_version", "minor_version", "net_version",
  "game_type",          // e.g. "TAGame.Replay_Soccar_TA"
  "properties": {       // match metadata and stats
    "TeamSize":         int,
    "Team0Score":       int,
    "Team1Score":       int,
    "bForfeit":         bool,
    "UnfairTeamSize":   int,    // non-zero if teams were uneven
    "TotalSecondsPlayed": float,
    "MatchStartEpoch":  string, // unix epoch as string
    "WinningTeam":      int,
    "Date":             string, // "YYYY-MM-DD HH-MM-SS"
    "MapName":          string,
    "MatchType":        string, // e.g. "Online"
    "NumFrames":        int,
    "PlayerName":       string, // name of the recording player
    "Goals": [                  // one entry per goal scored
      { "frame": int, "PlayerName": string, "PlayerTeam": int }
    ],
    "PlayerStats": [            // one entry per player
      {
        "Name":       string,
        "Team":       int,
        "Platform":   { "kind": string, "value": "OnlinePlatform_Steam" },
        "Score":      int,
        "Goals":      int,
        "Assists":    int,
        "Saves":      int,
        "Shots":      int,
        "bBot":       bool,
        "OnlineID":   string,
        "PlayerID":   { "name": "UniqueNetId", "fields": { "Uid": string, ... } }
      }
    ],
    ...
  },
  "content_size", "content_crc",
  "network_frames":   [...],   // per-tick network data (very large)
  "levels":           [...],
  "keyframes":        [...],
  "debug_info":       [...],
  "tick_marks":       [...],
  "packages":         [...],
  "objects":          [...],
  "names":            [...],
  "class_indices":    [...],
  "net_cache":        [...]
}
```

Key notes:
- `properties.PlayerStats` is the main source for player info and scoreboard data
- `properties.Goals` lists goals in chronological order with frame numbers
- `game_type` encodes the game mode (Soccar, Hoops, Rumble, etc.)
- `network_frames` contains the bulk of the data (player inputs, physics, etc.)

## Planned Features

1. **Replay Verifier** — Hash replay nodes into a Merkle tree, sign the root, store as `.sig`
2. **Bot Detection** — Flag suspicious player input patterns (rapid alternating steer, repeated precise inputs)

## Conventions

- Keep parsing logic in `parser.rs`; add new features as separate modules
- Use `Box<dyn error::Error>` for error propagation in public functions
- Network data parsing is always enabled (`must_parse_network_data()`)
