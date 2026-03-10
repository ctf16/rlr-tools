# Contributing to rlr-tools

Thanks for your interest in contributing! This document describes how to get set up and what to keep in mind when submitting changes.

## Getting Started

1. Fork the repository and clone your fork.
2. Make sure you have Rust 1.85+ installed (see [README.md](README.md#prerequisites) for setup).
3. Run `cargo build` to verify everything compiles.
4. Place any test `.replay` files in a subdirectory under `assets/replays/` and run `cargo run` to confirm parsing works.

## What to Work On

Check [ROADMAP.md](ROADMAP.md) for the current feature roadmap. Unchecked items are open for contribution. If you want to work on something not listed, open an issue first to discuss the approach.

Bot detection improvements and new match analysis modules are the highest priority areas right now.

## Project Conventions

- **One module per feature** — Each analysis feature lives in its own `src/<name>.rs` file. Don't add analysis logic to `main.rs` or `parser.rs`.
- **Error handling** — Use `Box<dyn error::Error>` for public function return types. Don't `unwrap()` in library code; `unwrap()` is acceptable in `main.rs` for CLI-level errors.
- **Network data parsing** — Always enabled via `must_parse_network_data()`. If your feature reads network frames, follow the existing pattern of resolving object IDs from the `objects` array, then iterating `network_frames.frames[].updated_actors`.
- **Reporting** — Each module should expose an `analyze()` function that returns structured results, a `print_report()` function for CLI output, and `to_json()` / `results_to_json()` methods for programmatic use.
- **Keep it simple** — Don't over-abstract. Three similar lines of code is better than a premature helper function. Only add comments where the logic isn't self-evident.

## Submitting Changes

1. Create a feature branch from `main` (`git checkout -b feature/your-feature`).
2. Make your changes. Keep commits focused — one logical change per commit.
3. Make sure `cargo build` and `cargo clippy` pass without warnings.
4. Test your changes against real replay files. Include sample output in the PR description if it's a new analysis module.
5. Open a pull request against `main`. In the PR description:
   - Describe what the change does and why.
   - Reference the relevant ROADMAP item if applicable.
   - Include example output for new features.

## Adding a New Analysis Module

1. Create `src/your_module.rs`.
2. Add `mod your_module;` to `src/main.rs`.
3. Wire it into the interactive menu in `main.rs` (add a menu option and call your `analyze()` + `print_report()` functions).
4. Update [ROADMAP.md](ROADMAP.md) to check off the item.
5. Add a section to [README.md](README.md#features) describing the feature.

## Code Style

- Run `cargo fmt` before committing.
- Run `cargo clippy` and fix any warnings.
- No unnecessary dependencies — if the standard library or an existing dependency can do it, use that.

## Questions

Open an issue on the [GitHub repository](https://github.com/ctf16/rlr-tools/issues) for questions, bug reports, or feature discussions.
