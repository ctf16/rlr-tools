use serde_json::{json, Value};
use std::collections::HashMap;
use std::error;

// RigidBody coordinate units (~1/100 of Unreal units)
const FIELD_HALF_LENGTH: f64 = 51.2; // Y-axis, goal lines
// Metric thresholds
const DOUBLE_COMMIT_RADIUS: f64 = 15.0;
const DOUBLE_COMMIT_COOLDOWN: usize = 120; // ~1s between events for same pair
const BALL_CHASE_RADIUS: f64 = 20.0;
const DEFENSIVE_ZONE_DEPTH: f64 = 17.0; // distance from own goal = "defensive zone"
const MOMENTUM_MIN_FRAMES: usize = 30; // ~0.25s sustained upfield movement
const FAR_POST_X_MIN: f64 = 5.0;

fn resolve_object_id(objects: &[Value], needle: &str) -> Option<u64> {
    objects
        .iter()
        .position(|o| o.as_str().map_or(false, |s| s == needle))
        .map(|i| i as u64)
}

#[derive(Clone, Default)]
struct Vec3 {
    x: f64,
    y: f64,
    // z not needed for 2D rotation analysis
}

#[derive(Clone, Default)]
struct ActorState {
    pos: Vec3,
    vel: Vec3,
}

struct DoubleCommitEvent {
    frame: usize,
    time: f64,
    players: [String; 2],
    distance_from_ball: f64,
}

struct PlayerRotationStats {
    name: String,
    team: usize,
    frames_chasing: usize,
    frames_near_ball: usize,
    frames_offensive: usize,
    frames_defensive: usize,
    frames_active: usize,
    offensive_momentum_count: usize,
    consecutive_upfield_frames: usize,
    far_post_retreats: usize,
    total_retreats: usize,
    // Per-minute buckets
    per_minute_chase_frames: HashMap<usize, usize>,
    per_minute_offensive_frames: HashMap<usize, usize>,
    per_minute_total_frames: HashMap<usize, usize>,
}

struct TeamRotationStats {
    pairwise_distance_sum: f64,
    pairwise_distance_count: usize,
    double_commit_events: Vec<DoubleCommitEvent>,
    // Per-pair cooldown tracking: (player_a, player_b) -> last event frame
    pair_last_commit: HashMap<(String, String), usize>,
    // Per-minute avg distance buckets
    per_minute_distance_sum: HashMap<usize, f64>,
    per_minute_distance_count: HashMap<usize, usize>,
}

fn distance_2d(a: &Vec3, b: &Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn dot_toward(player: &ActorState, target: &Vec3) -> f64 {
    let dx = target.x - player.pos.x;
    let dy = target.y - player.pos.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.001 {
        return 0.0;
    }
    (player.vel.x * dx / len) + (player.vel.y * dy / len)
}

fn extract_rb_state(attr: &Value) -> Option<ActorState> {
    let rb = &attr["RigidBody"];
    let loc = &rb["location"];
    let lv = &rb["linear_velocity"];
    // boxcars outputs location as {"x":..., "y":..., "z":...} or as a flat array
    let (px, py) = if let (Some(x), Some(y)) = (loc["x"].as_f64(), loc["y"].as_f64()) {
        (x, y)
    } else if let Some(arr) = loc.as_array() {
        (arr.first()?.as_f64()?, arr.get(1)?.as_f64()?)
    } else {
        return None;
    };
    let (vx, vy) = if let (Some(x), Some(y)) = (lv["x"].as_f64(), lv["y"].as_f64()) {
        (x, y)
    } else if let Some(arr) = lv.as_array() {
        (
            arr.first().and_then(|v| v.as_f64()).unwrap_or(0.0),
            arr.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0),
        )
    } else {
        (0.0, 0.0)
    };
    Some(ActorState {
        pos: Vec3 { x: px, y: py },
        vel: Vec3 { x: vx, y: vy },
    })
}

fn make_pair_key(a: &str, b: &str) -> (String, String) {
    if a < b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

pub fn analyze(parsed_json: &Value) -> Result<Value, Box<dyn error::Error>> {
    let team_size = parsed_json["properties"]["TeamSize"]
        .as_i64()
        .unwrap_or(0);
    if team_size < 2 {
        return Err("Rotation analysis requires 2v2 or 3v3 (skipping 1v1)".into());
    }
    if team_size > 3 {
        return Err("Rotation analysis not supported for 4v4".into());
    }

    let objects = parsed_json["objects"]
        .as_array()
        .ok_or("missing objects array")?;

    let rb_oid = resolve_object_id(objects, "TAGame.RBActor_TA:ReplicatedRBState")
        .ok_or("ReplicatedRBState not found")?;
    let pri_link_oid = resolve_object_id(objects, "Engine.Pawn:PlayerReplicationInfo")
        .ok_or("PlayerReplicationInfo not found")?;
    let name_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:PlayerName")
        .ok_or("PlayerName not found")?;
    let team_oid = resolve_object_id(objects, "Engine.PlayerReplicationInfo:Team")
        .ok_or("Team not found")?;

    // Find archetype-based object IDs for ball and car detection in new_actors
    let ball_archetype = "Archetypes.Ball.Ball_Default";
    let car_archetype = "Archetypes.Car.Car_Default";
    let team0_archetype = "Archetypes.Teams.Team0";
    let team1_archetype = "Archetypes.Teams.Team1";

    let ball_arch_oid = resolve_object_id(objects, ball_archetype);
    let car_arch_oid = resolve_object_id(objects, car_archetype);
    let team0_arch_oid = resolve_object_id(objects, team0_archetype);
    let team1_arch_oid = resolve_object_id(objects, team1_archetype);

    let frames = parsed_json["network_frames"]["frames"]
        .as_array()
        .ok_or("missing network_frames.frames")?;

    // Linkage maps
    let mut player_names: HashMap<u64, String> = HashMap::new();
    let mut car_to_player: HashMap<u64, u64> = HashMap::new();
    let mut player_to_team: HashMap<u64, usize> = HashMap::new();
    let mut team_actor_to_index: HashMap<u64, usize> = HashMap::new();

    // Actor tracking
    let mut ball_actor_id: Option<u64> = None;
    let mut car_actor_ids: HashMap<u64, ()> = HashMap::new(); // set of known car actors

    // Last-known positions
    let mut actor_states: HashMap<u64, ActorState> = HashMap::new();

    // Per-player stats
    let mut player_stats: HashMap<u64, PlayerRotationStats> = HashMap::new();
    // Per-team stats (index 0, 1)
    let mut team_stats: [TeamRotationStats; 2] = [
        TeamRotationStats {
            pairwise_distance_sum: 0.0,
            pairwise_distance_count: 0,
            double_commit_events: Vec::new(),
            pair_last_commit: HashMap::new(),
            per_minute_distance_sum: HashMap::new(),
            per_minute_distance_count: HashMap::new(),
        },
        TeamRotationStats {
            pairwise_distance_sum: 0.0,
            pairwise_distance_count: 0,
            double_commit_events: Vec::new(),
            pair_last_commit: HashMap::new(),
            per_minute_distance_sum: HashMap::new(),
            per_minute_distance_count: HashMap::new(),
        },
    ];

    for (frame_idx, frame) in frames.iter().enumerate() {
        let frame_time = frame["time"].as_f64().unwrap_or(0.0);
        let minute_bucket = (frame_time / 60.0).floor() as usize;

        // Process new_actors for ball/car/team detection
        if let Some(new_actors) = frame["new_actors"].as_array() {
            for new_actor in new_actors {
                let actor_id = new_actor["actor_id"].as_u64().unwrap_or(u64::MAX);
                let object_id = new_actor["object_id"].as_u64().unwrap_or(u64::MAX);

                if Some(object_id) == ball_arch_oid {
                    ball_actor_id = Some(actor_id);
                } else if Some(object_id) == car_arch_oid {
                    car_actor_ids.insert(actor_id, ());
                } else if Some(object_id) == team0_arch_oid {
                    team_actor_to_index.insert(actor_id, 0);
                } else if Some(object_id) == team1_arch_oid {
                    team_actor_to_index.insert(actor_id, 1);
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
                    // Team update: attr is {"ActiveActor": {"actor": team_actor_id, ...}}
                    // or {"FlaggedInt": {"flag": bool, "int": team_index}}
                    if let Some(team_actor_id) = attr["ActiveActor"]["actor"].as_u64() {
                        if let Some(&team_idx) = team_actor_to_index.get(&team_actor_id) {
                            player_to_team.insert(actor_id, team_idx);
                        }
                    }
                    if let Some(team_idx) = attr["FlaggedInt"]["int"].as_u64() {
                        player_to_team.insert(actor_id, team_idx as usize);
                    }
                } else if object_id == rb_oid {
                    if let Some(state) = extract_rb_state(attr) {
                        actor_states.insert(actor_id, state);
                    }
                }
            }
        }

        // Per-frame analysis using carried-forward positions
        let ball_state = match ball_actor_id.and_then(|id| actor_states.get(&id)) {
            Some(s) => s.clone(),
            None => continue, // no ball position yet
        };

        // Build team rosters for this frame: team_index -> [(player_id, car_actor_id, ActorState)]
        let mut team_players: [Vec<(u64, ActorState)>; 2] = [Vec::new(), Vec::new()];

        for (&car_id, _) in &car_actor_ids {
            let Some(&player_id) = car_to_player.get(&car_id) else {
                continue;
            };
            let Some(&team_idx) = player_to_team.get(&player_id) else {
                continue;
            };
            if team_idx > 1 {
                continue;
            }
            let Some(car_state) = actor_states.get(&car_id) else {
                continue;
            };

            // Initialize player stats if needed
            let stats = player_stats.entry(player_id).or_insert_with(|| {
                let name = player_names
                    .get(&player_id)
                    .cloned()
                    .unwrap_or_else(|| format!("Actor_{}", player_id));
                PlayerRotationStats {
                    name,
                    team: team_idx,
                    frames_chasing: 0,
                    frames_near_ball: 0,
                    frames_offensive: 0,
                    frames_defensive: 0,
                    frames_active: 0,
                    offensive_momentum_count: 0,
                    consecutive_upfield_frames: 0,
                    far_post_retreats: 0,
                    total_retreats: 0,
                    per_minute_chase_frames: HashMap::new(),
                    per_minute_offensive_frames: HashMap::new(),
                    per_minute_total_frames: HashMap::new(),
                }
            });
            stats.team = team_idx; // update in case it changed

            stats.frames_active += 1;
            *stats.per_minute_total_frames.entry(minute_bucket).or_insert(0) += 1;

            let dist_to_ball = distance_2d(&car_state.pos, &ball_state.pos);

            // Near ball tracking
            if dist_to_ball < BALL_CHASE_RADIUS {
                stats.frames_near_ball += 1;
            }

            // Offensive/defensive half
            // Team 0 defends negative Y, attacks positive Y
            // Team 1 defends positive Y, attacks negative Y
            let is_offensive = match team_idx {
                0 => car_state.pos.y > 0.0,
                _ => car_state.pos.y < 0.0,
            };
            if is_offensive {
                stats.frames_offensive += 1;
                *stats.per_minute_offensive_frames.entry(minute_bucket).or_insert(0) += 1;
            } else {
                stats.frames_defensive += 1;
            }

            // Offensive momentum: sustained upfield movement
            let moving_upfield = match team_idx {
                0 => car_state.vel.y > 0.0,
                _ => car_state.vel.y < 0.0,
            };
            if moving_upfield && is_offensive {
                stats.consecutive_upfield_frames += 1;
                if stats.consecutive_upfield_frames == MOMENTUM_MIN_FRAMES {
                    stats.offensive_momentum_count += 1;
                }
            } else {
                stats.consecutive_upfield_frames = 0;
            }

            // Back-post rotation check
            let own_goal_y = match team_idx {
                0 => -FIELD_HALF_LENGTH,
                _ => FIELD_HALF_LENGTH,
            };
            let dist_from_own_goal = (car_state.pos.y - own_goal_y).abs();
            let retreating_toward_own_goal = match team_idx {
                0 => car_state.vel.y < 0.0, // moving toward negative Y
                _ => car_state.vel.y > 0.0,  // moving toward positive Y
            };

            if dist_from_own_goal < DEFENSIVE_ZONE_DEPTH && retreating_toward_own_goal {
                stats.total_retreats += 1;
                // Far post = player on opposite side from ball
                let at_far_post = (car_state.pos.x.signum() != ball_state.pos.x.signum())
                    && car_state.pos.x.abs() >= FAR_POST_X_MIN;
                if at_far_post {
                    stats.far_post_retreats += 1;
                }
            }

            team_players[team_idx].push((player_id, car_state.clone()));
        }

        // Team-level metrics
        for team_idx in 0..2 {
            let players = &team_players[team_idx];
            if players.len() < 2 {
                continue;
            }

            // Average pairwise distance
            for i in 0..players.len() {
                for j in (i + 1)..players.len() {
                    let dist = distance_2d(&players[i].1.pos, &players[j].1.pos);
                    team_stats[team_idx].pairwise_distance_sum += dist;
                    team_stats[team_idx].pairwise_distance_count += 1;
                    *team_stats[team_idx]
                        .per_minute_distance_sum
                        .entry(minute_bucket)
                        .or_insert(0.0) += dist;
                    *team_stats[team_idx]
                        .per_minute_distance_count
                        .entry(minute_bucket)
                        .or_insert(0) += 1;
                }
            }

            // Double commit detection
            for i in 0..players.len() {
                for j in (i + 1)..players.len() {
                    let (pid_a, state_a) = &players[i];
                    let (pid_b, state_b) = &players[j];

                    let dist_a = distance_2d(&state_a.pos, &ball_state.pos);
                    let dist_b = distance_2d(&state_b.pos, &ball_state.pos);

                    if dist_a < DOUBLE_COMMIT_RADIUS && dist_b < DOUBLE_COMMIT_RADIUS {
                        // Both approaching ball
                        let dot_a = dot_toward(state_a, &ball_state.pos);
                        let dot_b = dot_toward(state_b, &ball_state.pos);

                        if dot_a > 0.0 && dot_b > 0.0 {
                            let name_a = player_names
                                .get(pid_a)
                                .cloned()
                                .unwrap_or_else(|| format!("Actor_{}", pid_a));
                            let name_b = player_names
                                .get(pid_b)
                                .cloned()
                                .unwrap_or_else(|| format!("Actor_{}", pid_b));
                            let pair_key = make_pair_key(&name_a, &name_b);

                            let last_frame = team_stats[team_idx]
                                .pair_last_commit
                                .get(&pair_key)
                                .copied()
                                .unwrap_or(0);

                            if frame_idx.saturating_sub(last_frame) >= DOUBLE_COMMIT_COOLDOWN {
                                team_stats[team_idx]
                                    .pair_last_commit
                                    .insert(pair_key, frame_idx);
                                team_stats[team_idx]
                                    .double_commit_events
                                    .push(DoubleCommitEvent {
                                        frame: frame_idx,
                                        time: frame_time,
                                        players: [name_a, name_b],
                                        distance_from_ball: (dist_a + dist_b) / 2.0,
                                    });
                            }
                        }
                    }
                }
            }

            // Ball-chasing detection (per-team)
            let mut players_with_dist: Vec<(u64, f64, &ActorState)> = players
                .iter()
                .map(|(pid, state)| (*pid, distance_2d(&state.pos, &ball_state.pos), state))
                .collect();
            players_with_dist.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

            // Skip first man (closest to ball); others within BALL_CHASE_RADIUS who approach = chasing
            for &(pid, dist, state) in players_with_dist.iter().skip(1) {
                if dist < BALL_CHASE_RADIUS && dot_toward(state, &ball_state.pos) > 0.0 {
                    if let Some(stats) = player_stats.get_mut(&pid) {
                        stats.frames_chasing += 1;
                        *stats
                            .per_minute_chase_frames
                            .entry(minute_bucket)
                            .or_insert(0) += 1;
                    }
                }
            }
        }
    }

    // Assemble results
    let team_labels = ["Blue", "Orange"];
    let mut teams_json = Vec::new();

    for team_idx in 0..2 {
        let ts = &team_stats[team_idx];
        let avg_teammate_dist = if ts.pairwise_distance_count > 0 {
            ts.pairwise_distance_sum / ts.pairwise_distance_count as f64
        } else {
            0.0
        };

        let double_commit_events: Vec<Value> = ts
            .double_commit_events
            .iter()
            .map(|e| {
                json!({
                    "frame": e.frame,
                    "time": e.time,
                    "players": e.players,
                    "distance_from_ball": (e.distance_from_ball * 10.0).round() / 10.0,
                })
            })
            .collect();

        let mut players_json = Vec::new();
        for (_, stats) in player_stats.iter().filter(|(_, s)| s.team == team_idx) {
            let active = stats.frames_active.max(1) as f64;
            players_json.push(json!({
                "name": stats.name,
                "ball_chase_pct": (stats.frames_chasing as f64 / active * 1000.0).round() / 10.0,
                "time_near_ball_pct": (stats.frames_near_ball as f64 / active * 1000.0).round() / 10.0,
                "offensive_pct": (stats.frames_offensive as f64 / active * 1000.0).round() / 10.0,
                "defensive_pct": (stats.frames_defensive as f64 / active * 1000.0).round() / 10.0,
                "offensive_momentum_count": stats.offensive_momentum_count,
                "back_post_rotation_pct": if stats.total_retreats > 0 {
                    (stats.far_post_retreats as f64 / stats.total_retreats as f64 * 1000.0).round() / 10.0
                } else {
                    0.0
                },
                "defensive_retreats": stats.total_retreats,
            }));
        }
        players_json.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });

        // Per-minute breakdown
        let mut all_minutes: Vec<usize> = ts
            .per_minute_distance_count
            .keys()
            .copied()
            .collect();
        // Also include minutes from player stats
        for (_, stats) in player_stats.iter().filter(|(_, s)| s.team == team_idx) {
            for &m in stats.per_minute_total_frames.keys() {
                if !all_minutes.contains(&m) {
                    all_minutes.push(m);
                }
            }
        }
        all_minutes.sort();
        all_minutes.dedup();

        let mut per_minute_json = Vec::new();
        for &minute in &all_minutes {
            let avg_dist = ts
                .per_minute_distance_count
                .get(&minute)
                .and_then(|&count| {
                    if count > 0 {
                        Some(ts.per_minute_distance_sum.get(&minute).unwrap_or(&0.0) / count as f64)
                    } else {
                        None
                    }
                })
                .unwrap_or(0.0);

            let mut total_frames = 0usize;
            let mut offensive_frames = 0usize;
            let mut chase_frames = 0usize;
            for (_, stats) in player_stats.iter().filter(|(_, s)| s.team == team_idx) {
                total_frames += stats.per_minute_total_frames.get(&minute).unwrap_or(&0);
                offensive_frames += stats.per_minute_offensive_frames.get(&minute).unwrap_or(&0);
                chase_frames += stats.per_minute_chase_frames.get(&minute).unwrap_or(&0);
            }

            per_minute_json.push(json!({
                "minute": minute,
                "avg_teammate_distance": (avg_dist * 10.0).round() / 10.0,
                "offensive_pct": if total_frames > 0 {
                    (offensive_frames as f64 / total_frames as f64 * 1000.0).round() / 10.0
                } else { 0.0 },
                "ball_chase_frames": chase_frames,
            }));
        }

        teams_json.push(json!({
            "team_index": team_idx,
            "team_name": team_labels[team_idx],
            "avg_teammate_distance": (avg_teammate_dist * 10.0).round() / 10.0,
            "double_commits": {
                "count": ts.double_commit_events.len(),
                "events": double_commit_events,
            },
            "players": players_json,
            "per_minute": per_minute_json,
        }));
    }

    Ok(json!({ "teams": teams_json }))
}

pub fn print_report(result: &Value) {
    println!("=== Rotation Analysis ===\n");

    let teams = result["teams"].as_array().unwrap();

    // Section 1: Summary table
    println!(
        "  {:<24} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8} {:>10}",
        "Player", "Team", "Chase%", "Offens%", "Defens%", "Moment.", "BkPost%", "AvgTeamD"
    );
    println!("  {}", "-".repeat(90));

    for team in teams {
        let team_name = team["team_name"].as_str().unwrap_or("?");
        let avg_dist = team["avg_teammate_distance"].as_f64().unwrap_or(0.0);

        if let Some(players) = team["players"].as_array() {
            for player in players {
                let name = player["name"].as_str().unwrap_or("?");
                let chase = player["ball_chase_pct"].as_f64().unwrap_or(0.0);
                let offense = player["offensive_pct"].as_f64().unwrap_or(0.0);
                let defense = player["defensive_pct"].as_f64().unwrap_or(0.0);
                let momentum = player["offensive_momentum_count"].as_u64().unwrap_or(0);
                let back_post = player["back_post_rotation_pct"].as_f64().unwrap_or(0.0);

                println!(
                    "  {:<24} {:>6} {:>7.1}% {:>7.1}% {:>7.1}% {:>8} {:>7.1}% {:>10.1}",
                    name, team_name, chase, offense, defense, momentum, back_post, avg_dist
                );
            }
        }
    }

    // Section 2: Double-commit event list
    println!("\n  Double Commits:");
    let mut any_events = false;
    for team in teams {
        let team_name = team["team_name"].as_str().unwrap_or("?");
        if let Some(events) = team["double_commits"]["events"].as_array() {
            for event in events {
                any_events = true;
                let time = event["time"].as_f64().unwrap_or(0.0);
                let minutes = (time / 60.0).floor() as u32;
                let seconds = (time % 60.0).floor() as u32;
                let players = event["players"].as_array().unwrap();
                let dist = event["distance_from_ball"].as_f64().unwrap_or(0.0);
                println!(
                    "    [{:>2}:{:02}] {} + {} (team {}) @ {:.1} RB units from ball",
                    minutes,
                    seconds,
                    players[0].as_str().unwrap_or("?"),
                    players[1].as_str().unwrap_or("?"),
                    team_name,
                    dist
                );
            }
        }
    }
    if !any_events {
        println!("    (none detected)");
    }

    // Section 3: Per-minute breakdown
    println!("\n  Per-Minute Breakdown:");
    for team in teams {
        let team_name = team["team_name"].as_str().unwrap_or("?");
        if let Some(per_minute) = team["per_minute"].as_array() {
            if per_minute.is_empty() {
                continue;
            }
            println!(
                "\n    Team {}: {:>8} {:>10} {:>12}",
                team_name, "AvgDist", "Offense%", "ChaseFrames"
            );
            println!("    {}", "-".repeat(45));
            for entry in per_minute {
                let minute = entry["minute"].as_u64().unwrap_or(0);
                let avg_dist = entry["avg_teammate_distance"].as_f64().unwrap_or(0.0);
                let offense = entry["offensive_pct"].as_f64().unwrap_or(0.0);
                let chase = entry["ball_chase_frames"].as_u64().unwrap_or(0);
                println!(
                    "    Min {:>2}: {:>8.1} {:>9.1}% {:>12}",
                    minute, avg_dist, offense, chase
                );
            }
        }
    }
}
