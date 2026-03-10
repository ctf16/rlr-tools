# rlr-tools

Rocket League replay verification and analysis tools. Parses `.replay` binary files and provides interactive analysis including bot detection, kickoff behavior, boost usage, and cryptographic replay verification.

## Prerequisites

- **Rust** (1.85+ / 2024 edition) — install via [rustup](https://rustup.rs/)
- `cargo` is included with the Rust toolchain and used for building and running.

### macOS / Linux

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Windows

1. Download and run [rustup-init.exe](https://win.rustup.rs/x86_64).
2. When prompted, install the default toolchain.
3. You will also need the [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) — select the "Desktop development with C++" workload during installation.
4. After installation, open a new terminal (Command Prompt or PowerShell) and verify with `rustc --version`.

## Usage

```sh
# Build the project
cargo build

# Run the interactive replay selector
cargo run

# Run in release mode (recommended for large replays)
cargo run --release
```

### Interactive Menu

On launch, rlr-tools presents an interactive menu:

1. **Select a category** — Replays are organized into subdirectories under `assets/replays/` (e.g., `good/`, `suspect/`). Each subdirectory is a category.
2. **Select a replay** — Pick a `.replay` file from the category. A `[✓]` marker indicates the replay has already been parsed and cached.
3. **View summary** — After parsing, the tool displays a game overview (mode, score, duration, forfeit status), player list, and scoreboard.
4. **Choose an action** — After the summary, select from:
   - `[s]` **Sign** — Generate a Merkle tree and Ed25519 signature (saved as a `.sig` sidecar)
   - `[v]` **Verify** — Verify an existing `.sig` sidecar against the replay
   - `[b]` **Bot detection** — Analyze player inputs for bot-like patterns
   - `[k]` **Kickoff analysis** — Measure per-player kickoff reaction times and consistency
   - `[o]` **Boost analysis** — Track boost levels, pad pickups, and consumption per player
   - `[r]` **Rotation analysis** — Evaluate team rotation quality: double commits, ball-chasing, spacing, offensive momentum, and back-post rotation

### Replay Files

Place `.replay` files in subdirectories under `assets/replays/`. Parsed JSON output is cached in `parsed_games/` to avoid redundant parsing on subsequent runs.

## Features

### Replay Parsing

Uses the [boxcars](https://github.com/nickbabcock/boxcars) crate to parse `.replay` binaries into full JSON, including all network frame data (player inputs, physics state, game events). Parsed results are cached as `parsed_games/<name>.json`.

### Game Summary

Extracts match metadata directly from the replay header properties:

- **Game overview** — Game mode (Soccar, Hoops, Rumble, Dropshot, Snow Day), team size, date, final score, forfeit/unfair team detection, match duration.
- **Player list** — All players grouped by team, with platform (Steam, Epic, etc.) and bot flag.
- **Scoreboard** — Per-player score, goals, assists, saves, and shots.

### Replay Verification

Cryptographic integrity verification using a Merkle tree and Ed25519 signatures:

1. The parsed replay JSON is split into 6 semantic sections (header, match metadata, goals, player stats, network frames, content/indices).
2. Each section is hashed with SHA3-256 to form the Merkle tree leaves.
3. Leaves are combined pairwise up to a single root hash.
4. The root is signed with a freshly generated Ed25519 key pair.
5. The public key, signature, and full Merkle tree are stored in a `.sig` sidecar file.

Verification re-hashes the replay sections and checks both the Merkle leaf hashes (pinpointing which section was modified, if any) and the Ed25519 signature.

### Bot Detection

Produces a composite bot score (0.0–1.0) per player by combining multiple independent signals:

- **Input diversity** — Counts unique steer and throttle byte values across all network frames. Human analog stick input produces 100+ distinct values; bots and simple scripts produce very few (often only 0, 128, 255).
- **Platform weighting** — Adjusts the score based on player platform (Epic 1.0x, Steam 0.75x, other 0.85x), reflecting observed cheater distribution.
- **Kickoff pre-hold** — Detects if a player was already holding throttle before the countdown finished (a human habit), applying a 0.4x reduction to the bot score.
- **Kickoff reaction consistency** — Measures the standard deviation of reaction times across kickoffs. Near-zero variance (< 1 frame stddev) boosts the score by 1.5x; humans naturally vary.

Final verdict: **Bot** (>= 0.9), **Likely Bot** (>= 0.5), or **Human** (< 0.5).

See [METHODOLOGY.md](METHODOLOGY.md) for full scoring thresholds and formulas.

### Kickoff Analysis

Analyzes player behavior during every kickoff in the match:

- **Kickoff detection** — Identified by the round countdown number transitioning to 0. The analysis window runs until the ball is hit or 200 frames elapse.
- **Reaction latency** — Frames from countdown=0 to the player's first non-neutral throttle input, reported per-kickoff and as mean/stddev.
- **Pre-hold detection** — Counts how many kickoffs the player was already on throttle at frame 0.
- **Input variability** — Measures how consistently a player repeats the same steer/throttle sequence across kickoffs using average pairwise normalized distance (0.0 = identical every time, 1.0 = completely different).

### Boost Analysis

Tracks each player's boost level across the entire match via network frame data:

- **Average boost** — Mean boost level as a percentage.
- **Time at zero / full** — Percentage of samples spent at 0% or 100% boost.
- **Boost collected / consumed** — Total boost gained and spent over the match.
- **Pad pickups** — Big pad and small pad pickup counts, detected via `grant_count` increments (newer replays) or jump-size heuristics (older replays).

Supports both the newer `ReplicatedBoost` format (with grant count) and the older `ReplicatedBoostAmount` byte format.

### Rotation Analysis

Evaluates team rotation quality in 2v2 and 3v3 matches by extracting per-frame car and ball positions from `RigidBody` network frame data. This is the first module to use positional data — existing modules only use inputs and boost.

Five metrics are computed per player/team:

- **Double Commits** — Detects when two teammates simultaneously rush the ball (both within 15 RB units and approaching). Events are deduplicated with a 120-frame (~1 second) cooldown per pair.
- **Ball-Chasing %** — Percentage of frames where a player (who is not the closest teammate to the ball) is within 20 RB units and actively approaching it.
- **Average Teammate Distance** — Mean pairwise 2D distance between all teammates each frame. Low values indicate clumping (poor rotation); high values indicate good field spacing.
- **Offensive Momentum** — Tracks time in offensive/defensive half per player, and counts sustained upfield pushes (30+ consecutive frames moving toward the opponent's goal while in the offensive half).
- **Back-Post Rotation %** — When a player is retreating in the defensive zone (within 17 RB units of their own goal), checks whether they rotate to the far post (opposite side from the ball). Higher percentages indicate better defensive positioning habits.

The report includes a summary table, a chronological double-commit event list, and a per-minute breakdown showing how rotation quality evolves over the match.

See [METHODOLOGY.md](METHODOLOGY.md) for coordinate system details, thresholds, and formulas.

### Roadmap

See [ROADMAP.md](ROADMAP.md) for the full feature roadmap, including completed features and upcoming work. Highlights of what's next:

- Improved bot detection with input entropy scoring, steer alternation rate, and keyboard-aware scoring paths
- New behavioral signals: dodge timing consistency, post-impact recovery, boost tap variance
- Match analysis additions: goal sequence analysis, demolition tracking, speed/supersonic metrics
- Infrastructure: batch analysis mode, replay comparison, training data export, web API

See [CONTRIBUTING.md](CONTRIBUTING.md) for how to get involved.

## Project Structure

```
src/
  main.rs              — Entry point; interactive replay selector and menu
  parser.rs            — Replay parsing with JSON caching
  demystify.rs         — Human-readable summaries (overview, players, stats)
  bot_detection.rs     — Input diversity and composite bot scoring
  kickoff_analysis.rs  — Per-kickoff reaction timing and consistency
  boost_analysis.rs    — Boost level tracking and pad pickup detection
  rotation_analysis.rs — Team rotation metrics from positional data
  merkle.rs            — Merkle tree construction, Ed25519 signing, sidecar files
assets/
  replays/             — Sample .replay files organized by category
parsed_games/          — Cached JSON output (gitignored)
```

## Methodology

For detailed documentation of the analysis algorithms, scoring thresholds, and data extraction techniques, see [METHODOLOGY.md](METHODOLOGY.md).
