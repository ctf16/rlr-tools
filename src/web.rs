use axum::{
    Router,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::fs;
use std::net::SocketAddr;
use tower_http::services::ServeDir;

use crate::{
    boost_analysis, bot_detection, demystify, dribble_analysis, kickoff_analysis, merkle, parser,
    rotation_analysis,
};

const REPLAY_DIR: &str = "assets/replays";
const CACHE_DIR: &str = "parsed_games";
const UPLOAD_DIR: &str = "assets/replays/uploads";
const MAX_UPLOAD_SIZE: usize = 20 * 1024 * 1024; // 20 MB

#[derive(Clone)]
struct AppState {
    replay_dir: String,
    cache_dir: String,
}

// Simple error type that converts to HTTP responses
struct ApiError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl ApiError {
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
        }
    }
    fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }
    fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
}

/// Ensure a replay is parsed and cached, then load the cached JSON.
async fn resolve_and_load(state: &AppState, category: &str, name: &str) -> Result<Value, ApiError> {
    let replay_path = format!("{}/{}/{}.replay", state.replay_dir, category, name);
    let cache_path = format!("{}/{}.json", state.cache_dir, name);

    if !std::path::Path::new(&replay_path).exists() {
        return Err(ApiError::not_found(format!(
            "Replay not found: {}/{}",
            category, name
        )));
    }

    let rp = replay_path.clone();
    tokio::task::spawn_blocking(move || parser::run_cached(&rp).map_err(|e| e.to_string()))
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
        .map_err(|e| ApiError::internal(format!("Parse error: {e}")))?;

    let cp = cache_path.clone();
    let json_val = tokio::task::spawn_blocking(move || {
        demystify::load_parsed_json(&cp).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
    .map_err(|e| ApiError::not_found(format!("Cache load error: {e}")))?;

    Ok(json_val)
}

pub fn start_server(port: u16) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        let state = AppState {
            replay_dir: REPLAY_DIR.to_string(),
            cache_dir: CACHE_DIR.to_string(),
        };

        let api = Router::new()
            .route("/replays", get(handle_list_replays))
            .route("/replays/upload", post(handle_upload))
            .route("/replays/{category}", get(handle_list_category))
            .route("/replays/{category}/{name}/overview", get(handle_overview))
            .route("/replays/{category}/{name}/players", get(handle_players))
            .route("/replays/{category}/{name}/stats", get(handle_stats))
            .route(
                "/replays/{category}/{name}/bot-detection",
                get(handle_bot_detection),
            )
            .route("/replays/{category}/{name}/kickoff", get(handle_kickoff))
            .route("/replays/{category}/{name}/boost", get(handle_boost))
            .route("/replays/{category}/{name}/rotation", get(handle_rotation))
            .route("/replays/{category}/{name}/dribble", get(handle_dribble))
            .route("/replays/{category}/{name}/sign", post(handle_sign))
            .route("/replays/{category}/{name}/verify", get(handle_verify));

        let app = Router::new()
            .nest("/api", api)
            .fallback_service(ServeDir::new("static"))
            .with_state(state);

        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        println!("rlr-tools web server listening on http://localhost:{port}");
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("Failed to bind address");
        axum::serve(listener, app).await.expect("Server error");
    });
}

// ── List endpoints ──────────────────────────────────────────────────────────

async fn handle_list_replays(State(_state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let categories = list_categories_json()?;
    Ok(Json(json!({ "categories": categories })))
}

async fn handle_list_category(
    State(_state): State<AppState>,
    Path(category): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let dir = format!("{}/{}", REPLAY_DIR, category);
    if !std::path::Path::new(&dir).is_dir() {
        return Err(ApiError::not_found(format!(
            "Category not found: {}",
            category
        )));
    }
    let replays = list_replays_in_category_json(&category)?;
    Ok(Json(json!({ "category": category, "replays": replays })))
}

fn list_categories_json() -> Result<Vec<Value>, ApiError> {
    let dirs =
        fs::read_dir(REPLAY_DIR).map_err(|e| ApiError::internal(format!("Read dir: {e}")))?;

    let mut categories: Vec<Value> = Vec::new();
    let mut names: Vec<String> = dirs
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    for name in names {
        let replays = list_replays_in_category_json(&name).unwrap_or_default();
        categories.push(json!({
            "name": name,
            "replay_count": replays.len(),
            "replays": replays,
        }));
    }
    Ok(categories)
}

fn list_replays_in_category_json(category: &str) -> Result<Vec<Value>, ApiError> {
    let dir = format!("{}/{}", REPLAY_DIR, category);
    let read_dir = fs::read_dir(&dir).map_err(|e| ApiError::internal(format!("Read dir: {e}")))?;

    let mut replays: Vec<Value> = Vec::new();
    let mut names: Vec<String> = read_dir
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "replay"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    for name in names {
        let stem = std::path::Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let cache_path = format!("{}/{}.json", CACHE_DIR, stem);
        let cached = std::path::Path::new(&cache_path).exists();
        replays.push(json!({
            "name": stem,
            "filename": name,
            "cached": cached,
        }));
    }
    Ok(replays)
}

// ── Upload endpoint ─────────────────────────────────────────────────────────

async fn handle_upload(mut multipart: Multipart) -> Result<Json<Value>, ApiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Multipart error: {e}")))?
    {
        let filename = field.file_name().map(|s| s.to_string()).unwrap_or_default();

        if !filename.ends_with(".replay") {
            return Err(ApiError::bad_request("File must have .replay extension"));
        }

        let data = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(format!("Read error: {e}")))?;

        if data.len() > MAX_UPLOAD_SIZE {
            return Err(ApiError::bad_request(format!(
                "File too large (max {} MB)",
                MAX_UPLOAD_SIZE / 1024 / 1024
            )));
        }

        let stem = std::path::Path::new(&filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        // Save .replay file to uploads directory
        fs::create_dir_all(UPLOAD_DIR)
            .map_err(|e| ApiError::internal(format!("Create upload dir: {e}")))?;
        let upload_path = format!("{}/{}", UPLOAD_DIR, filename);
        fs::write(&upload_path, &data)
            .map_err(|e| ApiError::internal(format!("Write file: {e}")))?;

        // Parse and cache
        let data_vec = data.to_vec();
        let stem_clone = stem.clone();
        tokio::task::spawn_blocking(move || {
            parser::parse_and_cache_bytes(&data_vec, &stem_clone).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
        .map_err(|e| ApiError::internal(format!("Parse error: {e}")))?;

        // Load overview
        let cache_path = format!("{}/{}.json", CACHE_DIR, stem);
        let json_val = tokio::task::spawn_blocking(move || {
            demystify::load_parsed_json(&cache_path).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
        .map_err(|e| ApiError::internal(format!("Cache load: {e}")))?;

        let overview = demystify::game_overview_json(&json_val);
        return Ok(Json(json!({
            "name": stem,
            "category": "uploads",
            "overview": overview,
        })));
    }

    Err(ApiError::bad_request("No file in upload"))
}

// ── Demystify endpoints ─────────────────────────────────────────────────────

async fn handle_overview(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    Ok(Json(demystify::game_overview_json(&json)))
}

async fn handle_players(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    Ok(Json(demystify::list_players_json(&json)))
}

async fn handle_stats(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    Ok(Json(demystify::player_stats_json(&json)))
}

// ── Analysis endpoints ──────────────────────────────────────────────────────

async fn handle_bot_detection(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let results = tokio::task::spawn_blocking(move || {
        bot_detection::analyze(&json).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
    .map_err(|e| ApiError::internal(format!("Analysis error: {e}")))?;
    Ok(Json(bot_detection::results_to_json(&results)))
}

async fn handle_kickoff(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let results = tokio::task::spawn_blocking(move || {
        kickoff_analysis::analyze(&json).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
    .map_err(|e| ApiError::internal(format!("Analysis error: {e}")))?;
    Ok(Json(kickoff_analysis::results_to_json(&results)))
}

async fn handle_boost(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let results = tokio::task::spawn_blocking(move || {
        boost_analysis::analyze(&json).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
    .map_err(|e| ApiError::internal(format!("Analysis error: {e}")))?;
    Ok(Json(boost_analysis::results_to_json(&results)))
}

async fn handle_rotation(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let results = tokio::task::spawn_blocking(move || {
        rotation_analysis::analyze(&json).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
    .map_err(|e| ApiError::internal(format!("Analysis error: {e}")))?;
    Ok(Json(results))
}

async fn handle_dribble(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let results = tokio::task::spawn_blocking(move || {
        dribble_analysis::analyze(&json).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {e}")))?
    .map_err(|e| ApiError::internal(format!("Analysis error: {e}")))?;
    Ok(Json(dribble_analysis::results_to_json(&results)))
}

// ── Signing/Verification endpoints ──────────────────────────────────────────

async fn handle_sign(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let sig_path = format!("{}/{}.sig", CACHE_DIR, name);

    let tree = merkle::MerkleTree::from_replay_json(&json);
    let root_hex = hex::encode(&tree.root);
    let leaves: Vec<Value> = tree
        .leaves
        .iter()
        .enumerate()
        .map(|(i, leaf)| {
            let label = merkle::SECTION_LABELS.get(i).unwrap_or(&"Unknown");
            json!({ "index": i, "label": label, "hash": hex::encode(leaf) })
        })
        .collect();

    let sidecar =
        merkle::SidecarFile::create(tree).map_err(|e| ApiError::internal(format!("{e}")))?;
    sidecar
        .save(&sig_path)
        .map_err(|e| ApiError::internal(format!("Save sidecar: {e}")))?;

    Ok(Json(json!({
        "merkle_root": root_hex,
        "leaves": leaves,
        "algorithm": sidecar.algorithm,
        "sig_path": sig_path,
    })))
}

async fn handle_verify(
    State(state): State<AppState>,
    Path((category, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let json = resolve_and_load(&state, &category, &name).await?;
    let sig_path = format!("{}/{}.sig", CACHE_DIR, name);

    let sidecar = merkle::SidecarFile::load(&sig_path)
        .map_err(|e| ApiError::not_found(format!("No sidecar found: {e}")))?;

    let sig_result = sidecar.verify_signature();
    let integrity = sidecar.merkle.verify_replay_json(&json);

    let (integrity_valid, tampered_section) = match integrity {
        merkle::VerifyResult::Valid => (true, None),
        merkle::VerifyResult::Tampered { section_index } => {
            let label = section_index.and_then(|i| {
                merkle::SECTION_LABELS
                    .get(i)
                    .map(|l| format!("{} ({})", i, l))
            });
            (false, label)
        }
    };

    Ok(Json(json!({
        "algorithm": sidecar.algorithm,
        "ed25519_valid": sig_result.ed25519_ok,
        "mldsa65_valid": sig_result.mldsa65_ok,
        "both_valid": sig_result.both_valid(),
        "integrity_valid": integrity_valid,
        "tampered_section": tampered_section,
    })))
}
