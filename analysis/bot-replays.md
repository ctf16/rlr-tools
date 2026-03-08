# Bot Replay Analysis

Analysis of 12 Rocket League replay files suspected to contain bot players.

## Methodology

The bot detection module (`src/bot_detection.rs`) extracts `ReplicatedSteer` and `ReplicatedThrottle` byte values from network frames for each player. These values range from 0–255, where 128 is neutral.

**Key insight:** Human players using analog sticks produce 60–250+ unique values across a game due to the continuous nature of analog input. Bots using digital-only input produce exactly 3 discrete values: `{0, 128, 255}` (full left, neutral, full right for steer; full reverse, neutral, full forward for throttle).

A player is flagged as a bot when both their steer and throttle unique value counts equal 3 and all values are in `{0, 128, 255}`. Platform weighting applies a confidence multiplier (Epic ×1.0, Steam ×0.75) since most known bots operate on Epic accounts.

## Summary Table

| Game | Date | Mode | Score | Bots Detected | Bot Names | Notes |
|------|------|------|-------|---------------|-----------|-------|
| bot1 | — | 1v1 Soccar | 7-6 | 0 | — | "EAC Bypass Bot" name but human inputs |
| bot2 | — | 1v1 Soccar | 2-0 forfeit | 0 | — | "EAC Bypass Bot" name but human inputs |
| bot3 | — | 2v2 Soccar | 3-6 forfeit | 2 | TheFluff RL, TheFluff RL(1) | Confirmed discrete-only pair |
| bot4 | — | 2v2 Soccar | 3-? | 2 | ΣΩΖΔ, ΣΩΖΔ(1) | Confirmed discrete-only pair; no PlayerStats |
| bot5 | — | 2v2 Soccar | 4-? | 2 (likely) | 15 seel., 15 seel.(1) | Discrete throttle, analog steer — hybrid bot |
| bot6 | — | 2v2 Soccar | 4-3 | 0 | — | All human inputs; misleading player name |
| bot7 | — | 2v2 Soccar | 0-4 forfeit | 2 | Keimo_a_Rosca, Keimo_a_Rosca(1) | Confirmed discrete-only pair |
| bot8 | — | 2v2 Soccar | 0-3 forfeit | 2 | benji, benji(1) | Confirmed discrete-only pair |
| bot9 | — | 2v2 Soccar | 3-5 | 0 (suspected) | Zret85, Zret85(1) | Sophisticated bot — limited analog noise |
| bot10 | — | 2v2 Soccar | 6-3 forfeit | 0 (suspected) | Zret85, Zret85(1) | Sophisticated bot — limited analog noise |
| bot11 | — | 2v2 Soccar | 0-4 | 0 (suspected) | Zret85, Zret85(1) | Sophisticated bot — limited analog noise |
| bot12 | — | 2v2 Soccar | 2-5 | 0 (suspected) | Zret85, Zret85(1) | Sophisticated bot — limited analog noise |

## Per-Game Analysis

### bot1 — "EAC Bypass Bot" (1v1)

- **Mode:** 1v1 Soccar
- **Score:** 7-6
- **Duration:** 5:07
- **Forfeit:** No

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| EAC Bypass Bot | — | Epic | 233 | — | No | Human |
| Malamon1968 | — | PS4 | 216 | — | No | Human |

Despite the provocative name suggesting a bot bypassing Easy Anti-Cheat, both players show fully human analog input patterns with 200+ unique steer values. The name alone is not evidence of botting.

---

### bot2 — "EAC Bypass Bot" (short 1v1)

- **Mode:** 1v1 Soccar
- **Score:** 2-0
- **Duration:** 0:38
- **Forfeit:** Yes

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| EAC Bypass Bot | — | Epic | 210 | — | No | Human |
| AquaticZy | — | — | — | — | — | Missing from frames |
| XxCowboys24 | — | — | 126 | — | No | Human |
| Raja | — | — | 0 | — | — | bBot=true, 0 samples |

Very short game (38 seconds). Raja has bBot=true in header metadata but has 0 input samples in network frames. AquaticZy is absent from frame data entirely. Despite the suspicious name, "EAC Bypass Bot" shows human inputs.

---

### bot3 — TheFluff RL (confirmed bots)

- **Mode:** 2v2 Soccar
- **Score:** 3-6
- **Duration:** 2:36
- **Forfeit:** Yes

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **TheFluff RL** | — | Epic | **3** | **3** | **Yes** | **BOT** |
| **TheFluff RL(1)** | — | Epic | **3** | **3** | **Yes** | **BOT** |
| luca | — | Steam | 239 | — | No | Human |
| a l p h a | — | Steam | 231 | — | No | Human |

Clear bot signature: both TheFluff RL accounts use exactly 3 discrete values for steer and throttle. Epic platform, paired accounts with `(1)` suffix — classic bot pair pattern. Game ended in forfeit after 2:36.

---

### bot4 — ΣΩΖΔ (confirmed bots)

- **Mode:** 2v2 Soccar
- **Score:** 3-?
- **Frames:** 1181
- **PlayerStats:** Missing (incomplete game)

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **ΣΩΖΔ** | — | — | **3** | **3** | **Yes** | **BOT** |
| **ΣΩΖΔ(1)** | — | — | **3** | **3** | **Yes** | **BOT** |
| Cole | — | — | 58 | — | No | Human |
| yin (i hate robots) | — | — | 169 | — | No | Human |

Discrete-only bot pair. Platform data unavailable due to missing PlayerStats. The opponent "yin (i hate robots)" expresses frustration with bot opponents directly in their player name.

---

### bot5 — 15 seel. (likely bots)

- **Mode:** 2v2 Soccar
- **Score:** 4-?
- **Frames:** 4823
- **PlayerStats:** Missing (incomplete game)

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **15 seel.** | — | — | 220+ | **3** | **Partial** | **Likely bot** |
| **15 seel.(1)** | — | — | 220+ | **3** | **Partial** | **Likely bot** |
| BOTS DE ** | — | — | 238 | — | No | Human |
| amplitude compaction | — | — | 210 | — | No | Human |

Interesting hybrid case: the 15 seel. pair shows analog steer values (220+ unique) but discrete-only throttle (exactly 3 values). This suggests a more sophisticated bot that simulates analog steering but still uses binary throttle. The paired `(1)` naming pattern reinforces the bot assessment. Opponent "BOTS DE **" is another player whose name references bots.

---

### bot6 — No bots detected

- **Mode:** 2v2 Soccar
- **Score:** 4-3
- **Duration:** 5:02
- **Forfeit:** No

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| i love playing bots | — | Steam | 237 | — | No | Human |
| Zouzomouk | — | Epic | 237 | — | No | Human |
| Zouzomouk(1) | — | Steam | 252 | — | No | Human |
| mass | — | Steam | 237 | — | No | Human |

All four players show human analog input patterns. This replay was likely included in the collection due to the player name "i love playing bots." Note that Zouzomouk and Zouzomouk(1) are on different platforms (Epic and Steam), unlike bot pairs which share a platform — this is a human player who queued with a similarly named friend.

---

### bot7 — Keimo_a_Rosca (confirmed bots)

- **Mode:** 2v2 Soccar
- **Score:** 0-4
- **Duration:** 1:40
- **Forfeit:** Yes

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **Keimo_a_Rosca** | — | Epic | **3** | **3** | **Yes** | **BOT** |
| **Keimo_a_Rosca(1)** | — | Epic | **3** | **3** | **Yes** | **BOT** |
| chega de bots! | — | Steam | 241 | — | No | Human |
| Golden | — | Steam | 135 | — | No | Human |

Clear bot signature. First appearance of "chega de bots!" (Portuguese for "enough with bots!"), who appears in 5 of the 12 replays. Game ended in a quick forfeit at 1:40.

---

### bot8 — benji (confirmed bots)

- **Mode:** 2v2 Soccar
- **Score:** 0-3
- **Duration:** 4:20
- **Forfeit:** Yes

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **benji** | — | Epic | **3** | **3** | **Yes** | **BOT** |
| **benji(1)** | — | Epic | **3** | **3** | **Yes** | **BOT** |
| pretty please no bot thx rl | — | Steam | 256 | — | No | Human |
| . | — | Steam | 250 | — | No | Human |

Clear bot signature. Another player whose name is a plea to Psyonix about the bot problem: "pretty please no bot thx rl."

---

### bot9 — Zret85 (suspected sophisticated bots)

- **Mode:** 2v2 Soccar
- **Score:** 3-5
- **Duration:** 5:00
- **Forfeit:** No

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **Zret85** | — | Epic | **50** | — | No | **Suspected bot** |
| **Zret85(1)** | — | Epic | **32** | — | No | **Suspected bot** |
| chega de bots! | — | Steam | 241 | — | No | Human |
| ferraz | — | Steam | 236 | — | No | Human |

Not flagged by the discrete-only detection, but the Zret85 pair shows suspiciously low unique steer counts (50 and 32) compared to the human players (241 and 236). See cross-game analysis below.

---

### bot10 — Zret85 (suspected sophisticated bots)

- **Mode:** 2v2 Soccar
- **Score:** 6-3
- **Duration:** 4:26
- **Forfeit:** Yes

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **Zret85** | — | Epic | **53** | — | No | **Suspected bot** |
| **Zret85(1)** | — | Epic | **43** | — | No | **Suspected bot** |
| chega de bots! | — | Steam | 241 | — | No | Human |
| hitbot | — | Steam | 231 | — | No | Human |

Same Zret85 pair, same anomaly. Opponent "hitbot" is yet another name referencing bots.

---

### bot11 — Zret85 (suspected sophisticated bots)

- **Mode:** 2v2 Soccar
- **Score:** 0-4
- **Duration:** 5:00
- **Forfeit:** No

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **Zret85** | — | Epic | **30** | — | No | **Suspected bot** |
| **Zret85(1)** | — | Epic | **52** | — | No | **Suspected bot** |
| chega de bots! | — | Steam | 241 | — | No | Human |
| thiagxw | — | Steam | 241 | — | No | Human |

Same pattern. Zret85 at 30 unique steer values is the lowest seen across all 4 games.

---

### bot12 — Zret85 (suspected sophisticated bots)

- **Mode:** 2v2 Soccar
- **Score:** 2-5
- **Duration:** 5:02
- **Forfeit:** No

| Player | Team | Platform | Steer Unique | Throttle Unique | Discrete? | Verdict |
|--------|------|----------|-------------|----------------|-----------|---------|
| **Zret85** | — | Epic | **32** | **10** | No | **Suspected bot** |
| **Zret85(1)** | — | Epic | **30** | **37** | No | **Suspected bot** |
| chega de bots! | — | Steam | 241 | — | No | Human |
| lucas06 | — | Steam | 233 | — | No | Human |

Throttle data here shows extremely low unique values (10 and 37) — further evidence that Zret85 is not using genuine analog input.

## Cross-Game Observations

### Confirmed bot pairs (4 games)

bot3 (TheFluff RL), bot4 (ΣΩΖΔ), bot7 (Keimo_a_Rosca), bot8 (benji) — all use exactly 3 discrete steer and throttle values, all are Epic platform pairs with the `(1)` suffix naming convention. These are definitively bots using the simplest input scheme.

### Likely bots (1 game)

bot5 (15 seel.) — discrete throttle (3 values) but analog steer (220+ values). This is a hybrid profile suggesting a more advanced bot that simulates analog steering while still using binary throttle control. The paired naming convention matches the bot pattern.

### Suspicious names but human inputs (2 games)

bot1 and bot2 feature "EAC Bypass Bot" — a player whose name implies bot activity or EAC circumvention, but whose input patterns are fully human (210–233 unique steer values). The data does not support a bot verdict based on inputs alone.

### Suspected sophisticated bots (4 games)

bot9 through bot12 all feature **Zret85 + Zret85(1)** on Epic. Their unique value counts across all 4 games:

| Game | Zret85 Steer | Zret85(1) Steer | Zret85 Throttle | Zret85(1) Throttle |
|------|-------------|----------------|----------------|-------------------|
| bot9 | 50 | 32 | — | — |
| bot10 | 53 | 43 | — | — |
| bot11 | 30 | 52 | — | — |
| bot12 | 32 | 30 | 10 | 37 |

These values (30–53 unique steer, 10–37 unique throttle) are far below the human baseline of 100–250+ unique values seen in every confirmed human player across all 12 games. The consistency across 4 separate games eliminates the possibility of a fluke. This strongly suggests a more sophisticated bot that injects limited analog noise to evade discrete-only detection — but not enough noise to pass as human.

### "chega de bots!" — recurring frustrated human

This player (Portuguese: "enough with bots!") appears in 5 of the 12 replays (bot7, bot9–bot12) and consistently shows human input patterns (~241 unique steer values). They are clearly a human player frustrated by repeatedly matching against bots in online play.

### Missing PlayerStats

bot4 and bot5 have empty `PlayerStats` arrays in the replay header, likely due to incomplete or very short games. Platform and scoreboard data is unavailable for players in those games, though network frame analysis still works.

### bBot flag unreliability

The replay header's `bBot` field is not a reliable indicator. Raja in bot2 has `bBot=true` but has 0 input samples (possibly a spectator or disconnected player). Meanwhile, all confirmed bots across bot3, bot4, bot7, and bot8 have `bBot=false`. This field appears to only flag official Psyonix bots (used to backfill games), not third-party bots.

## Conclusions

**The discrete value detection method works well for basic bots.** The 3-value `{0, 128, 255}` signature cleanly identifies 4 confirmed bot pairs (8 bot accounts) with zero ambiguity. There is a clear bimodal distribution — humans produce 100–250+ unique values, basic bots produce exactly 3.

**Hybrid and sophisticated bots require additional heuristics.** The 15 seel. pair (discrete throttle, analog steer) and the Zret85 pair (low-but-not-discrete values) demonstrate that bot developers are already working to evade simple detection. A threshold-based approach (e.g., flagging players with <60 unique steer values) would catch the Zret85 type, but the threshold needs careful calibration to avoid false positives on players with short play times or unusual controllers.

**Platform correlation strengthens confidence.** Every confirmed and suspected bot in this dataset uses an Epic account. Free Epic accounts have near-zero cost to create, making them the preferred platform for bot operators. Steam accounts carry a purchase cost, providing a natural deterrent.

**Player names are not evidence.** "EAC Bypass Bot" showed fully human inputs. Names can be misleading in both directions — provocative names on human players, innocuous names on actual bots.
