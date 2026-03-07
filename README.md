# rlr-tools
Rocket League replay verification and analysis tools.

---

## Usage

```sh
# Build the project
cargo build

# Run the interactive replay selector
cargo run

# Run in release mode
cargo run --release
```

Replays should be placed in `assets/replays/`. Parsed output is cached as JSON in `parsed_games/`.

## Features

### Replay Verifier
- Parses the `.replay` binary
- Hashes each node into a Merkle tree
- Signs the root with Ed25519
- Stores verification in a `.sig` file

### Bot Detection (WIP)
- Detect anomalies in player input patterns
  - Rapid, alternating full-steer inputs in a short time
  - Repeated inputs from 0 to a precise input level

### Planned

- **Replay Diffing** — Compare two replays side-by-side to highlight differences in positioning, boost usage, and decision-making
- **Player Heatmaps** — Extract positional data from network frames to generate per-player heatmaps on the field
- **Boost Analysis** — Track boost pad pickups, boost consumption rate, and time spent at zero boost per player
- **Rotation Metrics** — Analyze team rotation patterns and flag breakdowns (double commits, ball-chasing)
- **Match Timeline Export** — Generate a structured timeline of key events (goals, demos, saves, boost steals) for use in external tools or visualizations
