# CLAUDE.md

## Project Overview

rlr-tools is a Rust project for Rocket League replay analysis and verification. It parses `.replay` binary files and provides interactive CLI and web-based analysis including bot detection, kickoff/boost/rotation/dribble analysis, and cryptographic replay signing.

## Tech Stack

- **Rust** (2024 edition)
- **boxcars** — Rocket League replay parser (parses network data)
- **axum** — Async web framework (REST API + static file serving)
- **tokio** — Async runtime
- **clap** — CLI argument parsing (derive)
- **serde / serde_json** — Serialization
- **ed25519-dalek** — Ed25519 signing (classical)
- **fips204** — ML-DSA-65 (NIST post-quantum signing)
- **sha3** — SHA3-256 hashing (Merkle tree)

## Project Structure

```
src/
  main.rs              — Entry point; CLI menu + web server subcommands
  parser.rs            — Replay parsing with caching (parsed JSON stored in parsed_games/)
  demystify.rs         — Human-readable summaries from parsed JSON (overview, players, stats)
  bot_detection.rs     — Composite bot scoring (input diversity, timing, platform, kickoff, dribble)
  input_synchrony.rs   — Timing-based input analysis for discrete/keyboard players
  kickoff_analysis.rs  — Per-kickoff reaction timing & input consistency
  boost_analysis.rs    — Boost level tracking & pad pickup detection
  rotation_analysis.rs — Team rotation metrics (double commits, ball-chasing, spacing)
  dribble_analysis.rs  — Ground dribble detection using 3D ball-car positioning
  merkle.rs            — Merkle tree construction + hybrid Ed25519/ML-DSA-65 signing
  web.rs               — Axum web server & REST API handlers
static/
  index.html           — Single-file web UI (HTML + CSS + vanilla JS)
assets/
  replays/             — Organized by category: good/, bad/, bots/, kbm/, partial/, uploads/
parsed_games/          — Cached JSON output from parsed replays (gitignored)
```

## Building & Running

```sh
cargo build
cargo run                        # Interactive CLI menu
cargo run -- serve --port 3000   # Web server mode
```

## CLI Menu Flow

1. **Category selection** — Browse replay subdirectories (good/, bad/, bots/, kbm/, partial/)
2. **Replay selection** — Pick a `.replay` file (marked with [✓] if cached)
3. **Game summary** — Overview, player list, scoreboard
4. **Action menu:**
   - `[s]` Sign — Merkle tree + hybrid Ed25519/ML-DSA-65 signature (.sig sidecar)
   - `[v]` Verify — Verify signature & detect per-section tampering
   - `[b]` Bot detection — Composite scoring from multiple signals
   - `[i]` Input synchrony — Timing-based input pattern analysis
   - `[k]` Kickoff analysis — Reaction latency & consistency
   - `[o]` Boost analysis — Boost levels, pad pickups, collection/consumption
   - `[r]` Rotation analysis — Double commits, ball-chasing %, spacing
   - `[d]` Dribble analysis — Micro-corrections, zero-steer periods, flicks

## Current Functionality

- Parse `.replay` files into full JSON (including network frame data) via `boxcars::ParserBuilder`
- Cache parsed results as `parsed_games/<name>.json` to avoid redundant parsing
- `parser::run_cached()` — parse with cache; `parser::parse_and_cache_bytes()` — parse from bytes
- **Replay verification** — Merkle tree of 6 semantic sections, signed with Ed25519 + ML-DSA-65, stored as `.sig` sidecar
- **Bot detection** — Composite 0.0–1.0 score combining input diversity, platform weighting, kickoff consistency, dribble mechanics, and timing analysis
- **Kickoff analysis** — Per-kickoff reaction time, pre-hold detection, input variability across kickoffs
- **Boost analysis** — Boost level tracking, big/small pad pickups, time at zero/full boost
- **Rotation analysis** — Double commits, ball-chasing %, teammate spacing, offensive momentum
- **Dribble analysis** — Snappy micro-corrections, zero-steer periods, opponent-timed flicks (3D physics)
- **Web UI** — Replay browser, file upload (max 20 MB), all analyses available via REST API

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

## Architectural Patterns

- **Object ID resolution:** All analysis modules resolve property names to numeric IDs for efficient frame filtering
- **Actor linkage:** Build lookup maps (player names, car-to-player, component-to-car) in a first pass over frames
- **Modular analysis:** Each module produces a result struct and `print_report` function
- **Composite scoring:** Bot detection combines input diversity, timing, platform, kickoff, and dribble signals via weighted multiplication

## Conventions

- Keep parsing logic in `parser.rs`; add new features as separate modules
- Use `Box<dyn error::Error>` for error propagation in public functions
- Network data parsing is always enabled (`must_parse_network_data()`)
- Each analysis module follows the pattern: resolve object IDs → build actor maps → scan frames → produce result struct → print report
