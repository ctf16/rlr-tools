use crate::dribble_analysis;
use crate::input_synchrony;
use crate::kickoff_analysis;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::error;

struct PlayerInputProfile {
    name: String,
    steer_values: Vec<u8>,
    throttle_values: Vec<u8>,
}

pub struct BotDetectionResult {
    pub name: String,
    pub platform: String,
    pub is_spectator: bool,
    pub unique_steer_count: usize,
    pub unique_throttle_count: usize,
    pub total_steer_updates: usize,
    pub total_throttle_updates: usize,
    pub steer_only_discrete: bool,
    pub throttle_only_discrete: bool,
    pub pre_hold_count: usize,
    pub pre_hold_mult: f64,
    pub kickoff_count: usize,
    pub kickoff_consistency_mult: f64,
    pub kickoff_sequence_mult: f64,
    pub discrete_kickoff_similarity: Option<f64>,
    pub discrete_kickoff_mult: f64,
    pub qualifying_dribble_count: usize,
    pub dribble_mult: f64,
    pub base_score: f64,
    pub asymmetry_ratio: f64,
    pub asymmetry_mult: f64,
    pub platform_multiplier: f64,
    pub timing_bot_score: Option<f64>,
    pub timing_detail: Option<Value>,
    pub used_timing_path: bool,
    pub bot_score: f64,
    pub verdict: &'static str,
}

const DISCRETE_VALUES: [u8; 3] = [0, 128, 255];

/// Linearly interpolate: returns `high_out` at `low_in`, `low_out` at `high_in`,
/// clamped outside the range.
fn lerp_clamp(value: f64, low_in: f64, high_in: f64, high_out: f64, low_out: f64) -> f64 {
    if value <= low_in {
        return high_out;
    }
    if value >= high_in {
        return low_out;
    }
    let t = (value - low_in) / (high_in - low_in);
    high_out + t * (low_out - high_out)
}

// 95% of cheaters are on Epic, 5% on Steam.
// Epic gets no reduction, Steam gets a significant reduction, others in between.
fn platform_multiplier(platform: &str) -> f64 {
    match platform {
        "Epic" => 1.0,
        "Steam" => 0.75,
        _ => 0.85,
    }
}

fn build_platform_lookup(parsed_json: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(players) = parsed_json["properties"]["PlayerStats"].as_array() {
        for player in players {
            let name = player["Name"].as_str().unwrap_or_default();
            let platform_raw = player["Platform"]["value"].as_str().unwrap_or("Unknown");
            let platform = platform_raw
                .strip_prefix("OnlinePlatform_")
                .unwrap_or(platform_raw);
            map.insert(name.to_string(), platform.to_string());
        }
    }
    map
}

fn is_discrete_only(values: &[u8]) -> bool {
    values.iter().all(|v| DISCRETE_VALUES.contains(v))
}

fn resolve_object_id(objects: &[Value], needle: &str) -> Option<u64> {
    objects
        .iter()
        .position(|o| o.as_str().map_or(false, |s| s == needle))
        .map(|i| i as u64)
}

pub fn analyze(parsed_json: &Value) -> Result<Vec<BotDetectionResult>, Box<dyn error::Error>> {
    let objects = parsed_json["objects"]
        .as_array()
        .ok_or("missing objects array")?;

    let steer_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedSteer")
        .ok_or("ReplicatedSteer not found in objects")?;
    let throttle_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedThrottle")
        .ok_or("ReplicatedThrottle not found in objects")?;
    let pri_link_oid = resolve_object_id(objects, "Engine.Pawn:PlayerReplicationInfo")
        .ok_or("PlayerReplicationInfo not found in objects")?;
    let name_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:PlayerName")
        .ok_or("PlayerName not found in objects")?;

    let frames = parsed_json["network_frames"]["frames"]
        .as_array()
        .ok_or("missing network_frames.frames")?;

    // player actor_id -> name
    let mut player_names: HashMap<u64, String> = HashMap::new();
    // car actor_id -> player actor_id
    let mut car_to_player: HashMap<u64, u64> = HashMap::new();
    // player actor_id -> input profile
    let mut profiles: HashMap<u64, PlayerInputProfile> = HashMap::new();

    for frame in frames {
        let Some(updated) = frame["updated_actors"].as_array() else {
            continue;
        };

        for actor in updated {
            let actor_id = actor["actor_id"].as_u64().unwrap_or(u64::MAX);
            let object_id = actor["object_id"].as_u64().unwrap_or(u64::MAX);
            let attr = &actor["attribute"];

            if object_id == name_oid {
                if let Some(name) = attr["String"].as_str() {
                    player_names.insert(actor_id, name.to_string());
                    profiles
                        .entry(actor_id)
                        .or_insert_with(|| PlayerInputProfile {
                            name: name.to_string(),
                            steer_values: Vec::new(),
                            throttle_values: Vec::new(),
                        });
                }
            } else if object_id == pri_link_oid {
                if let Some(player_actor_id) = attr["ActiveActor"]["actor"].as_u64() {
                    car_to_player.insert(actor_id, player_actor_id);
                }
            } else if object_id == steer_oid {
                if let Some(byte_val) = attr["Byte"].as_u64() {
                    if let Some(&player_id) = car_to_player.get(&actor_id) {
                        if let Some(profile) = profiles.get_mut(&player_id) {
                            profile.steer_values.push(byte_val as u8);
                        }
                    }
                }
            } else if object_id == throttle_oid {
                if let Some(byte_val) = attr["Byte"].as_u64() {
                    if let Some(&player_id) = car_to_player.get(&actor_id) {
                        if let Some(profile) = profiles.get_mut(&player_id) {
                            profile.throttle_values.push(byte_val as u8);
                        }
                    }
                }
            }
        }
    }

    let platform_lookup = build_platform_lookup(parsed_json);

    // Run kickoff analysis to get pre-hold, reaction consistency, sequence variability,
    // and discrete kickoff similarity per player.
    #[allow(clippy::type_complexity)]
    let kickoff_lookup: HashMap<
        String,
        (
            usize,
            usize,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
        ),
    > = kickoff_analysis::analyze(parsed_json)
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            // For discrete players, use the max similarity across steer/throttle.
            let discrete_sim = match (r.discrete_steer_similarity, r.discrete_throttle_similarity) {
                (Some(s), Some(t)) => Some(s.max(t)),
                (Some(s), None) => Some(s),
                (None, Some(t)) => Some(t),
                _ => None,
            };
            (
                r.name,
                (
                    r.pre_hold_count,
                    r.kickoff_count,
                    r.reaction_stddev,
                    r.steer_variability,
                    r.throttle_variability,
                    discrete_sim,
                ),
            )
        })
        .collect();

    let dribble_lookup: HashMap<String, (f64, usize)> = dribble_analysis::analyze(parsed_json)
        .unwrap_or_default()
        .into_iter()
        .map(|r| (r.name, (r.dribble_suspicion_score, r.dribble_count)))
        .collect();
    // Run input synchrony analysis for timing-based scoring of keyboard players.
    let synchrony_results = input_synchrony::analyze(parsed_json).unwrap_or_default();
    let timing_lookup: HashMap<String, f64> = synchrony_results
        .iter()
        .filter(|r| r.is_discrete_only)
        .map(|r| (r.name.clone(), r.timing_bot_score))
        .collect();
    let synchrony_lookup: HashMap<String, Value> = synchrony_results
        .into_iter()
        .filter(|r| r.is_discrete_only)
        .map(|r| {
            (
                r.name.clone(),
                json!({
                    "steer_alternation_rate": r.steer_alternation_rate,
                    "throttle_alternation_rate": r.throttle_alternation_rate,
                    "steer_hold_mean": r.steer_hold_mean,
                    "steer_hold_stddev": r.steer_hold_stddev,
                    "steer_hold_cv": r.steer_hold_cv,
                    "throttle_hold_mean": r.throttle_hold_mean,
                    "throttle_hold_stddev": r.throttle_hold_stddev,
                    "throttle_hold_cv": r.throttle_hold_cv,
                    "simultaneous_changes": r.simultaneous_changes,
                    "total_change_frames": r.total_change_frames,
                    "simultaneous_change_rate": r.simultaneous_change_rate,
                    "timing_bot_score": r.timing_bot_score,
                }),
            )
        })
        .collect();

    let mut results: Vec<BotDetectionResult> = profiles
        .into_values()
        .map(|profile| {
            let unique_steer: HashSet<u8> = profile.steer_values.iter().copied().collect();
            let unique_throttle: HashSet<u8> = profile.throttle_values.iter().copied().collect();

            let is_spectator =
                profile.steer_values.is_empty() && profile.throttle_values.is_empty();

            let steer_only_discrete = is_discrete_only(&profile.steer_values);
            let throttle_only_discrete = is_discrete_only(&profile.throttle_values);

            let has_enough_samples = profile.steer_values.len() >= 10;

            // Score based on unique input variety. Humans on analog sticks produce
            // 100+ distinct values; bots typically produce far fewer.
            // Use the average of steer and throttle counts — asymmetry between
            // channels is handled separately by the asymmetry multiplier.
            let avg_unique = (unique_steer.len() + unique_throttle.len()) as f64 / 2.0;

            let input_score = if !has_enough_samples {
                0.0
            } else if steer_only_discrete && throttle_only_discrete {
                1.0
            } else {
                lerp_clamp(avg_unique, 10.0, 220.0, 0.9, 0.05)
            };

            let platform = platform_lookup
                .get(&profile.name)
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            let plat_mult = platform_multiplier(&platform);

            let (pre_hold_count, kickoff_count, reaction_stddev, steer_var, throttle_var, discrete_kickoff_sim) =
                kickoff_lookup
                    .get(&profile.name)
                    .copied()
                    .unwrap_or((0, 0, None, None, None, None));

            // Pre-holding throttle before countdown is a human signal.
            // Scale by ratio of kickoffs with pre-holds; max 30% reduction at 100%.
            let pre_hold_ratio = if kickoff_count > 0 {
                pre_hold_count as f64 / kickoff_count as f64
            } else {
                0.0
            };
            let pre_hold_mult = 1.0 - 0.3 * pre_hold_ratio;

            // Very consistent reaction timing across kickoffs is a strong bot signal.
            // Humans have natural variance; bots respond with near-identical delays.
            // Requires 3+ kickoffs for meaningful stddev. Scale up to 2.0x for near-zero stddev.
            let kickoff_consistency_mult = match reaction_stddev {
                Some(stddev) if kickoff_count >= 3 => lerp_clamp(stddev, 0.0, 5.0, 2.0, 1.0),
                _ => 1.0,
            };

            // Very similar input sequences across kickoffs is a bot signal.
            // Uses the more suspicious (lower variability) channel.
            let kickoff_sequence_mult = match (steer_var, throttle_var) {
                (Some(sv), Some(tv)) if kickoff_count >= 3 => {
                    let min_var = sv.min(tv);
                    lerp_clamp(min_var, 0.0, 0.05, 1.5, 1.0)
                }
                _ => 1.0,
            };

            // For keyboard players: exact-match kickoff similarity.
            // Bots replay near-identical frame sequences; humans vary timing even
            // with the same 3 discrete values. Similarity > 0.85 is suspicious.
            let discrete_kickoff_mult = match discrete_kickoff_sim {
                Some(sim) if kickoff_count >= 3 && steer_only_discrete && throttle_only_discrete => {
                    lerp_clamp(sim, 0.70, 0.95, 1.0, 1.5)
                }
                _ => 1.0,
            };

            let (dribble_suspicion, qualifying_dribble_count) = dribble_lookup
                .get(&profile.name)
                .copied()
                .unwrap_or((0.0, 0));
            let dribble_mult = lerp_clamp(dribble_suspicion, 0.1, 0.8, 1.0, 1.4);

            // Steer/throttle asymmetry: real players have correlated diversity
            // (both high for analog, both discrete for keyboard). A large ratio
            // with one channel showing genuine analog diversity is suspicious.
            let (hi, lo) = if unique_steer.len() >= unique_throttle.len() {
                (unique_steer.len(), unique_throttle.len())
            } else {
                (unique_throttle.len(), unique_steer.len())
            };
            let asymmetry_ratio = if lo > 0 {
                hi as f64 / lo as f64
            } else {
                f64::MAX
            };
            let asymmetry_mult = if hi <= 30 {
                1.0
            } else {
                lerp_clamp(asymmetry_ratio, 1.5, 4.5, 1.0, 1.8)
            };

            // For discrete-only players (keyboard), use timing-based scoring instead
            // of penalizing them for having few unique input values.
            let timing_score = timing_lookup.get(&profile.name).copied();
            let both_discrete = steer_only_discrete && throttle_only_discrete;
            let used_timing_path = both_discrete && timing_score.is_some();

            // Discrete-only inputs are inherently suspicious (3 of 256 values).
            // Add a floor so multipliers have something to amplify even when
            // timing analysis returns 0.0.
            let base_score = if used_timing_path {
                (timing_score.unwrap() + 0.15).min(1.0)
            } else {
                input_score
            };

            // Only apply dribble multiplier when there are enough qualifying dribbles.
            let effective_dribble_mult = if qualifying_dribble_count >= 2 {
                dribble_mult
            } else {
                1.0
            };

            let bot_score = (base_score
                * plat_mult
                * pre_hold_mult
                * kickoff_consistency_mult
                * kickoff_sequence_mult
                * discrete_kickoff_mult
                * effective_dribble_mult
                * asymmetry_mult)
                .min(1.0);

            let verdict = if is_spectator {
                "Spectator"
            } else if bot_score >= 0.9 {
                "Bot"
            } else if bot_score >= 0.5 {
                "Likely Bot"
            } else {
                "Human"
            };

            let timing_detail = synchrony_lookup.get(&profile.name).cloned();

            BotDetectionResult {
                name: profile.name,
                platform,
                is_spectator,
                unique_steer_count: unique_steer.len(),
                unique_throttle_count: unique_throttle.len(),
                total_steer_updates: profile.steer_values.len(),
                total_throttle_updates: profile.throttle_values.len(),
                steer_only_discrete,
                throttle_only_discrete,
                pre_hold_count,
                pre_hold_mult,
                kickoff_count,
                kickoff_consistency_mult,
                kickoff_sequence_mult,
                discrete_kickoff_similarity: discrete_kickoff_sim,
                discrete_kickoff_mult,
                qualifying_dribble_count,
                dribble_mult,
                base_score,
                asymmetry_ratio: if asymmetry_ratio == f64::MAX { -1.0 } else { asymmetry_ratio },
                asymmetry_mult,
                platform_multiplier: plat_mult,
                timing_bot_score: timing_score,
                timing_detail,
                used_timing_path,
                bot_score,
                verdict,
            }
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

impl BotDetectionResult {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "platform": self.platform,
            "is_spectator": self.is_spectator,
            "unique_steer_count": self.unique_steer_count,
            "unique_throttle_count": self.unique_throttle_count,
            "total_steer_updates": self.total_steer_updates,
            "total_throttle_updates": self.total_throttle_updates,
            "steer_only_discrete": self.steer_only_discrete,
            "throttle_only_discrete": self.throttle_only_discrete,
            "pre_hold_count": self.pre_hold_count,
            "pre_hold_mult": self.pre_hold_mult,
            "kickoff_count": self.kickoff_count,
            "kickoff_consistency_mult": self.kickoff_consistency_mult,
            "kickoff_sequence_mult": self.kickoff_sequence_mult,
            "discrete_kickoff_similarity": self.discrete_kickoff_similarity,
            "discrete_kickoff_mult": self.discrete_kickoff_mult,
            "qualifying_dribble_count": self.qualifying_dribble_count,
            "dribble_mult": self.dribble_mult,
            "base_score": self.base_score,
            "asymmetry_ratio": self.asymmetry_ratio,
            "asymmetry_mult": self.asymmetry_mult,
            "platform_multiplier": self.platform_multiplier,
            "timing_bot_score": self.timing_bot_score,
            "timing_detail": self.timing_detail,
            "used_timing_path": self.used_timing_path,
            "bot_score": self.bot_score,
            "verdict": self.verdict,
        })
    }
}

pub fn results_to_json(results: &[BotDetectionResult]) -> Value {
    let players: Vec<Value> = results
        .iter()
        .filter(|r| !r.is_spectator)
        .map(|r| r.to_json())
        .collect();
    let spectators: Vec<Value> = results
        .iter()
        .filter(|r| r.is_spectator)
        .map(|r| r.to_json())
        .collect();
    json!({
        "players": players,
        "spectators": spectators,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demystify;

    fn load_and_analyze(replay_name: &str) -> Vec<BotDetectionResult> {
        let cache_path = format!("parsed_games/{}.json", replay_name);
        let json = demystify::load_parsed_json(&cache_path)
            .unwrap_or_else(|e| panic!("Failed to load {}: {}", cache_path, e));
        analyze(&json).unwrap_or_else(|e| panic!("Failed to analyze {}: {}", replay_name, e))
    }

    fn find_player<'a>(results: &'a [BotDetectionResult], name: &str) -> &'a BotDetectionResult {
        results
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("Player '{}' not found in results", name))
    }

    fn assert_not_human(result: &BotDetectionResult) {
        assert!(
            result.verdict != "Human",
            "{} should not be Human, got score={:.2} verdict={}",
            result.name,
            result.bot_score,
            result.verdict
        );
    }

    // bot9: Zret85(1) has 32 unique steer / 10 unique throttle — low diversity
    // triggers the input_score=0.9 path (min_unique <= 10).
    #[test]
    fn bot9_zret85_clone_detected() {
        let results = load_and_analyze("bot9");
        assert_not_human(find_player(&results, "Zret85(1)"));
    }

    // bot10-12: same bot (Zret85(1)) in different matches
    #[test]
    fn bot10_zret85_clone_detected() {
        let results = load_and_analyze("bot10");
        assert_not_human(find_player(&results, "Zret85(1)"));
    }

    #[test]
    fn bot11_zret85_clone_detected() {
        let results = load_and_analyze("bot11");
        assert_not_human(find_player(&results, "Zret85(1)"));
    }

    #[test]
    fn bot12_zret85_clone_detected() {
        let results = load_and_analyze("bot12");
        assert_not_human(find_player(&results, "Zret85(1)"));
    }

    // Known detection gap: discrete-only bots with human-like timing.
    // These bots use only {0, 128, 255} inputs but have low alternation rates
    // (<5/s) and high hold CV (>1.0), so they score 0.0 on the timing path.
    // bot3: TheFluff RL(1), bot7: Keimo_a_Rosca(1), bot8: ᵇᵉⁿʲi(1)
    #[test]
    fn bot3_thefluff_clone_is_discrete_only() {
        let results = load_and_analyze("bot3");
        let bot = find_player(&results, "TheFluff RL(1)");
        assert!(bot.steer_only_discrete && bot.throttle_only_discrete);
        assert!(bot.used_timing_path, "should use timing path for discrete inputs");
    }

    #[test]
    fn bot7_keimo_clone_is_discrete_only() {
        let results = load_and_analyze("bot7");
        let bot = find_player(&results, "Keimo_a_Rosca(1)");
        assert!(bot.steer_only_discrete && bot.throttle_only_discrete);
        assert!(bot.used_timing_path, "should use timing path for discrete inputs");
    }

    #[test]
    fn bot8_benji_clone_is_discrete_only() {
        let results = load_and_analyze("bot8");
        let bot = find_player(&results, "ᵇᵉⁿʲi(1)");
        assert!(bot.steer_only_discrete && bot.throttle_only_discrete);
        assert!(bot.used_timing_path, "should use timing path for discrete inputs");
    }

    /// Parse-and-analyze helper: ensures the replay is cached, then runs bot detection.
    fn parse_and_analyze(replay_path: &str, cache_name: &str) -> Vec<BotDetectionResult> {
        crate::parser::run_cached(replay_path)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", replay_path, e));
        load_and_analyze(cache_name)
    }

    #[test]
    fn discrete_kickoff_similarity_yukeo_vs_bots() {
        // Known keyboard bots
        let bot_cases: Vec<(&str, &str, &str)> = vec![
            ("bot3", "TheFluff RL(1)", "TheFluff RL"),
            ("bot7", "Keimo_a_Rosca(1)", "Keimo_a_Rosca"),
            ("bot8", "ᵇᵉⁿʲi(1)", "ᵇᵉⁿʲi"),
        ];

        eprintln!("\n=== Discrete Kickoff Similarity: Bots vs Humans ===");
        eprintln!(
            "  {:<24} {:>6} {:>8} {:>10} {:>10} {:>8} {:>8}",
            "Player", "Type", "KBM?", "Kickoffs", "Similarity", "Mult", "Score"
        );
        eprintln!("  {}", "-".repeat(80));

        for (replay, bot_name, human_name) in &bot_cases {
            let results = load_and_analyze(replay);
            for (name, label) in [(bot_name, "BOT"), (human_name, "HUMAN")] {
                if let Some(r) = results.iter().find(|r| r.name == *name) {
                    let kbm = if r.steer_only_discrete && r.throttle_only_discrete {
                        "Yes"
                    } else {
                        "No"
                    };
                    let sim = r
                        .discrete_kickoff_similarity
                        .map_or("N/A".to_string(), |v| format!("{:.4}", v));
                    eprintln!(
                        "  {:<24} {:>6} {:>8} {:>10} {:>10} {:>7.3}x {:>7.3}",
                        r.name, label, kbm, r.kickoff_count, sim, r.discrete_kickoff_mult, r.bot_score
                    );
                }
            }
        }

        // Known keyboard humans (yukeo replays)
        eprintln!();
        let yukeo_replays = [
            ("assets/replays/kbm/yukeo.replay", "yukeo"),
            ("assets/replays/kbm/yukeo1.replay", "yukeo1"),
            ("assets/replays/kbm/yukeo2.replay", "yukeo2"),
            ("assets/replays/kbm/yukeo3.replay", "yukeo3"),
        ];

        for (path, cache_name) in &yukeo_replays {
            let Ok(results) = std::panic::catch_unwind(|| parse_and_analyze(path, cache_name))
            else {
                eprintln!("  {:<24} (parse failed, skipping)", cache_name);
                continue;
            };
            for r in &results {
                let kbm = if r.steer_only_discrete && r.throttle_only_discrete {
                    "Yes"
                } else {
                    "No"
                };
                let sim = r
                    .discrete_kickoff_similarity
                    .map_or("N/A".to_string(), |v| format!("{:.4}", v));
                eprintln!(
                    "  {:<24} {:>6} {:>8} {:>10} {:>10} {:>7.3}x {:>7.3}",
                    r.name, "HUMAN", kbm, r.kickoff_count, sim, r.discrete_kickoff_mult, r.bot_score
                );
            }
        }
        eprintln!();
    }
}

pub fn print_report(results: &[BotDetectionResult]) {
    let spectators: Vec<&BotDetectionResult> = results.iter().filter(|r| r.is_spectator).collect();
    let players: Vec<&BotDetectionResult> = results.iter().filter(|r| !r.is_spectator).collect();

    println!("=== Bot Detection Analysis ===");
    println!("  {}", "-".repeat(145));
    println!(
        "  {:<20} {:<8} {:>14} {:>7} {:>17} {:>7} {:>9} {:>8} {:>8} {:>10} {:>10} {:>7} {:>6}  {}",
        "Player", "Platform", "Steer Samples", "Unique", "Throttle Samples", "Unique",
        "Discrete", "PlatMult", "PreHold", "KickoffMul", "DribbleMul", "Timing", "Score", "Verdict"
    );
    println!("  {}", "-".repeat(155));

    for r in &players {
        let discrete = if r.steer_only_discrete && r.throttle_only_discrete {
            "Yes"
        } else {
            "No"
        };
        let pre_hold = if r.kickoff_count > 0 {
            format!("{}/{}", r.pre_hold_count, r.kickoff_count)
        } else {
            "N/A".to_string()
        };
        let kickoff_mul = if r.kickoff_consistency_mult > 1.0 {
            format!("{:.2}x", r.kickoff_consistency_mult)
        } else {
            "-".to_string()
        };
        let dribble_mul = if r.dribble_mult > 1.0 {
            format!("{:.2}x", r.dribble_mult)
        } else {
            "-".to_string()
        };
        let timing = if r.used_timing_path {
            format!("{:.2}*", r.timing_bot_score.unwrap_or(0.0))
        } else if r.timing_bot_score.is_some() {
            format!("{:.2}", r.timing_bot_score.unwrap())
        } else {
            "-".to_string()
        };
        println!(
            "  {:<20} {:<8} {:>14} {:>7} {:>17} {:>7} {:>9} {:>7.2}x {:>8} {:>10} {:>10} {:>7} {:>5.2}  {}",
            r.name,
            r.platform,
            r.total_steer_updates,
            r.unique_steer_count,
            r.total_throttle_updates,
            r.unique_throttle_count,
            discrete,
            r.platform_multiplier,
            pre_hold,
            kickoff_mul,
            dribble_mul,
            timing,
            r.bot_score,
            r.verdict,
        );
    }

    if !spectators.is_empty() {
        println!();
        println!("  Spectators (no inputs recorded):");
        for r in &spectators {
            println!("    {} ({})", r.name, r.platform);
        }
    }

    println!();
    println!("  * = timing path used (keyboard player scored via input synchrony analysis)");
}
