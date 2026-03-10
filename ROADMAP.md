# Roadmap

## Core

- [x] **Replay parsing & caching** — Parse `.replay` files into full JSON (including network data) via boxcars, with cached output in `parsed_games/`
- [x] **Match overview & player stats** — Human-readable summaries: game type, score, duration, forfeit detection, per-player scoreboard
- [x] **Interactive replay selector** — CLI menu to browse replay categories, select replays, and choose analysis actions

## Replay Verification

- [x] **Merkle tree replay signing** — Split replay JSON into semantic sections, hash into a Merkle tree, sign the root with hybrid Ed25519 + ML-DSA-65
- [x] **Signature verification** — Load `.sig` sidecar files, verify both signatures, and detect which section was tampered with
- [ ] **Replay integrity chain** — Extend the merkle/sidecar system to chain replays together for a verifiable tournament match history

## Bot Detection

- [x] **Input analysis** — Score players based on unique steer/throttle value counts, discrete-only input detection, and platform weighting
- [x] **Kickoff signals** — Integrate kickoff pre-hold (human signal) and reaction consistency (bot signal) into the bot score
- [ ] **Steer alternation rate** — Count direction changes per second to separate keyboard players (human timing, ~4 changes/sec max) from bots (rapid mechanical alternation, 15+/sec)
- [ ] **Hold duration variance** — Measure how long each steer/throttle value is held; humans have high variance, bots tend toward uniform durations
- [ ] **Input entropy scoring** — Replace the unique-value-count scoring ladder with Shannon entropy over the steer/throttle histogram; controller players produce a smooth bell curve, bots produce narrow spikes
- [ ] **Keyboard-aware scoring path** — When a player is identified as discrete-only (keyboard), shift scoring to timing-based signals instead of penalizing low unique counts
- [ ] **Multi-input synchrony** — Track how often steer + throttle + boost + jump change on the exact same frame; bots show unnaturally high simultaneous input rates
- [ ] **Dodge timing consistency** — Measure variance of dodge-after-jump delay across the match; frame-perfect double jumps every time = bot signal
- [ ] **Post-impact recovery timing** — After bumps/demos (velocity spikes), measure time-to-first-input; bots recover in fixed frame counts, humans vary
- [ ] **Boost tap duration variance** — Measure the length of each boost-on period; low variance across many taps = suspicious
- [ ] **Ball prediction accuracy** — Compare player heading to the direction of the ball's extrapolated future position; consistently low intercept-angle error = bot signal
- [ ] **Rotation period regularity** — Measure stddev of time between offensive/defensive transitions; mechanically regular cycling = suspicious
- [ ] **Steer-to-ball correlation** — Compute ideal steer to face the ball each frame, compare to actual; bots track with low consistent error, humans overshoot/undershoot noisily
- [ ] **Aggregate suspicion profile** — Combine bot detection + kickoff + boost + rotation into a single per-player composite score with confidence interval

## Match Analysis

- [x] **Kickoff analysis** — Detect kickoff windows, measure per-player reaction latency, pre-hold detection, steer/throttle sequence variability across kickoffs
- [x] **Boost analysis** — Track average boost level, time at zero/full, boost collected/consumed, big and small pad pickup counts
- [x] **Rotation & positioning analysis** — Ball chasing %, offensive/defensive split, double commit detection, back-post rotation rate, per-minute breakdowns
- [ ] **Goal sequence analysis** — Analyze network frames before each goal: ball touches, pass sequences, time-to-goal from possession change
- [ ] **Demolition & bump tracking** — Parse demo/bump events, track counts per player, and correlate with bot-like targeting patterns
- [ ] **Speed & supersonic tracking** — Track car speed over time, % at supersonic, acceleration patterns; bots may show unnaturally consistent speed management

## Tooling & Infrastructure

- [ ] **Replay comparison / diff** — Compare the same player across multiple replays to detect consistent bot fingerprints
- [ ] **Training data export** — Export labeled feature vectors (human vs bot) to CSV/JSON for ML classifier training; one row per player-per-match
- [ ] **Batch analysis mode** — Analyze all replays in a directory, output a summary CSV; useful for scanning tournament replays
- [ ] **Player identity tracking** — Track players across replays via OnlineID/UniqueNetId; build a history profile with rolling bot score and play style fingerprint
- [ ] **Web API / server mode** — Expose analysis endpoints via HTTP (e.g. axum) for integration with Discord bots or web frontends
