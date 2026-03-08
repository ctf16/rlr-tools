use serde_json::{json, Value};
use std::collections::HashMap;
use std::error;

/// A single kickoff window: from countdown=0 until ball is hit (or a fixed frame cap).
struct KickoffWindow {
    start_frame_idx: usize,
    ball_hit_frame_idx: Option<usize>,
    /// Per car actor_id: ordered input samples within this kickoff window.
    player_inputs: HashMap<u64, KickoffInputs>,
}

struct KickoffInputs {
    /// (frame_offset_from_start, steer_value)
    steer: Vec<(usize, u8)>,
    /// (frame_offset_from_start, throttle_value)
    throttle: Vec<(usize, u8)>,
}

pub struct KickoffAnalysisResult {
    pub name: String,
    pub kickoff_count: usize,
    /// Reaction latency in frames for each kickoff (None if player had no input change).
    pub reaction_frames: Vec<Option<usize>>,
    /// Mean reaction latency in frames.
    pub mean_reaction: Option<f64>,
    /// Standard deviation of reaction latency.
    pub reaction_stddev: Option<f64>,
    /// How many kickoffs had throttle already at 255 on the first frame (pre-holding).
    pub pre_hold_count: usize,
    /// Average pairwise distance of steer sequences across kickoffs (0.0=identical, 1.0=max different).
    pub steer_variability: Option<f64>,
    /// Average pairwise distance of throttle sequences across kickoffs.
    pub throttle_variability: Option<f64>,
}

fn resolve_object_id(objects: &[Value], needle: &str) -> Option<u64> {
    objects
        .iter()
        .position(|o| o.as_str().map_or(false, |s| s == needle))
        .map(|i| i as u64)
}

/// Compute mean and stddev of a slice of f64 values.
fn mean_stddev(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    (mean, variance.sqrt())
}

/// Normalized mean absolute difference between two input sequences.
/// Returns 0.0 for identical sequences, 1.0 for maximally different.
/// Sequences are centered (subtract 128) before comparison.
fn sequence_distance(a: &[u8], b: &[u8]) -> f64 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 1.0;
    }
    let sum_diff: f64 = a[..len]
        .iter()
        .zip(b[..len].iter())
        .map(|(&x, &y)| (x as f64 - y as f64).abs())
        .sum();
    sum_diff / (len as f64 * 255.0)
}

/// Maximum number of frames after countdown=0 to consider part of the kickoff.
const KICKOFF_WINDOW_FRAMES: usize = 200;

pub fn analyze(parsed_json: &Value) -> Result<Vec<KickoffAnalysisResult>, Box<dyn error::Error>> {
    let objects = parsed_json["objects"]
        .as_array()
        .ok_or("missing objects array")?;

    let steer_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedSteer")
        .ok_or("ReplicatedSteer not found")?;
    let throttle_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedThrottle")
        .ok_or("ReplicatedThrottle not found")?;
    let countdown_oid =
        resolve_object_id(objects, "TAGame.GameEvent_TA:ReplicatedRoundCountDownNumber")
            .ok_or("CountDown not found")?;
    let ball_hit_oid =
        resolve_object_id(objects, "TAGame.GameEvent_Soccar_TA:bBallHasBeenHit");
    let pri_link_oid = resolve_object_id(objects, "Engine.Pawn:PlayerReplicationInfo")
        .ok_or("PlayerReplicationInfo not found")?;
    let name_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:PlayerName")
        .ok_or("PlayerName not found")?;

    let frames = parsed_json["network_frames"]["frames"]
        .as_array()
        .ok_or("missing network_frames.frames")?;

    // First pass: build player name map and car-to-player mapping.
    let mut player_names: HashMap<u64, String> = HashMap::new();
    let mut car_to_player: HashMap<u64, u64> = HashMap::new();

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
            }
        }
    }

    // Second pass: find kickoff windows and collect per-player inputs within them.
    // Detect kickoff starts by countdown transitioning to 0.
    let mut kickoff_windows: Vec<KickoffWindow> = Vec::new();
    let mut last_countdown: Option<i64> = None;
    let mut current_kickoff: Option<usize> = None; // index into kickoff_windows

    for (frame_idx, frame) in frames.iter().enumerate() {
        let Some(updated) = frame["updated_actors"].as_array() else {
            continue;
        };

        // Check if we're past the kickoff window cap.
        if let Some(ko_idx) = current_kickoff {
            let ko = &kickoff_windows[ko_idx];
            if frame_idx - ko.start_frame_idx > KICKOFF_WINDOW_FRAMES {
                current_kickoff = None;
            }
        }

        for actor in updated {
            let actor_id = actor["actor_id"].as_u64().unwrap_or(u64::MAX);
            let object_id = actor["object_id"].as_u64().unwrap_or(u64::MAX);
            let attr = &actor["attribute"];

            // Detect countdown transitions.
            if object_id == countdown_oid {
                if let Some(val) = attr["Int"].as_i64() {
                    if val == 0 && last_countdown.map_or(true, |prev| prev != 0) {
                        // New kickoff starting.
                        let ko = KickoffWindow {
                            start_frame_idx: frame_idx,
                            ball_hit_frame_idx: None,
                            player_inputs: HashMap::new(),
                        };
                        kickoff_windows.push(ko);
                        current_kickoff = Some(kickoff_windows.len() - 1);
                    }
                    last_countdown = Some(val);
                }
            }

            // Detect ball hit.
            if let Some(bh_oid) = ball_hit_oid {
                if object_id == bh_oid {
                    if let Some(true) = attr["Boolean"].as_bool() {
                        if let Some(ko_idx) = current_kickoff {
                            if kickoff_windows[ko_idx].ball_hit_frame_idx.is_none() {
                                kickoff_windows[ko_idx].ball_hit_frame_idx = Some(frame_idx);
                            }
                        }
                    }
                }
            }

            // Collect inputs during active kickoff window.
            if let Some(ko_idx) = current_kickoff {
                let offset = frame_idx - kickoff_windows[ko_idx].start_frame_idx;

                if object_id == steer_oid {
                    if let Some(byte_val) = attr["Byte"].as_u64() {
                        if let Some(&player_id) = car_to_player.get(&actor_id) {
                            let inputs = kickoff_windows[ko_idx]
                                .player_inputs
                                .entry(player_id)
                                .or_insert_with(|| KickoffInputs {
                                    steer: Vec::new(),
                                    throttle: Vec::new(),
                                });
                            inputs.steer.push((offset, byte_val as u8));
                        }
                    }
                } else if object_id == throttle_oid {
                    if let Some(byte_val) = attr["Byte"].as_u64() {
                        if let Some(&player_id) = car_to_player.get(&actor_id) {
                            let inputs = kickoff_windows[ko_idx]
                                .player_inputs
                                .entry(player_id)
                                .or_insert_with(|| KickoffInputs {
                                    steer: Vec::new(),
                                    throttle: Vec::new(),
                                });
                            inputs.throttle.push((offset, byte_val as u8));
                        }
                    }
                }
            }
        }
    }

    // Build results per player.
    // Gather all player IDs that appear in any kickoff.
    let mut all_player_ids: Vec<u64> = kickoff_windows
        .iter()
        .flat_map(|ko| ko.player_inputs.keys().copied())
        .collect::<std::collections::HashSet<u64>>()
        .into_iter()
        .collect();
    all_player_ids.sort();

    let mut results: Vec<KickoffAnalysisResult> = Vec::new();

    for &player_id in &all_player_ids {
        let name = player_names
            .get(&player_id)
            .cloned()
            .unwrap_or_else(|| format!("Actor_{}", player_id));

        let mut reaction_frames: Vec<Option<usize>> = Vec::new();
        let mut pre_hold_count: usize = 0;
        let mut steer_sequences: Vec<Vec<u8>> = Vec::new();
        let mut throttle_sequences: Vec<Vec<u8>> = Vec::new();

        for ko in &kickoff_windows {
            let Some(inputs) = ko.player_inputs.get(&player_id) else {
                reaction_frames.push(None);
                continue;
            };

            // Reaction latency: first throttle value that is not 128 (neutral).
            let first_non_neutral_throttle = inputs
                .throttle
                .iter()
                .find(|(_, val)| *val != 128);

            match first_non_neutral_throttle {
                Some(&(offset, val)) => {
                    if offset == 0 && val == 255 {
                        pre_hold_count += 1;
                    }
                    reaction_frames.push(Some(offset));
                }
                None => {
                    reaction_frames.push(None);
                }
            }

            // Build input sequences for consistency analysis.
            // Interpolate sparse updates into per-frame sequences.
            let end_frame = ko
                .ball_hit_frame_idx
                .map(|f| f.saturating_sub(ko.start_frame_idx))
                .unwrap_or(KICKOFF_WINDOW_FRAMES)
                .min(KICKOFF_WINDOW_FRAMES);

            // Helper to interpolate sparse (offset, value) samples into a per-frame sequence.
            let interpolate = |samples: &[(usize, u8)], len: usize| -> Vec<u8> {
                let mut seq = vec![128u8; len];
                let mut last_val = 128u8;
                let mut idx = 0;
                for frame_offset in 0..len {
                    while idx < samples.len() && samples[idx].0 <= frame_offset {
                        last_val = samples[idx].1;
                        idx += 1;
                    }
                    seq[frame_offset] = last_val;
                }
                seq
            };

            let steer_seq = interpolate(&inputs.steer, end_frame);
            let throttle_seq = interpolate(&inputs.throttle, end_frame);

            if steer_seq.len() >= 5 {
                steer_sequences.push(steer_seq);
            }
            if throttle_seq.len() >= 5 {
                throttle_sequences.push(throttle_seq);
            }
        }

        // Compute mean reaction latency.
        let valid_reactions: Vec<f64> = reaction_frames
            .iter()
            .filter_map(|r| r.map(|v| v as f64))
            .collect();
        let (mean_reaction, reaction_stddev) = if valid_reactions.len() >= 2 {
            let (m, s) = mean_stddev(&valid_reactions);
            (Some(m), Some(s))
        } else {
            (None, None)
        };

        // Compute average pairwise distance for steer and throttle sequences.
        let avg_pairwise_distance = |seqs: &[Vec<u8>]| -> Option<f64> {
            if seqs.len() < 2 {
                return None;
            }
            let mut distances = Vec::new();
            for i in 0..seqs.len() {
                for j in (i + 1)..seqs.len() {
                    distances.push(sequence_distance(&seqs[i], &seqs[j]));
                }
            }
            Some(distances.iter().sum::<f64>() / distances.len() as f64)
        };

        let steer_variability = avg_pairwise_distance(&steer_sequences);
        let throttle_variability = avg_pairwise_distance(&throttle_sequences);

        results.push(KickoffAnalysisResult {
            name,
            kickoff_count: kickoff_windows.len(),
            reaction_frames,
            mean_reaction,
            reaction_stddev,
            pre_hold_count,
            steer_variability,
            throttle_variability,
        });
    }

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

impl KickoffAnalysisResult {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "kickoff_count": self.kickoff_count,
            "reaction_frames": self.reaction_frames,
            "mean_reaction": self.mean_reaction,
            "reaction_stddev": self.reaction_stddev,
            "pre_hold_count": self.pre_hold_count,
            "steer_variability": self.steer_variability,
            "throttle_variability": self.throttle_variability,
        })
    }
}

pub fn results_to_json(results: &[KickoffAnalysisResult]) -> Value {
    let kickoff_count = results.first().map_or(0, |r| r.kickoff_count);
    json!({
        "kickoff_count": kickoff_count,
        "players": results.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
    })
}

pub fn print_report(results: &[KickoffAnalysisResult]) {
    println!("=== Kickoff Analysis ===");
    println!(
        "  Kickoffs detected: {}",
        results.first().map_or(0, |r| r.kickoff_count)
    );
    println!();
    println!(
        "  {:<24} {:>10} {:>10} {:>10} {:>12} {:>12}",
        "Player", "MeanReact", "StdDev", "PreHold", "SteerVar", "ThrottleVar"
    );
    println!("  {}", "-".repeat(82));

    for r in results {
        let mean_str = r
            .mean_reaction
            .map_or("N/A".to_string(), |m| format!("{:.1}", m));
        let std_str = r
            .reaction_stddev
            .map_or("N/A".to_string(), |s| format!("{:.1}", s));
        let steer_var = r
            .steer_variability
            .map_or("N/A".to_string(), |v| format!("{:.4}", v));
        let throttle_var = r
            .throttle_variability
            .map_or("N/A".to_string(), |v| format!("{:.4}", v));

        println!(
            "  {:<24} {:>10} {:>10} {:>8}/{:<1} {:>12} {:>12}",
            r.name,
            mean_str,
            std_str,
            r.pre_hold_count,
            r.kickoff_count,
            steer_var,
            throttle_var,
        );
    }

    println!();
    println!("  Per-kickoff reaction (frames from countdown=0 to first non-neutral throttle):");
    for r in results {
        let reactions: String = r
            .reaction_frames
            .iter()
            .map(|f| match f {
                Some(v) => format!("{:>3}", v),
                None => "  -".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        println!("  {:<24} [{}]", r.name, reactions);
    }
}
