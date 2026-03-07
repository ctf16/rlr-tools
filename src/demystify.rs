// Small module to help demystify the massive parsed JSON files
use serde_json::Value;
use std::fs;

pub fn load_parsed_json(cache_path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(cache_path)?;
    let json: Value = serde_json::from_str(&content)?;
    Ok(json)
}

// List a basic overview of the facts of this match
// Includes:
//  - Game type: {soccar, rumble, hoops, etc.}
//      - Grab from game_type field -> "TAGame.Replay_Soccar_TA"
//  - Team size: {1v1, 2v2, 3v3, 4v4}
//      - Grab from TeamSize field
//  - Kickoff time (convert epoch to datetime)
//  - Result: include ff/rage quit
//      - UnfairTeamSize and bForfeit fields in properties field of JSON
//      - If game not completed, include how long was played
//      - Grab result from properties.Team{0,1}Score fields
pub fn game_overview(parsed_json: &Value) {
    let game_type_raw = parsed_json["game_type"].as_str().unwrap_or("Unknown");
    let game_type = match game_type_raw {
        "TAGame.Replay_Soccar_TA" => "Soccar",
        "TAGame.Replay_Hoops_TA" => "Hoops",
        "TAGame.Replay_Rumble_TA" => "Rumble",
        "TAGame.Replay_Breakout_TA" => "Dropshot",
        "TAGame.Replay_Snowday_TA" => "Snow Day",
        _ => game_type_raw,
    };

    let props = &parsed_json["properties"];

    let team_size = props["TeamSize"].as_u64().unwrap_or(0);
    let team_size_label = format!("{}v{}", team_size, team_size);

    let date = props["Date"].as_str().unwrap_or("Unknown");

    let team0_score = props["Team0Score"].as_u64().unwrap_or(0);
    let team1_score = props["Team1Score"].as_u64().unwrap_or(0);

    let forfeit = props["bForfeit"].as_bool().unwrap_or(false);
    let unfair = props["UnfairTeamSize"].as_u64().unwrap_or(0);
    let total_seconds = props["TotalSecondsPlayed"].as_f64().unwrap_or(0.0);

    let minutes = (total_seconds / 60.0) as u64;
    let seconds = (total_seconds % 60.0) as u64;

    println!("=== Game Overview ===");
    println!("  Game Type:  {game_type}");
    println!("  Team Size:  {team_size_label}");
    println!("  Date:       {date}");
    println!("  Score:      Team 0: {team0_score}  -  Team 1: {team1_score}");

    if forfeit {
        let losing_team = if team0_score < team1_score { 0 } else { 1 };
        println!("  Result:     Forfeit (Team {losing_team} forfeited)");
    }

    if unfair != 0 {
        println!("  Note:       Unfair team size detected ({unfair})");
    }

    println!("  Duration:   {minutes}m {seconds}s played");
}

// List the players in the lobby
// Includes:
//  - Player name
//  - Player platform
pub fn list_players(parsed_json: &Value) {
    let props = &parsed_json["properties"];
    let players = match props["PlayerStats"].as_array() {
        Some(arr) => arr,
        None => {
            println!("No player stats found.");
            return;
        }
    };

    let (team0, team1): (Vec<_>, Vec<_>) = players.iter().partition(|p| {
        p["Team"].as_u64().unwrap_or(0) == 0
    });

    println!("=== Players ===");
    for (label, team) in [("Team 0 (Blue)", &team0), ("Team 1 (Orange)", &team1)] {
        println!("  --- {label} ---");
        for player in team {
            let name = player["Name"].as_str().unwrap_or("Unknown");
            let platform_raw = player["Platform"]["value"].as_str().unwrap_or("Unknown");
            let platform = platform_raw
                .strip_prefix("OnlinePlatform_")
                .unwrap_or(platform_raw);
            let is_bot = player["bBot"].as_bool().unwrap_or(false);

            let bot_tag = if is_bot { " [BOT]" } else { "" };
            println!("    {name} ({platform}){bot_tag}");
        }
    }
}

// List all player stats
// Includes:
//  - Score
//  - Goals
//  - Assists
//  - Saves
//  - Shots
pub fn player_stats(parsed_json: &Value) {
    let props = &parsed_json["properties"];
    let players = match props["PlayerStats"].as_array() {
        Some(arr) => arr,
        None => {
            println!("No player stats found.");
            return;
        }
    };

    let (team0, team1): (Vec<_>, Vec<_>) = players.iter().partition(|p| {
        p["Team"].as_u64().unwrap_or(0) == 0
    });

    println!("=== Player Stats ===");
    for (label, team) in [("Team 0 (Blue)", &team0), ("Team 1 (Orange)", &team1)] {
        println!("  --- {label} ---");
        println!(
            "  {:<20} {:>5} {:>5} {:>5} {:>5} {:>5}",
            "Name", "Score", "Goals", "Asst", "Saves", "Shots"
        );
        println!("  {}", "-".repeat(55));

        for player in team {
            let name = player["Name"].as_str().unwrap_or("Unknown");
            let score = player["Score"].as_u64().unwrap_or(0);
            let goals = player["Goals"].as_u64().unwrap_or(0);
            let assists = player["Assists"].as_u64().unwrap_or(0);
            let saves = player["Saves"].as_u64().unwrap_or(0);
            let shots = player["Shots"].as_u64().unwrap_or(0);

            println!(
                "  {:<20} {:>5} {:>5} {:>5} {:>5} {:>5}",
                name, score, goals, assists, saves, shots
            );
        }
    }
}
