// Replay parsing
use boxcars::{ParseError, Replay};
use std::error;
use std::fs;
use std::path::Path;

pub fn parse_rl(data: &[u8]) -> Result<Replay, ParseError> {
    boxcars::ParserBuilder::new(data)
        .must_parse_network_data()
        .parse()
}

fn get_cache_path(replay_path: &str) -> String {
    let stem = Path::new(replay_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    format!("parsed_games/{}.json", stem)
}

pub fn run_cached(filename: &str) -> Result<(), Box<dyn error::Error>> {
    let cache_path = get_cache_path(filename);

    if Path::new(&cache_path).exists() {
        return Ok(());
    }

    let buffer = fs::read(filename)?;
    let replay = parse_rl(&buffer)?;

    fs::create_dir_all("parsed_games")?;
    let json = serde_json::to_string_pretty(&replay)?;
    fs::write(&cache_path, &json)?;
    Ok(())
}

// pub fn run(filename: &str) -> Result<(), Box<dyn error::Error>> {
//     let buffer = fs::read(filename)?;
//     let replay = parse_rl(&buffer)?;
//
//     fs::create_dir_all("parsed_games")?;
//     let cache_path = get_cache_path(filename);
//     let json = serde_json::to_string_pretty(&replay)?;
//     fs::write(&cache_path, &json)?;
//     Ok(())
// }

