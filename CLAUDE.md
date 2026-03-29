# CLAUDE.md

## Project Overview

rlr-tools is a Rust project for Rocket League replay analysis and verification. It parses `.replay` binary files and provides interactive CLI and web-based analysis including bot detection, kickoff/boost/rotation/dribble analysis, and cryptographic replay signing.

## Tech Stack

- **Rust** (2024 edition)
- **boxcars** — Rocket League replay parser (parses network data)
- **axum** + **tower-http** — Web server with REST API, file serving, CORS
- **tokio** — Async runtime
- **clap** — CLI argument parsing with subcommands
- **serde** / **serde_json** — Serialization
- **ed25519-dalek** + **fips204** — Hybrid Ed25519 + ML-DSA-65 signing
- **sha3** — SHA3-256 hashing for Merkle tree

## Project Structure

```
src/
  main.rs              — Entry point; CLI menu + web server subcommands
  parser.rs            — Replay parsing with caching (parsed JSON stored in parsed_games/)
  demystify.rs         — Human-readable summaries from parsed JSON (overview, players, stats)
  bot_detection.rs     — Composite bot scoring (analog + discrete paths, platform weighting)
  input_synchrony.rs   — Timing-based bot detection for keyboard/discrete-input players
  kickoff_analysis.rs  — Per-kickoff reaction timing, pre-hold detection, sequence variability
  boost_analysis.rs    — Boost tracking: avg level, time at zero/full, pad pickups, consumption
  rotation_analysis.rs — Team rotation: double commits, ball-chasing, teammate distance, back-post
  dribble_analysis.rs  — Ground dribble detection: micro-corrections, zero-steer, opponent-timed flicks
  merkle.rs            — Merkle tree construction, hybrid Ed25519 + ML-DSA-65 signing, .sig sidecar
  web.rs               — Axum web server & REST API handlers
static/
  index.html           — Single-file web UI (HTML + CSS + vanilla JS)
assets/
  replays/             — Organized by category: good/, bad/, bots/, kbm/, partial/, uploads/
parsed_games/          — Cached JSON output from parsed replays (gitignored)
analysis/
  bot-replays.md       — Case study of suspected bot replays
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

## Web API Endpoints

All under `/api/replays`:
- `GET /` — List replay categories
- `GET /{category}` — List replays in category
- `POST /upload` — Upload a replay (20MB max, goes to `assets/replays/uploads/`)
- `GET /{category}/{name}/bot-detection` — Bot detection analysis
- `GET /{category}/{name}/input-synchrony` — Input synchrony analysis
- `GET /{category}/{name}/kickoff` — Kickoff analysis
- `GET /{category}/{name}/boost` — Boost analysis
- `GET /{category}/{name}/rotation` — Rotation analysis
- `GET /{category}/{name}/dribble` — Dribble analysis
- `POST /{category}/{name}/sign` — Generate .sig sidecar
- `GET /{category}/{name}/verify` — Verify existing signature

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

## Bot Detection Architecture

Two scoring paths based on input type:
- **Analog path** — For controller players: input diversity, unique value count, platform weighting
- **Discrete/timing path** — For keyboard players: uses `input_synchrony` module (alternation rate, hold duration variance, multi-input synchrony), with a +0.15 discrete floor

Key fields in `BotDetectionResult`: `steer_only_discrete`, `throttle_only_discrete`, `used_timing_path`, `timing_detail`, `discrete_kickoff_similarity`

Each analysis module exposes: `analyze(&Value) -> Result<Vec<T>>`, `print_report(&[T])`, `results_to_json(&[T]) -> Value`

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
- Each analysis module follows the pattern: `analyze()`, `print_report()`, `results_to_json()`
- Use `Box<dyn error::Error>` for error propagation in public functions
- Network data parsing is always enabled (`must_parse_network_data()`)
- See `METHODOLOGY.md` for detailed algorithm documentation
- See `ROADMAP.md` for planned features
