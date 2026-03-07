use serde_json::Value;
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
    pub unique_steer_count: usize,
    pub unique_throttle_count: usize,
    pub total_steer_updates: usize,
    pub total_throttle_updates: usize,
    pub steer_only_discrete: bool,
    pub throttle_only_discrete: bool,
    pub input_score: f64,
    pub platform_multiplier: f64,
    pub bot_score: f64,
    pub verdict: &'static str,
}

const DISCRETE_VALUES: [u8; 3] = [0, 128, 255];

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

    let mut results: Vec<BotDetectionResult> = profiles
        .into_values()
        .map(|profile| {
            let unique_steer: HashSet<u8> = profile.steer_values.iter().copied().collect();
            let unique_throttle: HashSet<u8> = profile.throttle_values.iter().copied().collect();

            let steer_only_discrete = is_discrete_only(&profile.steer_values);
            let throttle_only_discrete = is_discrete_only(&profile.throttle_values);

            let has_enough_samples = profile.steer_values.len() >= 10;

            let input_score = if !has_enough_samples {
                0.0
            } else if steer_only_discrete && throttle_only_discrete {
                1.0
            } else if unique_steer.len() <= 5 || unique_throttle.len() <= 5 {
                0.6
            } else {
                0.0
            };

            let platform = platform_lookup
                .get(&profile.name)
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            let plat_mult = platform_multiplier(&platform);
            let bot_score = input_score * plat_mult;

            let verdict = if bot_score >= 0.9 {
                "Bot"
            } else if bot_score >= 0.5 {
                "Likely Bot"
            } else {
                "Human"
            };

            BotDetectionResult {
                name: profile.name,
                platform,
                unique_steer_count: unique_steer.len(),
                unique_throttle_count: unique_throttle.len(),
                total_steer_updates: profile.steer_values.len(),
                total_throttle_updates: profile.throttle_values.len(),
                steer_only_discrete,
                throttle_only_discrete,
                input_score,
                platform_multiplier: plat_mult,
                bot_score,
                verdict,
            }
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

pub fn print_report(results: &[BotDetectionResult]) {
    println!("=== Bot Detection Analysis ===");
    println!(
        "  {:<20} {:<8} {:>14} {:>7} {:>17} {:>7} {:>9} {:>8} {:>6}  {}",
        "Player", "Platform", "Steer Samples", "Unique", "Throttle Samples", "Unique",
        "Discrete", "PlatMult", "Score", "Verdict"
    );
    println!("  {}", "-".repeat(112));

    for r in results {
        let discrete = if r.steer_only_discrete && r.throttle_only_discrete {
            "Yes"
        } else {
            "No"
        };
        println!(
            "  {:<20} {:<8} {:>14} {:>7} {:>17} {:>7} {:>9} {:>7.2}x {:>5.2}  {}",
            r.name,
            r.platform,
            r.total_steer_updates,
            r.unique_steer_count,
            r.total_throttle_updates,
            r.unique_throttle_count,
            discrete,
            r.platform_multiplier,
            r.bot_score,
            r.verdict,
        );
    }
}
