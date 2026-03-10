use serde_json::{json, Value};
use std::collections::HashMap;
use std::error;

const DISCRETE_VALUES: [u8; 3] = [0, 128, 255];

struct InputEvent {
    frame_idx: usize,
    time: f64,
    value: u8,
}

struct PlayerInputTimeline {
    steer_events: Vec<InputEvent>,
    throttle_events: Vec<InputEvent>,
}

pub struct InputSynchronyResult {
    pub name: String,
    pub is_discrete_only: bool,

    // Alternation: value changes per second
    pub steer_alternation_rate: f64,
    pub throttle_alternation_rate: f64,

    // Hold duration stats (seconds)
    pub steer_hold_mean: f64,
    pub steer_hold_stddev: f64,
    pub steer_hold_cv: f64,
    pub throttle_hold_mean: f64,
    pub throttle_hold_stddev: f64,
    pub throttle_hold_cv: f64,

    // Multi-input synchrony
    pub simultaneous_changes: usize,
    pub total_change_frames: usize,
    pub simultaneous_change_rate: f64,

    // Composite timing-based bot score (0.0 = human, 1.0 = bot)
    pub timing_bot_score: f64,
}

fn resolve_object_id(objects: &[Value], needle: &str) -> Option<u64> {
    objects
        .iter()
        .position(|o| o.as_str().map_or(false, |s| s == needle))
        .map(|i| i as u64)
}

fn is_discrete_only(events: &[InputEvent]) -> bool {
    events.iter().all(|e| DISCRETE_VALUES.contains(&e.value))
}

/// Count actual value transitions (consecutive events with different values) per second.
fn alternation_rate(events: &[InputEvent]) -> f64 {
    if events.len() < 2 {
        return 0.0;
    }
    let time_span = events.last().unwrap().time - events.first().unwrap().time;
    if time_span <= 0.0 {
        return 0.0;
    }
    let changes = events
        .windows(2)
        .filter(|w| w[0].value != w[1].value)
        .count();
    changes as f64 / time_span
}

/// Compute hold durations in seconds between consecutive value changes.
fn hold_durations(events: &[InputEvent]) -> Vec<f64> {
    events
        .windows(2)
        .filter(|w| w[0].value != w[1].value)
        .map(|w| w[1].time - w[0].time)
        .filter(|&d| d > 0.0)
        .collect()
}

fn mean_and_stddev(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    (mean, variance.sqrt())
}

/// Coefficient of variation (stddev / mean). Returns 0.0 if mean is ~zero.
fn cv(mean: f64, stddev: f64) -> f64 {
    if mean.abs() < 1e-9 {
        0.0
    } else {
        stddev / mean
    }
}

/// Count frames where multiple input channels changed simultaneously.
/// Returns (simultaneous_change_frames, total_change_frames).
fn compute_synchrony(
    steer_events: &[InputEvent],
    throttle_events: &[InputEvent],
) -> (usize, usize) {
    let mut changes_per_frame: HashMap<usize, usize> = HashMap::new();

    for events in [steer_events, throttle_events] {
        for w in events.windows(2) {
            if w[0].value != w[1].value {
                *changes_per_frame.entry(w[1].frame_idx).or_insert(0) += 1;
            }
        }
    }

    let total = changes_per_frame.len();
    let simultaneous = changes_per_frame.values().filter(|&&c| c >= 2).count();
    (simultaneous, total)
}

// --- Scoring functions ---
// Each returns 0.0 (human) to 1.0 (bot).

fn score_alternation_rate(rate: f64) -> f64 {
    if rate < 5.0 {
        0.0
    } else if rate < 8.0 {
        0.2
    } else if rate < 12.0 {
        0.5
    } else if rate < 20.0 {
        0.8
    } else {
        1.0
    }
}

fn score_hold_cv(cv_val: f64) -> f64 {
    // Low CV = uniform hold durations = bot-like
    if cv_val > 1.0 {
        0.0
    } else if cv_val > 0.6 {
        0.2
    } else if cv_val > 0.3 {
        0.5
    } else if cv_val > 0.1 {
        0.8
    } else {
        1.0
    }
}

fn score_synchrony_rate(rate: f64) -> f64 {
    if rate < 0.15 {
        0.0
    } else if rate < 0.30 {
        0.2
    } else if rate < 0.50 {
        0.5
    } else if rate < 0.70 {
        0.8
    } else {
        1.0
    }
}

pub fn analyze(parsed_json: &Value) -> Result<Vec<InputSynchronyResult>, Box<dyn error::Error>> {
    let objects = parsed_json["objects"]
        .as_array()
        .ok_or("missing objects array")?;

    let steer_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedSteer")
        .ok_or("ReplicatedSteer not found")?;
    let throttle_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedThrottle")
        .ok_or("ReplicatedThrottle not found")?;
    let pri_link_oid = resolve_object_id(objects, "Engine.Pawn:PlayerReplicationInfo")
        .ok_or("PlayerReplicationInfo not found")?;
    let name_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:PlayerName")
        .ok_or("PlayerName not found")?;

    let frames = parsed_json["network_frames"]["frames"]
        .as_array()
        .ok_or("missing network_frames.frames")?;

    let mut player_names: HashMap<u64, String> = HashMap::new();
    let mut car_to_player: HashMap<u64, u64> = HashMap::new();
    let mut timelines: HashMap<u64, PlayerInputTimeline> = HashMap::new();

    for (frame_idx, frame) in frames.iter().enumerate() {
        let frame_time = frame["time"].as_f64().unwrap_or(0.0);

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
                    timelines.entry(actor_id).or_insert_with(|| PlayerInputTimeline {
                        steer_events: Vec::new(),
                        throttle_events: Vec::new(),
                    });
                }
            } else if object_id == pri_link_oid {
                if let Some(player_actor_id) = attr["ActiveActor"]["actor"].as_u64() {
                    car_to_player.insert(actor_id, player_actor_id);
                }
            } else if object_id == steer_oid {
                if let Some(byte_val) = attr["Byte"].as_u64() {
                    if let Some(&player_id) = car_to_player.get(&actor_id) {
                        if let Some(timeline) = timelines.get_mut(&player_id) {
                            timeline.steer_events.push(InputEvent {
                                frame_idx,
                                time: frame_time,
                                value: byte_val as u8,
                            });
                        }
                    }
                }
            } else if object_id == throttle_oid {
                if let Some(byte_val) = attr["Byte"].as_u64() {
                    if let Some(&player_id) = car_to_player.get(&actor_id) {
                        if let Some(timeline) = timelines.get_mut(&player_id) {
                            timeline.throttle_events.push(InputEvent {
                                frame_idx,
                                time: frame_time,
                                value: byte_val as u8,
                            });
                        }
                    }
                }
            }
        }
    }

    let mut results: Vec<InputSynchronyResult> = timelines
        .into_iter()
        .map(|(player_id, timeline)| {
            let name = player_names
                .get(&player_id)
                .cloned()
                .unwrap_or_else(|| format!("Actor_{}", player_id));

            let discrete_only = is_discrete_only(&timeline.steer_events)
                && is_discrete_only(&timeline.throttle_events);

            let steer_alt_rate = alternation_rate(&timeline.steer_events);
            let throttle_alt_rate = alternation_rate(&timeline.throttle_events);

            let steer_holds = hold_durations(&timeline.steer_events);
            let (steer_h_mean, steer_h_std) = mean_and_stddev(&steer_holds);
            let steer_h_cv = cv(steer_h_mean, steer_h_std);

            let throttle_holds = hold_durations(&timeline.throttle_events);
            let (throttle_h_mean, throttle_h_std) = mean_and_stddev(&throttle_holds);
            let throttle_h_cv = cv(throttle_h_mean, throttle_h_std);

            let (simultaneous, total_change_frames) =
                compute_synchrony(&timeline.steer_events, &timeline.throttle_events);
            let sync_rate = if total_change_frames > 0 {
                simultaneous as f64 / total_change_frames as f64
            } else {
                0.0
            };

            // Composite timing score: use the more suspicious signal from each pair.
            let max_alt_rate = steer_alt_rate.max(throttle_alt_rate);
            let min_hold_cv = steer_h_cv.min(throttle_h_cv);

            let alt_score = score_alternation_rate(max_alt_rate);
            let hold_score = score_hold_cv(min_hold_cv);
            let sync_score = score_synchrony_rate(sync_rate);

            let has_enough_data =
                timeline.steer_events.len() >= 10 && timeline.throttle_events.len() >= 10;

            let timing_bot_score = if has_enough_data {
                (0.45 * alt_score + 0.35 * hold_score + 0.20 * sync_score).min(1.0)
            } else {
                0.0
            };

            InputSynchronyResult {
                name,
                is_discrete_only: discrete_only,
                steer_alternation_rate: steer_alt_rate,
                throttle_alternation_rate: throttle_alt_rate,
                steer_hold_mean: steer_h_mean,
                steer_hold_stddev: steer_h_std,
                steer_hold_cv: steer_h_cv,
                throttle_hold_mean: throttle_h_mean,
                throttle_hold_stddev: throttle_h_std,
                throttle_hold_cv: throttle_h_cv,
                simultaneous_changes: simultaneous,
                total_change_frames,
                simultaneous_change_rate: sync_rate,
                timing_bot_score,
            }
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

impl InputSynchronyResult {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "is_discrete_only": self.is_discrete_only,
            "steer_alternation_rate": (self.steer_alternation_rate * 100.0).round() / 100.0,
            "throttle_alternation_rate": (self.throttle_alternation_rate * 100.0).round() / 100.0,
            "steer_hold_mean": (self.steer_hold_mean * 1000.0).round() / 1000.0,
            "steer_hold_stddev": (self.steer_hold_stddev * 1000.0).round() / 1000.0,
            "steer_hold_cv": (self.steer_hold_cv * 100.0).round() / 100.0,
            "throttle_hold_mean": (self.throttle_hold_mean * 1000.0).round() / 1000.0,
            "throttle_hold_stddev": (self.throttle_hold_stddev * 1000.0).round() / 1000.0,
            "throttle_hold_cv": (self.throttle_hold_cv * 100.0).round() / 100.0,
            "simultaneous_changes": self.simultaneous_changes,
            "total_change_frames": self.total_change_frames,
            "simultaneous_change_rate": (self.simultaneous_change_rate * 1000.0).round() / 1000.0,
            "timing_bot_score": (self.timing_bot_score * 100.0).round() / 100.0,
        })
    }
}

pub fn results_to_json(results: &[InputSynchronyResult]) -> Value {
    json!({
        "players": results.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
    })
}

pub fn print_report(results: &[InputSynchronyResult]) {
    println!("=== Input Synchrony Analysis ===");
    println!(
        "  {:<20} {:>5} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>8} {:>6}",
        "Player", "KBM?", "SteerΔ/s", "ThrotΔ/s", "SteerHold", "SteerCV",
        "ThrotHold", "ThrotCV", "Sync%", "TScore"
    );
    println!("  {}", "-".repeat(110));

    for r in results {
        let kbm = if r.is_discrete_only { "Yes" } else { "No" };
        let steer_hold = if r.steer_hold_mean > 0.0 {
            format!("{:.3}s", r.steer_hold_mean)
        } else {
            "N/A".to_string()
        };
        let throttle_hold = if r.throttle_hold_mean > 0.0 {
            format!("{:.3}s", r.throttle_hold_mean)
        } else {
            "N/A".to_string()
        };
        let steer_cv = if r.steer_hold_mean > 0.0 {
            format!("{:.2}", r.steer_hold_cv)
        } else {
            "N/A".to_string()
        };
        let throttle_cv = if r.throttle_hold_mean > 0.0 {
            format!("{:.2}", r.throttle_hold_cv)
        } else {
            "N/A".to_string()
        };

        println!(
            "  {:<20} {:>5} {:>9.1}/s {:>9.1}/s {:>10} {:>10} {:>10} {:>10} {:>7.1}% {:>5.2}",
            r.name,
            kbm,
            r.steer_alternation_rate,
            r.throttle_alternation_rate,
            steer_hold,
            steer_cv,
            throttle_hold,
            throttle_cv,
            r.simultaneous_change_rate * 100.0,
            r.timing_bot_score,
        );
    }
}
