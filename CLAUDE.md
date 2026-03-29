# CLAUDE.md

## Project Overview

rlr-tools is a Rust project for Rocket League replay analysis, bot detection, and cryptographic verification. It parses `.replay` binary files and exposes analysis via both a CLI and a web server.

## Tech Stack

- **Rust** (2024 edition)
- **boxcars** — Rocket League replay parser (parses network data)
- **axum** + **tower-http** — Web server with REST API, file serving, CORS
- **tokio** — Async runtime
- **ed25519-dalek** + **fips204** — Hybrid Ed25519 + ML-DSA-65 signing
- **sha3** — SHA3-256 hashing for Merkle tree
- **clap** — CLI argument parsing with subcommands
- **serde** / **serde_json** — Serialization

## Project Structure

```
src/
  main.rs               — Entry point; CLI menu + subcommand routing (cli / serve)
  parser.rs             — Replay parsing via boxcars with JSON caching
  demystify.rs          — Human-readable summaries (overview, players, stats)
  bot_detection.rs      — Composite bot scoring (analog + discrete paths, platform weighting)
  input_synchrony.rs    — Timing-based bot detection for keyboard/discrete-input players
  kickoff_analysis.rs   — Per-kickoff reaction timing, pre-hold detection, sequence variability
  boost_analysis.rs     — Boost tracking: avg level, time at zero/full, pad pickups, consumption
  rotation_analysis.rs  — Team rotation: double commits, ball-chasing, teammate distance, back-post
  dribble_analysis.rs   — Ground dribble detection: micro-corrections, zero-steer, opponent-timed flicks
  merkle.rs             — Merkle tree construction, hybrid Ed25519 + ML-DSA-65 signing, .sig sidecar
  web.rs                — Axum web server with REST API endpoints for all analysis types
static/
  index.html            — Single-file HTML/CSS/JS web frontend (replay browser, upload, analysis UI)
assets/
  replays/              — Sample .replay files organized by category subdirectories
  replays/uploads/      — User-uploaded replays via web UI
parsed_games/           — Cached JSON output from parsed replays (gitignored)
analysis/
  bot-replays.md        — Case study of suspected bot replays
```

## Building & Running

```sh
cargo build
cargo run               # Interactive CLI (default)
cargo run -- serve      # Web server on port 3000
cargo run -- serve -p 8080  # Custom port
```

## CLI Menu

After selecting a category and replay, the CLI parses and displays game overview + stats, then offers:

- `[s]` Sign replay (generate .sig sidecar with hybrid Ed25519 + ML-DSA-65)
- `[v]` Verify existing signature
- `[b]` Bot detection analysis
- `[i]` Input synchrony analysis (keyboard player timing)
- `[k]` Kickoff analysis
- `[o]` Boost analysis
- `[r]` Rotation analysis
- `[d]` Dribble analysis

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

## Core Parsing

- `parser::run_cached()` — Parse with cache; `parser::run()` — parse without cache
- `parser::parse_from_bytes()` — Parse from raw bytes (used by web upload)
- Cache stored as `parsed_games/<name>.json`

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

## Conventions

- Keep parsing logic in `parser.rs`; add new features as separate modules
- Each analysis module follows the pattern: `analyze()`, `print_report()`, `results_to_json()`
- Use `Box<dyn error::Error>` for error propagation in public functions
- Network data parsing is always enabled (`must_parse_network_data()`)
- See `METHODOLOGY.md` for detailed algorithm documentation
- See `ROADMAP.md` for planned features
