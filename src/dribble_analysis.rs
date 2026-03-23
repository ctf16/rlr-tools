use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::error;

// Dribble geometry thresholds (RigidBody coordinate units, ~1/100 Unreal units)
const DRIBBLE_XY_MAX: f64 = 2.5; // max horizontal distance ball-to-car
const DRIBBLE_Z_MIN: f64 = 1.0; // ball must be at least this far above car
const DRIBBLE_Z_MAX: f64 = 3.5; // ball must not be higher than this above car
const DRIBBLE_MIN_FRAMES: usize = 60; // ~0.5s minimum to qualify
const DRIBBLE_GAP_TOLERANCE: usize = 15; // allow brief interruptions

// Snappy micro-correction thresholds
const SNAPPY_STEER_DELTA: u8 = 40; // minimum steer change to count as "snappy"
const SNAPPY_RETURN_FRAMES: usize = 5; // must return near neutral within this window
const SNAPPY_NEUTRAL_TOLERANCE: u8 = 15; // within ±15 of 128 counts as "returned"
const STEER_NEUTRAL: u8 = 128;

// Zero-steer (sustained neutral) threshold
const SUSTAINED_NEUTRAL_MIN: usize = 30; // ~0.25s of perfect neutral during dribble

// Flick detection thresholds
const FLICK_BALL_VZ_MIN: f64 = 5.0; // ball upward velocity spike
const FLICK_SEPARATION_SPEED: f64 = 8.0; // ball-car 3D distance increase rate
const OPPONENT_CHALLENGE_RADIUS: f64 = 20.0; // opponent within this XY distance
const OPPONENT_APPROACH_DOT_MIN: f64 = 2.0; // opponent velocity toward ball
const FLICK_TIMING_WINDOW: usize = 30; // check opponent approach within this many frames

// Minimum dribbles for a nonzero suspicion score
const MIN_DRIBBLES_FOR_SCORE: usize = 2;

// Approximate frame rate for per-second calculations
const APPROX_FPS: f64 = 120.0;

#[derive(Clone, Default)]
struct Vec3 {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Default)]
struct ActorState {
    pos: Vec3,
    vel: Vec3,
}

struct DribbleWindow {
    start_frame: usize,
    frames: usize,
    gap_frames: usize, // consecutive non-dribble frames (for gap tolerance)
    steer_history: Vec<u8>, // steer values during dribble
    active: bool,
}

struct DribbleEvent {
    _start_frame: usize,
    duration_frames: usize,
    snappy_corrections: usize,
    zero_steer_frames: usize,
    flick_detected: bool,
    flick_opponent_timed: bool,
}

pub struct PlayerDribbleResult {
    pub name: String,
    pub dribble_count: usize,
    pub total_dribble_frames: usize,
    pub avg_snappy_rate: f64, // corrections per second, averaged across dribbles
    pub avg_zero_steer_pct: f64, // average % of dribble spent at exact neutral
    pub timed_flick_count: usize,
    pub total_flick_count: usize,
    pub snappy_score: f64,
    pub zero_steer_score: f64,
    pub flick_score: f64,
    pub dribble_suspicion_score: f64,
}

fn resolve_object_id(objects: &[Value], needle: &str) -> Option<u64> {
    objects
        .iter()
        .position(|o| o.as_str().map_or(false, |s| s == needle))
        .map(|i| i as u64)
}

fn extract_rb_state_3d(attr: &Value) -> Option<ActorState> {
    let rb = &attr["RigidBody"];
    let loc = &rb["location"];
    let lv = &rb["linear_velocity"];

    let (px, py, pz) = if let (Some(x), Some(y)) = (loc["x"].as_f64(), loc["y"].as_f64()) {
        (x, y, loc["z"].as_f64().unwrap_or(0.0))
    } else if let Some(arr) = loc.as_array() {
        (
            arr.first()?.as_f64()?,
            arr.get(1)?.as_f64()?,
            arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
        )
    } else {
        return None;
    };

    let (vx, vy, vz) = if let (Some(x), Some(y)) = (lv["x"].as_f64(), lv["y"].as_f64()) {
        (x, y, lv["z"].as_f64().unwrap_or(0.0))
    } else if let Some(arr) = lv.as_array() {
        (
            arr.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
            arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
            arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0),
        )
    } else {
        (0.0, 0.0, 0.0)
    };

    Some(ActorState {
        pos: Vec3 {
            x: px,
            y: py,
            z: pz,
        },
        vel: Vec3 {
            x: vx,
            y: vy,
            z: vz,
        },
    })
}

fn distance_2d(a: &Vec3, b: &Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn distance_3d(a: &Vec3, b: &Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn dot_toward_2d(vel: &Vec3, from: &Vec3, to: &Vec3) -> f64 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.001 {
        return 0.0;
    }
    (vel.x * dx / len) + (vel.y * dy / len)
}

/// Count snappy micro-corrections in a steer history.
/// A snappy correction: steer changes by > SNAPPY_STEER_DELTA, then returns
/// within ±SNAPPY_NEUTRAL_TOLERANCE of neutral within SNAPPY_RETURN_FRAMES.
fn count_snappy_corrections(steer_history: &[u8]) -> usize {
    let mut count = 0;
    let mut i = 1;
    while i < steer_history.len() {
        let prev = steer_history[i - 1];
        let curr = steer_history[i];
        let delta = (curr as i16 - prev as i16).unsigned_abs() as u8;

        if delta >= SNAPPY_STEER_DELTA {
            // Check if steer returns near neutral within the window
            let window_end = (i + SNAPPY_RETURN_FRAMES).min(steer_history.len());
            for j in i..window_end {
                let diff_from_neutral =
                    (steer_history[j] as i16 - STEER_NEUTRAL as i16).unsigned_abs() as u8;
                if diff_from_neutral <= SNAPPY_NEUTRAL_TOLERANCE {
                    count += 1;
                    i = j + 1; // skip past the return to avoid double-counting
                    break;
                }
            }
        }
        i += 1;
    }
    count
}

/// Count frames at exactly steer=128 in sustained runs of SUSTAINED_NEUTRAL_MIN+.
fn count_sustained_neutral_frames(steer_history: &[u8]) -> usize {
    let mut total = 0;
    let mut run = 0;
    for &s in steer_history {
        if s == STEER_NEUTRAL {
            run += 1;
        } else {
            if run >= SUSTAINED_NEUTRAL_MIN {
                total += run;
            }
            run = 0;
        }
    }
    if run >= SUSTAINED_NEUTRAL_MIN {
        total += run;
    }
    total
}

fn score_snappy_rate(rate: f64) -> f64 {
    if rate >= 5.0 {
        1.0
    } else if rate >= 3.0 {
        0.7
    } else if rate >= 2.0 {
        0.3
    } else {
        0.0
    }
}

fn score_zero_steer_pct(pct: f64) -> f64 {
    if pct >= 30.0 {
        1.0
    } else if pct >= 15.0 {
        0.7
    } else if pct >= 5.0 {
        0.3
    } else {
        0.0
    }
}

fn score_flick_timing(timed: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let ratio = timed as f64 / total as f64;
    if ratio >= 0.8 && timed >= 3 {
        1.0
    } else if ratio >= 0.6 && timed >= 2 {
        0.7
    } else if ratio >= 0.5 && timed >= 1 {
        0.3
    } else {
        0.0
    }
}

pub fn analyze(parsed_json: &Value) -> Result<Vec<PlayerDribbleResult>, Box<dyn error::Error>> {
    let objects = parsed_json["objects"]
        .as_array()
        .ok_or("missing objects array")?;

    let rb_oid = resolve_object_id(objects, "TAGame.RBActor_TA:ReplicatedRBState")
        .ok_or("ReplicatedRBState not found")?;
    let pri_link_oid = resolve_object_id(objects, "Engine.Pawn:PlayerReplicationInfo")
        .ok_or("PlayerReplicationInfo not found")?;
    let name_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:PlayerName")
        .ok_or("PlayerName not found")?;
    let steer_oid = resolve_object_id(objects, "TAGame.Vehicle_TA:ReplicatedSteer")
        .ok_or("ReplicatedSteer not found")?;
    let team_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:Team")
        .ok_or("Team not found")?;

    let ball_arch_oid = resolve_object_id(objects, "Archetypes.Ball.Ball_Default");
    let car_arch_oid = resolve_object_id(objects, "Archetypes.Car.Car_Default");
    let team0_arch_oid = resolve_object_id(objects, "Archetypes.Teams.Team0");
    let team1_arch_oid = resolve_object_id(objects, "Archetypes.Teams.Team1");

    let frames = parsed_json["network_frames"]["frames"]
        .as_array()
        .ok_or("missing network_frames.frames")?;

    // Linkage maps
    let mut player_names: HashMap<u64, String> = HashMap::new();
    let mut car_to_player: HashMap<u64, u64> = HashMap::new();
    let mut player_to_team: HashMap<u64, usize> = HashMap::new();
    let mut team_actor_to_index: HashMap<u64, usize> = HashMap::new();

    let mut ball_actor_id: Option<u64> = None;
    let mut car_actor_ids: HashSet<u64> = HashSet::new();
    let mut actor_states: HashMap<u64, ActorState> = HashMap::new();
    let mut car_steer: HashMap<u64, u8> = HashMap::new(); // last-known steer per car

    // Per-car dribble window tracking
    let mut dribble_windows: HashMap<u64, DribbleWindow> = HashMap::new();
    // Completed dribble events per player actor id
    let mut player_dribble_events: HashMap<u64, Vec<DribbleEvent>> = HashMap::new();

    // Track opponent approach state for flick timing: (frame_idx, car_actor_id approaching)
    // Per-frame snapshot of whether any opponent is challenging the ball
    // We store per-team: for each frame, whether an opponent from the other team is approaching
    // Actually, we need to check at flick time, so store recent opponent approach frames per team
    let mut opponent_challenging: HashMap<usize, Vec<(u64, usize)>> = HashMap::new(); // team -> [(car_actor, frame)]

    // Z-axis availability check
    let mut seen_nonzero_z = false;
    let z_check_frames = 500.min(frames.len());

    for (frame_idx, frame) in frames.iter().enumerate() {
        // Process new_actors
        if let Some(new_actors) = frame["new_actors"].as_array() {
            for new_actor in new_actors {
                let actor_id = new_actor["actor_id"].as_u64().unwrap_or(u64::MAX);
                let object_id = new_actor["object_id"].as_u64().unwrap_or(u64::MAX);

                if Some(object_id) == ball_arch_oid {
                    ball_actor_id = Some(actor_id);
                } else if Some(object_id) == car_arch_oid {
                    car_actor_ids.insert(actor_id);
                } else if Some(object_id) == team0_arch_oid {
                    team_actor_to_index.insert(actor_id, 0);
                } else if Some(object_id) == team1_arch_oid {
                    team_actor_to_index.insert(actor_id, 1);
                }
            }
        }

        // Process deleted_actors — reset dribble windows for destroyed cars
        if let Some(deleted) = frame["deleted_actors"].as_array() {
            for del in deleted {
                if let Some(actor_id) = del.as_u64() {
                    if car_actor_ids.remove(&actor_id) {
                        // Finalize any active dribble window for this car
                        finalize_dribble_window(
                            actor_id,
                            frame_idx,
                            &car_to_player,
                            &mut dribble_windows,
                            &mut player_dribble_events,
                            None, // no flick on deletion
                            &opponent_challenging,
                            &player_to_team,
                        );
                        car_steer.remove(&actor_id);
                        actor_states.remove(&actor_id);
                    }
                }
            }
        }

        // Process updated_actors
        if let Some(updated) = frame["updated_actors"].as_array() {
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
                } else if object_id == team_oid {
                    if let Some(team_actor_id) = attr["ActiveActor"]["actor"].as_u64() {
                        if let Some(&team_idx) = team_actor_to_index.get(&team_actor_id) {
                            player_to_team.insert(actor_id, team_idx);
                        }
                    }
                    if let Some(team_idx) = attr["FlaggedInt"]["int"].as_u64() {
                        player_to_team.insert(actor_id, team_idx as usize);
                    }
                } else if object_id == rb_oid {
                    if let Some(state) = extract_rb_state_3d(attr) {
                        if !seen_nonzero_z && frame_idx < z_check_frames && state.pos.z.abs() > 0.01
                        {
                            seen_nonzero_z = true;
                        }
                        actor_states.insert(actor_id, state);
                    }
                } else if object_id == steer_oid {
                    if let Some(byte_val) = attr["Byte"].as_u64() {
                        car_steer.insert(actor_id, byte_val as u8);
                    }
                }
            }
        }

        // Z-axis check: bail early if no z data after check window
        if frame_idx == z_check_frames && !seen_nonzero_z {
            return Ok(Vec::new());
        }

        // Per-frame dribble analysis
        let ball_state = match ball_actor_id.and_then(|id| actor_states.get(&id)) {
            Some(s) => s.clone(),
            None => continue,
        };

        // Track opponent challenges per team (for flick timing)
        // For each car, check if it's approaching the ball
        for &car_id in &car_actor_ids {
            let Some(&player_id) = car_to_player.get(&car_id) else {
                continue;
            };
            let Some(&team_idx) = player_to_team.get(&player_id) else {
                continue;
            };
            let Some(car_state) = actor_states.get(&car_id) else {
                continue;
            };

            let dist = distance_2d(&car_state.pos, &ball_state.pos);
            if dist < OPPONENT_CHALLENGE_RADIUS {
                let approach_dot = dot_toward_2d(&car_state.vel, &car_state.pos, &ball_state.pos);
                if approach_dot > OPPONENT_APPROACH_DOT_MIN {
                    // This car is challenging the ball — record for the opposing team's use
                    let opposing_team = 1 - team_idx;
                    opponent_challenging
                        .entry(opposing_team)
                        .or_default()
                        .push((car_id, frame_idx));
                }
            }
        }

        // Check dribble state for each car
        for &car_id in &car_actor_ids {
            let Some(car_state) = actor_states.get(&car_id) else {
                continue;
            };

            let xy_dist = distance_2d(&car_state.pos, &ball_state.pos);
            let z_diff = ball_state.pos.z - car_state.pos.z;
            let is_dribbling =
                xy_dist < DRIBBLE_XY_MAX && z_diff > DRIBBLE_Z_MIN && z_diff < DRIBBLE_Z_MAX;

            let steer = car_steer.get(&car_id).copied().unwrap_or(STEER_NEUTRAL);

            let window = dribble_windows.entry(car_id).or_insert(DribbleWindow {
                start_frame: frame_idx,
                frames: 0,
                gap_frames: 0,
                steer_history: Vec::new(),
                active: false,
            });

            if is_dribbling {
                if !window.active {
                    // Start a new dribble window
                    window.start_frame = frame_idx;
                    window.frames = 0;
                    window.gap_frames = 0;
                    window.steer_history.clear();
                    window.active = true;
                } else {
                    // Resume after gap
                    window.gap_frames = 0;
                }
                window.frames += 1;
                window.steer_history.push(steer);
            } else if window.active {
                window.gap_frames += 1;
                if window.gap_frames > DRIBBLE_GAP_TOLERANCE {
                    // Dribble ended — check for flick at the end
                    let flick_info = detect_flick(car_state, &ball_state);
                    finalize_dribble_window(
                        car_id,
                        frame_idx,
                        &car_to_player,
                        &mut dribble_windows,
                        &mut player_dribble_events,
                        flick_info,
                        &opponent_challenging,
                        &player_to_team,
                    );
                }
            }
        }

        // Prune old opponent challenge records (older than FLICK_TIMING_WINDOW frames)
        for entries in opponent_challenging.values_mut() {
            entries.retain(|&(_, f)| frame_idx.saturating_sub(f) <= FLICK_TIMING_WINDOW * 2);
        }
    }

    // Finalize any remaining active dribble windows
    let remaining_cars: Vec<u64> = dribble_windows
        .keys()
        .copied()
        .filter(|id| dribble_windows.get(id).map_or(false, |w| w.active))
        .collect();
    let last_frame = frames.len();
    for car_id in remaining_cars {
        finalize_dribble_window(
            car_id,
            last_frame,
            &car_to_player,
            &mut dribble_windows,
            &mut player_dribble_events,
            None,
            &opponent_challenging,
            &player_to_team,
        );
    }

    // Aggregate per-player results
    let mut results: Vec<PlayerDribbleResult> = Vec::new();

    for (&player_id, events) in &player_dribble_events {
        let name = player_names
            .get(&player_id)
            .cloned()
            .unwrap_or_else(|| format!("Actor_{}", player_id));

        let dribble_count = events.len();
        let total_dribble_frames: usize = events.iter().map(|e| e.duration_frames).sum();
        let total_flick_count = events.iter().filter(|e| e.flick_detected).count();
        let timed_flick_count = events.iter().filter(|e| e.flick_opponent_timed).count();

        // Average snappy correction rate (per second of dribble)
        let avg_snappy_rate = if dribble_count > 0 {
            let rates: Vec<f64> = events
                .iter()
                .filter(|e| e.duration_frames > 0)
                .map(|e| {
                    let seconds = e.duration_frames as f64 / APPROX_FPS;
                    if seconds > 0.0 {
                        e.snappy_corrections as f64 / seconds
                    } else {
                        0.0
                    }
                })
                .collect();
            if rates.is_empty() {
                0.0
            } else {
                rates.iter().sum::<f64>() / rates.len() as f64
            }
        } else {
            0.0
        };

        // Average zero-steer percentage across dribbles
        let avg_zero_steer_pct = if dribble_count > 0 {
            let pcts: Vec<f64> = events
                .iter()
                .filter(|e| e.duration_frames > 0)
                .map(|e| e.zero_steer_frames as f64 / e.duration_frames as f64 * 100.0)
                .collect();
            if pcts.is_empty() {
                0.0
            } else {
                pcts.iter().sum::<f64>() / pcts.len() as f64
            }
        } else {
            0.0
        };

        let snappy_score = score_snappy_rate(avg_snappy_rate);
        let zero_steer_score = score_zero_steer_pct(avg_zero_steer_pct);
        let flick_score = score_flick_timing(timed_flick_count, total_flick_count);

        let dribble_suspicion_score = if dribble_count >= MIN_DRIBBLES_FOR_SCORE {
            snappy_score * 0.4 + zero_steer_score * 0.35 + flick_score * 0.25
        } else {
            0.0
        };

        results.push(PlayerDribbleResult {
            name,
            dribble_count,
            total_dribble_frames,
            avg_snappy_rate,
            avg_zero_steer_pct,
            timed_flick_count,
            total_flick_count,
            snappy_score,
            zero_steer_score,
            flick_score,
            dribble_suspicion_score,
        });
    }

    results.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(results)
}

/// Check if the current ball state indicates a flick (ball moving upward and separating fast).
fn detect_flick(car_state: &ActorState, ball_state: &ActorState) -> Option<bool> {
    if ball_state.vel.z > FLICK_BALL_VZ_MIN {
        let separation_speed = distance_3d(&ball_state.vel, &car_state.vel);
        if separation_speed > FLICK_SEPARATION_SPEED {
            return Some(true);
        }
    }
    None
}

/// Finalize a dribble window: if it qualifies, analyze it and record the event.
fn finalize_dribble_window(
    car_id: u64,
    frame_idx: usize,
    car_to_player: &HashMap<u64, u64>,
    dribble_windows: &mut HashMap<u64, DribbleWindow>,
    player_dribble_events: &mut HashMap<u64, Vec<DribbleEvent>>,
    flick_info: Option<bool>, // Some(true) if flick detected at dribble end
    opponent_challenging: &HashMap<usize, Vec<(u64, usize)>>,
    player_to_team: &HashMap<u64, usize>,
) {
    let Some(window) = dribble_windows.get_mut(&car_id) else {
        return;
    };
    if !window.active {
        return;
    }

    window.active = false;

    if window.frames < DRIBBLE_MIN_FRAMES {
        return; // too short to qualify
    }

    let Some(&player_id) = car_to_player.get(&car_id) else {
        return;
    };

    let snappy_corrections = count_snappy_corrections(&window.steer_history);
    let zero_steer_frames = count_sustained_neutral_frames(&window.steer_history);

    let flick_detected = flick_info == Some(true);

    // Check if flick was timed to opponent challenge
    let flick_opponent_timed = if flick_detected {
        let team_idx = player_to_team.get(&player_id).copied();
        if let Some(team) = team_idx {
            // Check if any opponent was challenging within the timing window
            opponent_challenging.get(&team).map_or(false, |entries| {
                entries.iter().any(|&(_, f)| {
                    frame_idx.saturating_sub(FLICK_TIMING_WINDOW) <= f && f <= frame_idx
                })
            })
        } else {
            false
        }
    } else {
        false
    };

    player_dribble_events
        .entry(player_id)
        .or_default()
        .push(DribbleEvent {
            _start_frame: window.start_frame,
            duration_frames: window.frames,
            snappy_corrections,
            zero_steer_frames,
            flick_detected,
            flick_opponent_timed,
        });
}

pub fn print_report(results: &[PlayerDribbleResult]) {
    println!("=== Dribble Analysis ===");

    if results.is_empty() || results.iter().all(|r| r.dribble_count == 0) {
        println!("  No qualifying dribbles detected (60+ frame ball carry required).");
        return;
    }

    println!(
        "  {:<20} {:>8} {:>10} {:>10} {:>10} {:>8} {:>8} {:>10}",
        "Player", "Dribbles", "Frames", "SnappyR/s", "Neutral%", "Flicks", "Timed", "Suspicion"
    );
    println!("  {}", "-".repeat(94));

    for r in results {
        if r.dribble_count == 0 {
            println!(
                "  {:<20} {:>8} {:>10} {:>10} {:>10} {:>8} {:>8} {:>10}",
                r.name, 0, "-", "-", "-", "-", "-", "-"
            );
            continue;
        }

        let suspicion_str = if r.dribble_count >= MIN_DRIBBLES_FOR_SCORE {
            format!("{:.2}", r.dribble_suspicion_score)
        } else {
            "N/A (<2)".to_string()
        };

        println!(
            "  {:<20} {:>8} {:>10} {:>9.1}/s {:>9.1}% {:>8} {:>8} {:>10}",
            r.name,
            r.dribble_count,
            r.total_dribble_frames,
            r.avg_snappy_rate,
            r.avg_zero_steer_pct,
            r.total_flick_count,
            r.timed_flick_count,
            suspicion_str,
        );
    }

    println!("\n  Score Breakdown:");
    for r in results.iter().filter(|r| r.dribble_count >= MIN_DRIBBLES_FOR_SCORE) {
        println!(
            "    {}: snappy={:.2} zero_steer={:.2} flick_timing={:.2} -> composite={:.2}",
            r.name, r.snappy_score, r.zero_steer_score, r.flick_score, r.dribble_suspicion_score,
        );
    }
}
