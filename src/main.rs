mod boost_analysis;
mod bot_detection;
mod demystify;
mod kickoff_analysis;
mod merkle;
mod parser;

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const REPLAY_DIR: &str = "assets/replays";
const CACHE_DIR: &str = "parsed_games";
struct ReplayEntry {
    path: String,
    name: String,
    // cached: bool,
}

fn list_categories() -> Vec<String> {
    let Ok(dirs) = fs::read_dir(REPLAY_DIR) else {
        eprintln!("Could not read replay directory: {}", REPLAY_DIR);
        return Vec::new();
    };

    let mut categories: Vec<String> = dirs
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    categories.sort();
    categories
}

fn list_replays_in_category(category: &str) -> Vec<ReplayEntry> {
    let dir = format!("{}/{}", REPLAY_DIR, category);
    let Ok(read_dir) = fs::read_dir(&dir) else {
        eprintln!("Could not read directory: {}", dir);
        return Vec::new();
    };

    let mut names: Vec<String> = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map_or(false, |ext| ext == "replay")
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    let mut entries = Vec::new();
    for name in names {
        let path = format!("{}/{}/{}", REPLAY_DIR, category, name);
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let cache_path = format!("{}/{}.json", CACHE_DIR, stem);
        let cached = Path::new(&cache_path).exists();

        let index = entries.len() + 1;
        let marker = if cached { "[✓]" } else { "[ ]" };
        println!("  {index:>3}. {marker} {name}");

        entries.push(ReplayEntry {
            path,
            name,
            // cached,
        });
    }

    entries
}

fn prompt_input(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

fn wait_for_enter() {
    prompt_input("\nPress Enter to continue...");
}

fn with_loading<F, T>(message: &str, f: F) -> T
where
    F: FnOnce() -> T,
{
    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();
    let msg = message.to_string();

    let handle = thread::spawn(move || {
        let dots = [".", "..", "..."];
        let mut i = 0;
        while !done_clone.load(Ordering::Relaxed) {
            print!("\r  {}{:<3}", msg, dots[i % 3]);
            io::stdout().flush().unwrap();
            i += 1;
            thread::sleep(Duration::from_millis(300));
        }
        print!("\r  {}... done\n", msg);
        io::stdout().flush().unwrap();
    });

    let result = f();
    done.store(true, Ordering::Relaxed);
    handle.join().unwrap();
    result
}

fn main() {
    loop {
        println!("\n=== Rocket League Replay Tools ===");
        println!("Select a category:");

        let categories = list_categories();
        if categories.is_empty() {
            println!("  No categories found.");
            break;
        }

        for (i, cat) in categories.iter().enumerate() {
            println!("  {:>3}. {}", i + 1, cat);
        }
        println!("  [q] Quit\n");

        let cat_input = prompt_input("Category: ");

        if cat_input.eq_ignore_ascii_case("q") {
            break;
        }

        let Ok(cat_index) = cat_input.parse::<usize>() else {
            println!("Invalid input.");
            continue;
        };

        if cat_index < 1 || cat_index > categories.len() {
            println!("Number out of range.");
            continue;
        }

        let category = &categories[cat_index - 1];

        println!("\n=== Replays in [{}] ===", category);
        let entries = list_replays_in_category(category);

        if entries.is_empty() {
            println!("  No replays found.");
            wait_for_enter();
            continue;
        }

        println!("  [b] Back\n");

        let input = prompt_input("Select a replay number (or b to go back): ");

        if input.eq_ignore_ascii_case("b") {
            continue;
        }

        let Ok(index) = input.parse::<usize>() else {
            println!("Invalid input.");
            continue;
        };

        if index < 1 || index > entries.len() {
            println!("Number out of range.");
            continue;
        }

        let entry = &entries[index - 1];
        println!();

        let parse_result = with_loading("Parsing replay", || parser::run_cached(&entry.path));

        match parse_result {
            Ok(_) => {
                let stem = Path::new(&entry.name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown");
                let cache_path = format!("{}/{}.json", CACHE_DIR, stem);
                match demystify::load_parsed_json(&cache_path) {
                    Ok(json) => {
                        demystify::game_overview(&json);
                        println!();
                        demystify::list_players(&json);
                        println!();
                        demystify::player_stats(&json);

                        println!("\n  [s] Sign this replay (generate .sig sidecar)");
                        println!("  [v] Verify existing signature");
                        println!("  [b] Bot detection analysis");
                        println!("  [k] Kickoff analysis");
                        println!("  [o] Boost analysis");
                        println!("  [Enter] Continue\n");
                        let action = prompt_input("Action: ");

                        let sig_path = format!("{}/{}.sig", CACHE_DIR, stem);

                        if action.eq_ignore_ascii_case("s") {
                            let tree = merkle::MerkleTree::from_replay_json(&json);
                            println!("\nMerkle root: {}", hex::encode(tree.root));
                            for (i, leaf) in tree.leaves.iter().enumerate() {
                                let label = merkle::SECTION_LABELS.get(i).unwrap_or(&"Unknown");
                                println!("  Leaf {i} ({label}): {}", hex::encode(leaf));
                            }

                            let sidecar = merkle::SidecarFile::create(tree);
                            match sidecar.save(&sig_path) {
                                Ok(_) => println!("\nSidecar saved to {sig_path}"),
                                Err(e) => eprintln!("Failed to save sidecar: {e}"),
                            }
                        } else if action.eq_ignore_ascii_case("b") {
                            match bot_detection::analyze(&json) {
                                Ok(results) => {
                                    println!();
                                    bot_detection::print_report(&results);
                                }
                                Err(e) => eprintln!("Bot detection failed: {e}"),
                            }
                        } else if action.eq_ignore_ascii_case("k") {
                            match kickoff_analysis::analyze(&json) {
                                Ok(results) => {
                                    println!();
                                    kickoff_analysis::print_report(&results);
                                }
                                Err(e) => eprintln!("Kickoff analysis failed: {e}"),
                            }
                        } else if action.eq_ignore_ascii_case("o") {
                            match boost_analysis::analyze(&json) {
                                Ok(results) => {
                                    println!();
                                    boost_analysis::print_report(&results);
                                }
                                Err(e) => eprintln!("Boost analysis failed: {e}"),
                            }
                        } else if action.eq_ignore_ascii_case("v") {
                            match merkle::SidecarFile::load(&sig_path) {
                                Ok(sidecar) => {
                                    let sig_ok = sidecar.verify_signature();
                                    println!("\nSignature valid: {sig_ok}");

                                    match sidecar.merkle.verify_replay_json(&json) {
                                        merkle::VerifyResult::Valid => {
                                            println!("Replay integrity: VALID");
                                        }
                                        merkle::VerifyResult::Tampered { section_index } => {
                                            println!("Replay integrity: TAMPERED");
                                            if let Some(i) = section_index {
                                                let label = merkle::SECTION_LABELS
                                                    .get(i)
                                                    .unwrap_or(&"Unknown");
                                                println!("  Tampered section: {i} ({label})");
                                            }
                                        }
                                    }
                                }
                                Err(e) => eprintln!("No sidecar found at {sig_path}: {e}"),
                            }
                        }
                    }
                    Err(e) => eprintln!("Failed to load parsed JSON: {}", e),
                }
            }
            Err(e) => eprintln!("Failed to parse replay: {}", e),
        }

        wait_for_enter();
    }
}
