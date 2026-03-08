# Methodology

This document describes the analytical methods used by rlr-tools to extract and analyze data from Rocket League replay files.

## Replay Parsing and Data Extraction

Replay files (`.replay`) are binary files produced by Rocket League's Unreal Engine networking layer. rlr-tools uses the [boxcars](https://github.com/nickbabcock/boxcars) crate to parse them with `must_parse_network_data()` enabled, producing a full JSON representation including all network frame data.

### Object ID Resolution

The parsed JSON contains an `objects` array — an ordered list of strings naming every replicated property type in the replay. Each network frame update references properties by their index in this array (the "object ID"). All analysis modules resolve human-readable property names (e.g., `"TAGame.Vehicle_TA:ReplicatedSteer"`) to their numeric object ID at startup, then use those IDs to efficiently filter frame updates.

### Actor Linkage

Rocket League's replication system uses separate actor IDs for players, cars, and components. To attribute data to a specific player, the tools build lookup maps by scanning frame updates:

1. **Player names** — `Engine.PlayerReplicationInfo:PlayerName` updates map a player actor ID to their display name.
2. **Car-to-player** — `Engine.Pawn:PlayerReplicationInfo` updates map a car actor ID to the player actor ID that owns it.
3. **Component-to-car** — For boost analysis, `TAGame.CarComponent_TA:Vehicle` updates map a boost component actor ID to its parent car actor ID.

These maps are built in a first pass (or inline during a single pass) and used to resolve all subsequent per-player data.

### Header Properties

Match-level metadata (scores, team size, date, player stats, goals) lives in `properties` at the top level of the parsed JSON. The `demystify` module reads these directly — no network frame processing is needed for game overview, player lists, or scoreboard stats.

## Bot Detection

Bot detection produces a composite score (0.0–1.0) for each player based on multiple independent signals. The signals are multiplied together to produce a final `bot_score`, which maps to a verdict: **Bot** (>= 0.9), **Likely Bot** (>= 0.5), or **Human** (< 0.5).

### Input Diversity Analysis

The primary signal. Human players using analog sticks produce a wide range of values (100+ distinct values across the 0–255 byte range) for both steering and throttle. Bots and keyboard players produce far fewer distinct values.

The analysis collects every `ReplicatedSteer` and `ReplicatedThrottle` update per player across all network frames, then counts the number of unique byte values observed for each.

Scoring (using the lower of steer/throttle unique counts):

| Condition | Input Score |
|---|---|
| Both steer and throttle use only discrete values (0, 128, 255) | 1.0 |
| <= 10 unique values | 0.9 |
| <= 50 unique values | 0.75 |
| <= 75 unique values | 0.6 |
| <= 100 unique values | 0.4 |
| > 100 unique values | 0.0 |

A minimum of 10 steer samples is required; below that the input score defaults to 0.0 (insufficient data).

### Platform Multiplier

Empirically, the vast majority of cheaters play on Epic. The platform multiplier scales the bot score based on platform:

| Platform | Multiplier |
|---|---|
| Epic | 1.0x |
| Steam | 0.75x |
| Other | 0.85x |

Platform is extracted from `properties.PlayerStats[].Platform.value`.

### Kickoff Behavior (Pre-Hold)

If a player is already holding throttle (value 255) on the very first frame of a kickoff countdown (frame offset 0), that's a strong human signal — humans often pre-hold the gas in anticipation. Each such detection applies a **0.4x** multiplier to the bot score, significantly reducing it.

### Kickoff Reaction Consistency

Bots tend to react to kickoff countdowns with near-identical timing across all kickoffs. Humans naturally vary. The reaction standard deviation (in frames) across 3+ kickoffs is used as a multiplier:

| Reaction Stddev (frames) | Multiplier |
|---|---|
| < 1.0 | 1.5x (strong bot signal) |
| < 3.0 | 1.3x |
| < 5.0 | 1.1x |
| >= 5.0 | 1.0x (no effect) |

### Final Score

```
bot_score = min(input_score * platform_mult * pre_hold_mult * kickoff_consistency_mult, 1.0)
```

## Kickoff Analysis

Kickoff analysis identifies every kickoff event in the match and measures per-player behavior during each one.

### Kickoff Detection

A kickoff is detected when `TAGame.GameEvent_TA:ReplicatedRoundCountDownNumber` transitions to 0. The kickoff window extends from that frame until either `TAGame.GameEvent_Soccar_TA:bBallHasBeenHit` becomes true or a cap of 200 frames is reached, whichever comes first.

### Metrics Collected

For each player during each kickoff window:

- **Reaction latency** — The frame offset from countdown=0 to the player's first non-neutral throttle value (anything other than 128). Measured in frames.
- **Pre-hold detection** — If the first throttle update is at frame offset 0 with value 255, the player was pre-holding gas.
- **Input sequences** — Steer and throttle values are interpolated into per-frame sequences (sparse updates are forward-filled from the last known value, defaulting to 128/neutral).

### Aggregate Statistics

Across all kickoffs for a given player:

- **Mean reaction** — Average reaction latency in frames.
- **Reaction stddev** — Standard deviation of reaction latency (requires 2+ valid measurements).
- **Steer/throttle variability** — Average pairwise normalized distance across all kickoff input sequences. Computed as the mean absolute difference divided by 255, averaged over all pairs. A value near 0.0 means the player does nearly identical things every kickoff; near 1.0 means high variation.

## Boost Analysis

Boost analysis tracks each player's boost level over the entire match by monitoring boost component updates in the network frames.

### Data Sources

Two replay formats are supported:

- **Newer replays** — `TAGame.CarComponent_Boost_TA:ReplicatedBoost` contains both `boost_amount` (0–255) and `grant_count` (increments on each pad pickup).
- **Older replays** — `TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount` provides only a `Byte` value for the boost level.

Boost component actors are linked to cars via `TAGame.CarComponent_TA:Vehicle`, then cars are linked to players through the standard car-to-player mapping.

### Pad Pickup Detection

When `grant_count` is available (newer format):
- **Big pad** — `grant_count` increments and `boost_amount` is 255 (full boost).
- **Small pad** — `grant_count` increments and boost increased by at least 10 units (but didn't reach 255).

When `grant_count` is unavailable (older format), heuristics are used:
- **Big pad** — Boost jumps to 255 with an increase of at least 80 units.
- **Small pad** — Boost increases by at least 10 units.

### Metrics Reported

| Metric | Description |
|---|---|
| Average Boost | Mean boost level as a percentage (0–100%) across all samples |
| At Zero % | Percentage of samples where boost was 0 |
| At Full % | Percentage of samples where boost was 255 |
| Collected | Total boost gained (sum of all positive deltas), as percentage units |
| Consumed | Total boost spent (sum of all negative deltas), as percentage units |
| Big Pads | Number of detected big pad pickups |
| Small Pads | Number of detected small pad pickups |

All percentage values are normalized from the raw 0–255 byte range to 0–100%.
