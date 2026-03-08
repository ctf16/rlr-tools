use serde_json::{json, Value};
use std::collections::HashMap;
use std::error;

pub struct BoostAnalysisResult {
    pub name: String,
    pub avg_boost: f64,
    pub zero_boost_pct: f64,
    pub full_boost_pct: f64,
    pub boost_collected: f64,
    pub boost_consumed: f64,
    pub big_pad_pickups: usize,
    pub small_pad_pickups: usize,
    pub total_samples: usize,
}

fn resolve_object_id(objects: &[Value], needle: &str) -> Option<u64> {
    objects
        .iter()
        .position(|o| o.as_str().map_or(false, |s| s == needle))
        .map(|i| i as u64)
}

/// Big pad pickup: boost jumps to 255 with a grant_count increment.
const BIG_PAD_BOOST: u8 = 255;
/// Small pads grant ~12% boost (~30 on the 0-255 scale).
const SMALL_PAD_THRESHOLD: u8 = 10;
/// Heuristic for detecting big pad pickups when grant_count is unavailable:
/// boost increases by at least this much in one update.
const BIG_PAD_JUMP: u8 = 80;

pub fn analyze(parsed_json: &Value) -> Result<Vec<BoostAnalysisResult>, Box<dyn error::Error>> {
    let objects = parsed_json["objects"]
        .as_array()
        .ok_or("missing objects array")?;

    // Newer replays use ReplicatedBoost (has grant_count + boost_amount).
    // Older replays use ReplicatedBoostAmount (simple Byte value).
    let replicated_boost_oid =
        resolve_object_id(objects, "TAGame.CarComponent_Boost_TA:ReplicatedBoost");
    let boost_amount_oid =
        resolve_object_id(objects, "TAGame.CarComponent_Boost_TA:ReplicatedBoostAmount");

    if replicated_boost_oid.is_none() && boost_amount_oid.is_none() {
        return Err("no boost data found (neither ReplicatedBoost nor ReplicatedBoostAmount)".into());
    }

    let vehicle_oid = resolve_object_id(objects, "TAGame.CarComponent_TA:Vehicle")
        .ok_or("Vehicle link not found")?;
    let pri_link_oid = resolve_object_id(objects, "Engine.Pawn:PlayerReplicationInfo")
        .ok_or("PlayerReplicationInfo not found")?;
    let name_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:PlayerName")
        .ok_or("PlayerName not found")?;

    let frames = parsed_json["network_frames"]["frames"]
        .as_array()
        .ok_or("missing network_frames.frames")?;

    let mut player_names: HashMap<u64, String> = HashMap::new();
    let mut car_to_player: HashMap<u64, u64> = HashMap::new();
    let mut boost_to_car: HashMap<u64, u64> = HashMap::new();

    struct BoostTracker {
        samples: Vec<u8>,
        last_boost: u8,
        last_grant_count: u32,
        big_pad_pickups: usize,
        small_pad_pickups: usize,
        boost_collected: f64,
        boost_consumed: f64,
    }

    let mut trackers: HashMap<u64, BoostTracker> = HashMap::new();

    // Helper to record a boost update into a tracker.
    let record_boost =
        |tracker: &mut BoostTracker, boost_val: u8, grant_count: Option<u32>| {
            // Detect pickups.
            match grant_count {
                Some(gc) if gc > tracker.last_grant_count => {
                    if boost_val == BIG_PAD_BOOST {
                        tracker.big_pad_pickups += 1;
                    } else if boost_val > tracker.last_boost
                        && (boost_val - tracker.last_boost) >= SMALL_PAD_THRESHOLD
                    {
                        tracker.small_pad_pickups += 1;
                    }
                    tracker.last_grant_count = gc;
                }
                None => {
                    // No grant_count available (old format) — use heuristic.
                    if boost_val > tracker.last_boost {
                        let jump = boost_val - tracker.last_boost;
                        if boost_val == BIG_PAD_BOOST && jump >= BIG_PAD_JUMP {
                            tracker.big_pad_pickups += 1;
                        } else if jump >= SMALL_PAD_THRESHOLD {
                            tracker.small_pad_pickups += 1;
                        }
                    }
                }
                _ => {}
            }

            if boost_val > tracker.last_boost {
                tracker.boost_collected += (boost_val - tracker.last_boost) as f64;
            } else if boost_val < tracker.last_boost {
                tracker.boost_consumed += (tracker.last_boost - boost_val) as f64;
            }

            tracker.samples.push(boost_val);
            tracker.last_boost = boost_val;
        };

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
                }
            } else if object_id == pri_link_oid {
                if let Some(player_actor_id) = attr["ActiveActor"]["actor"].as_u64() {
                    car_to_player.insert(actor_id, player_actor_id);
                }
            } else if object_id == vehicle_oid {
                if let Some(car_actor_id) = attr["ActiveActor"]["actor"].as_u64() {
                    boost_to_car.insert(actor_id, car_actor_id);
                }
            } else if Some(object_id) == replicated_boost_oid {
                // Newer format: ReplicatedBoost { grant_count, boost_amount, ... }
                let rb = &attr["ReplicatedBoost"];
                let Some(boost_amount) = rb["boost_amount"].as_u64() else {
                    continue;
                };
                let grant_count = rb["grant_count"].as_u64().unwrap_or(0) as u32;
                let boost_val = boost_amount as u8;

                let Some(&car_id) = boost_to_car.get(&actor_id) else {
                    continue;
                };
                let Some(&player_id) = car_to_player.get(&car_id) else {
                    continue;
                };

                let tracker = trackers.entry(player_id).or_insert_with(|| BoostTracker {
                    samples: Vec::new(),
                    last_boost: 85,
                    last_grant_count: 0,
                    big_pad_pickups: 0,
                    small_pad_pickups: 0,
                    boost_collected: 0.0,
                    boost_consumed: 0.0,
                });
                record_boost(tracker, boost_val, Some(grant_count));
            } else if Some(object_id) == boost_amount_oid {
                // Older format: ReplicatedBoostAmount as Byte
                let Some(byte_val) = attr["Byte"].as_u64() else {
                    continue;
                };
                let boost_val = byte_val as u8;

                let Some(&car_id) = boost_to_car.get(&actor_id) else {
                    continue;
                };
                let Some(&player_id) = car_to_player.get(&car_id) else {
                    continue;
                };

                let tracker = trackers.entry(player_id).or_insert_with(|| BoostTracker {
                    samples: Vec::new(),
                    last_boost: 85,
                    last_grant_count: 0,
                    big_pad_pickups: 0,
                    small_pad_pickups: 0,
                    boost_collected: 0.0,
                    boost_consumed: 0.0,
                });
                record_boost(tracker, boost_val, None);
            }
        }
    }

    let mut results: Vec<BoostAnalysisResult> = trackers
        .into_iter()
        .map(|(player_id, tracker)| {
            let name = player_names
                .get(&player_id)
                .cloned()
                .unwrap_or_else(|| format!("Actor_{}", player_id));

            let total = tracker.samples.len();
            let avg_boost = if total > 0 {
                tracker.samples.iter().map(|&v| v as f64).sum::<f64>() / total as f64
            } else {
                0.0
            };
            let zero_count = tracker.samples.iter().filter(|&&v| v == 0).count();
            let full_count = tracker.samples.iter().filter(|&&v| v == 255).count();

            BoostAnalysisResult {
                name,
                avg_boost: avg_boost / 255.0 * 100.0,
                zero_boost_pct: if total > 0 {
                    zero_count as f64 / total as f64 * 100.0
                } else {
                    0.0
                },
                full_boost_pct: if total > 0 {
                    full_count as f64 / total as f64 * 100.0
                } else {
                    0.0
                },
                boost_collected: tracker.boost_collected / 255.0 * 100.0,
                boost_consumed: tracker.boost_consumed / 255.0 * 100.0,
                big_pad_pickups: tracker.big_pad_pickups,
                small_pad_pickups: tracker.small_pad_pickups,
                total_samples: total,
            }
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

impl BoostAnalysisResult {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "avg_boost": self.avg_boost,
            "zero_boost_pct": self.zero_boost_pct,
            "full_boost_pct": self.full_boost_pct,
            "boost_collected": self.boost_collected,
            "boost_consumed": self.boost_consumed,
            "big_pad_pickups": self.big_pad_pickups,
            "small_pad_pickups": self.small_pad_pickups,
            "total_samples": self.total_samples,
        })
    }
}

pub fn results_to_json(results: &[BoostAnalysisResult]) -> Value {
    json!({
        "players": results.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
    })
}

pub fn print_report(results: &[BoostAnalysisResult]) {
    println!("=== Boost Analysis ===");
    println!(
        "  {:<24} {:>8} {:>8} {:>8} {:>10} {:>10} {:>8} {:>8} {:>8}",
        "Player", "AvgBoost", "AtZero%", "AtFull%", "Collected", "Consumed", "BigPads", "SmPads", "Samples"
    );
    println!("  {}", "-".repeat(100));

    for r in results {
        println!(
            "  {:<24} {:>7.1}% {:>7.1}% {:>7.1}% {:>9.0}% {:>9.0}% {:>8} {:>8} {:>8}",
            r.name,
            r.avg_boost,
            r.zero_boost_pct,
            r.full_boost_pct,
            r.boost_collected,
            r.boost_consumed,
            r.big_pad_pickups,
            r.small_pad_pickups,
            r.total_samples,
        );
    }
}
