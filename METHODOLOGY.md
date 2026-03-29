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

The analysis collects every `ReplicatedSteer` and `ReplicatedThrottle` update per player across all network frames, then counts the number of unique byte values observed for each. The **average** of the steer and throttle unique counts is used — channel asymmetry is handled by a separate multiplier.

Scoring uses continuous linear interpolation between anchor points:

| avg_unique | Input Score |
|---|---|
| ≤ 10 | 0.90 (ceiling) |
| ~110 | ~0.48 |
| ≥ 220 | 0.05 (floor) |

Special cases:
- Both steer and throttle discrete-only (0, 128, 255): **1.0** (routes to timing path)
- Fewer than 10 steer samples: **0.0** (insufficient data)

### Input Synchrony Analysis (Timing Path)

Discrete-only players (keyboard — all inputs are 0, 128, or 255) cannot be scored by input diversity since they inherently have few unique values. Instead, they are scored via timing-based analysis of their input patterns. The composite timing score replaces the input diversity score as the base, with a +0.15 discrete floor added.

Three sub-metrics are computed, each scored 0.0 (human) to 1.0 (bot) via continuous linear interpolation:

**Alternation rate** — value changes per second, using the higher of steer and throttle. Human keyboard players peak ~2 changes/s; above ~3/s is suspicious.

| Rate | Score |
|---|---|
| ≤ 2.5/s | 0.0 (human) |
| ~7/s | ~0.5 |
| ≥ 12/s | 1.0 (bot) |

**Hold duration CV** — coefficient of variation of hold durations, using the lower (more suspicious) of steer and throttle. Low CV means uniform timing — bot-like.

| CV | Score |
|---|---|
| ≤ 0.05 | 1.0 (bot — uniform timing) |
| ~0.5 | ~0.5 |
| ≥ 1.0 | 0.0 (human — natural variance) |

**Synchrony rate** — fraction of input change frames where both steer and throttle changed on the same frame. Bots often update all channels simultaneously.

| Rate | Score |
|---|---|
| ≤ 0.10 | 0.0 |
| ~0.40 | ~0.5 |
| ≥ 0.70 | 1.0 |

**Composite:**
```
timing_bot_score = 0.45 × alt_score + 0.35 × hold_score + 0.20 × sync_score
base_score = timing_bot_score + 0.15 (discrete floor)
```

### Platform Multiplier

Empirically, the vast majority of cheaters play on Epic. The platform multiplier scales the bot score based on platform:

| Platform | Multiplier |
|---|---|
| Epic | 1.0x |
| Steam | 0.75x |
| Other | 0.85x |

Platform is extracted from `properties.PlayerStats[].Platform.value`.

### Kickoff Behavior (Pre-Hold)

If a player is already holding throttle (value 255) on the very first frame of a kickoff countdown (frame offset 0), that's a human signal — humans often pre-hold the gas in anticipation. The reduction scales linearly with the proportion of kickoffs showing pre-holds:

```
pre_hold_mult = 1.0 - 0.3 × (pre_hold_count / kickoff_count)
```

A single pre-hold in a 5-kickoff game applies a ~6% reduction; consistent pre-holding across all kickoffs applies the maximum 30% reduction.

### Kickoff Reaction Consistency

Bots tend to react to kickoff countdowns with near-identical timing across all kickoffs. Humans naturally vary. The reaction standard deviation (in frames) across 3+ kickoffs is mapped to a multiplier via linear interpolation:

| Reaction Stddev | Multiplier |
|---|---|
| 0.0 | 2.0x (strongest bot signal) |
| ~2.5 | ~1.5x |
| ≥ 5.0 | 1.0x (no effect) |

### Kickoff Sequence Variability

Bots that replay the same input sequence every kickoff produce near-identical steer and throttle sequences. The average pairwise normalized distance (0.0 = identical, 1.0 = maximally different) is computed across all kickoff sequences per player. The more suspicious (lower variability) channel is used. Requires 3+ kickoffs.

| Min Variability | Multiplier |
|---|---|
| 0.0 | 1.5x (identical sequences — strong bot signal) |
| ~0.025 | ~1.25x |
| ≥ 0.05 | 1.0x (normal human variation) |

### Dribble Multiplier

When a player has 2+ qualifying dribbles (≥60 frames of ball carry), the dribble suspicion score feeds a multiplier via linear interpolation:

| Suspicion Score | Multiplier |
|---|---|
| ≤ 0.1 | 1.0x (no effect) |
| ~0.45 | ~1.2x |
| ≥ 0.8 | 1.4x |

### Input Asymmetry Multiplier

Real players have correlated steer/throttle diversity — both high (analog) or both discrete (keyboard). A mismatch where one channel shows genuine analog diversity (>30 unique values) while the other is sparse is suspicious. The ratio `max(steer, throttle) / min(steer, throttle)` is mapped via linear interpolation:

| Asymmetry Ratio | Multiplier |
|---|---|
| ≤ 1.5 | 1.0x (normal) |
| ~3.0 | ~1.4x |
| ≥ 4.5 | 1.8x |

### Final Score

```
bot_score = min(base_score × platform × pre_hold × kickoff_consistency × kickoff_sequence × dribble × asymmetry, 1.0)
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

## Rotation Analysis

Rotation analysis evaluates team rotation quality in 2v2 and 3v3 matches by extracting per-frame car and ball positions from `RigidBody` state updates in the network frames. It is restricted to 2v2 and 3v3 — 1v1 is skipped (no teammates) and 4v4 is unsupported (casual-only, not competitively relevant). Team size is read from `properties.TeamSize`.

### Coordinate System

Rocket League replays use a coordinate system where:

- **Y-axis** is the length of the field (goal-to-goal). Goals are at approximately Y = +/-51.2 RB units.
- **X-axis** is the width of the field (sideline-to-sideline). Sidelines are at approximately X = +/-40.96 RB units.
- RigidBody coordinates are scaled ~1/100 from Unreal engine units.
- **Team 0 (Blue)** defends the **negative Y** end and attacks toward positive Y.
- **Team 1 (Orange)** defends the **positive Y** end and attacks toward negative Y.

### Position Tracking

Position data comes from `TAGame.RBActor_TA:ReplicatedRBState` updates, which contain both `location` (x, y, z) and `linear_velocity` (x, y, z). Only the X and Y components are used (2D projection).

Position updates are sparse — only ~3-4% of frames contain a position update for any given actor. The module carries forward the last-known position for each actor, so all per-frame metric calculations use the most recent position even on frames with no update.

Ball and car actors are identified through `new_actors` entries matching the `Archetypes.Ball.Ball_Default` and `Archetypes.Car.Car_Default` object IDs respectively. The ball actor ID changes on respawn (~every 200 frames), so it is re-tracked whenever a new ball actor appears. Team membership is resolved by mapping `Engine.PlayerReplicationInfo:Team` updates to team actor IDs created from `Archetypes.Teams.Team0` / `Team1`.

### Constants and Thresholds

| Constant | Value | Description |
|---|---|---|
| `FIELD_HALF_LENGTH` | 51.2 | Y-axis distance from center to goal line (RB units) |
| `DOUBLE_COMMIT_RADIUS` | 15.0 | Max distance from ball for both players to count as a double commit |
| `DOUBLE_COMMIT_COOLDOWN` | 120 frames | Minimum gap between double commit events for the same pair (~1 second) |
| `BALL_CHASE_RADIUS` | 20.0 | Max distance from ball to count as "near ball" for chasing detection |
| `DEFENSIVE_ZONE_DEPTH` | 17.0 | Distance from own goal line that defines the defensive zone |
| `MOMENTUM_MIN_FRAMES` | 30 frames | Minimum consecutive upfield frames to count as one momentum event (~0.25 seconds) |
| `FAR_POST_X_MIN` | 5.0 | Minimum lateral offset from center to count as "at a post" |

### Metric 1: Double Commits

A double commit is detected when two teammates are both within `DOUBLE_COMMIT_RADIUS` (15.0 RB units) of the ball **and** both have a positive velocity dot product toward the ball (i.e., both are actively approaching it). The velocity check prevents false positives from two players who happen to be near the ball but moving away.

To avoid counting the same sustained double commit as many events, a cooldown of `DOUBLE_COMMIT_COOLDOWN` (120 frames, ~1 second) is enforced per pair of players. Each event records the frame number, game time, player names, and average distance from ball.

### Metric 2: Ball-Chasing Percentage

For each frame, teammates on a given team are sorted by distance to the ball. The closest player is designated "first man" and is excluded from chasing detection — they are the player who should be engaging the ball. Any other teammate within `BALL_CHASE_RADIUS` (20.0 RB units) of the ball whose velocity has a positive dot product toward the ball is counted as chasing for that frame.

```
ball_chase_pct = frames_chasing / frames_active * 100
```

A high ball-chase percentage indicates a player frequently contests the ball when a teammate is already closer — a hallmark of poor rotation.

### Metric 3: Average Teammate Distance

Each frame, the 2D Euclidean distance is computed between every pair of teammates. These pairwise distances are accumulated and averaged across the entire match to produce a single team-level metric.

```
avg_teammate_distance = sum(pairwise_distances) / count(pairwise_distances)
```

Low values (~10-15 RB units) indicate clumping, where teammates are too close together and cannot cover the field effectively. Higher values (~25-35 RB units) indicate good spacing. This metric is also computed per-minute to show trends over the match.

### Metric 4: Offensive Momentum

Two sub-metrics are reported per player:

**Offensive/Defensive Split:** Each frame, a player's field half is determined based on their Y position relative to midfield (Y = 0). Team 0 is offensive when Y > 0 (opponent's half); Team 1 is offensive when Y < 0.

```
offensive_pct = frames_in_offensive_half / frames_active * 100
defensive_pct = frames_in_defensive_half / frames_active * 100
```

**Momentum Events:** A momentum event is counted when a player spends `MOMENTUM_MIN_FRAMES` (30) consecutive frames moving upfield (positive Y velocity for Team 0, negative for Team 1) while already in the offensive half. This captures sustained attacking pushes rather than momentary drifts across midfield. The counter resets any time the player stops moving upfield or leaves the offensive half.

### Metric 5: Back-Post Rotation Percentage

This metric evaluates defensive positioning habits. It triggers when a player is:
1. Within `DEFENSIVE_ZONE_DEPTH` (17.0 RB units) of their own goal line, **and**
2. Moving toward their own goal (retreating — negative Y velocity for Team 0, positive for Team 1).

When both conditions are met, the frame counts as a "defensive retreat." The module then checks whether the player is rotating to the far post:

- The ball's X position determines which side of the goal is "near post" (same side as ball) and "far post" (opposite side).
- A player is at the far post if `sign(player.x) != sign(ball.x)` and `abs(player.x) >= FAR_POST_X_MIN` (5.0 RB units).

```
back_post_rotation_pct = far_post_retreats / total_retreats * 100
```

Rotating to the far post is a fundamental defensive principle in Rocket League — it gives the retreating player better visibility of the field and a wider angle to make saves. Higher percentages indicate more disciplined defensive rotation.

### Per-Minute Breakdown

All frame-level counters are bucketed into 60-second windows using the frame's `time` field. For each minute, the report shows per-team: average teammate distance, offensive percentage, and ball-chase frame count. This reveals whether a team's rotation improves or degrades over the course of a match — for example, due to fatigue, tilt, or adapting to an opponent's playstyle.

---

## Appendix: Legacy Discrete Thresholds

Prior to the switch to continuous linear interpolation, all bot detection metrics used stepped if/else thresholds. These are preserved here for reference.

### Input Diversity (original)

Used `min(steer, throttle)` unique counts:

| Condition | Input Score |
|---|---|
| Both discrete (0, 128, 255) | 1.0 |
| ≤ 10 unique values | 0.9 |
| ≤ 50 unique values | 0.75 |
| ≤ 75 unique values | 0.6 |
| ≤ 100 unique values | 0.4 |
| > 100 unique values | 0.0 |

### Pre-Hold (original)

Binary: any pre-hold applied a flat **0.4x** multiplier.

### Kickoff Reaction Consistency (original)

| Reaction Stddev (frames) | Multiplier |
|---|---|
| < 1.0 | 1.5x |
| < 3.0 | 1.3x |
| < 5.0 | 1.1x |
| ≥ 5.0 | 1.0x |

### Dribble (original)

| Suspicion Score | Multiplier |
|---|---|
| ≥ 0.7 | 1.4x |
| ≥ 0.4 | 1.2x |
| ≥ 0.1 | 1.1x |
| < 0.1 | 1.0x |

### Input Asymmetry (original)

| Ratio | Multiplier |
|---|---|
| ≥ 4.0 | 1.8x |
| ≥ 2.5 | 1.5x |
| ≥ 1.75 | 1.2x |
| < 1.75 | 1.0x |
