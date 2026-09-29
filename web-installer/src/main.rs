mod updates;
mod disks;
mod install;

use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use install::{run_installation, InstallManager, InstallRequest, InstallStatusResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tower_http::cors::CorsLayer;

pub struct AppState {
    pub manager: Arc<Mutex<InstallManager>>,
    pub tx: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() {
    let (manager, tx) = InstallManager::new();
    let state = Arc::new(AppState {
        manager: Arc::new(Mutex::new(manager)),
        tx,
    });

    let app = Router::new()
        .route("/", get(serve_index))
        .route("/css/style.css", get(serve_css))
        .route("/js/installer.js", get(serve_js))
        .route("/api/disks", get(get_disks))
        .route("/api/validate-user", post(validate_user_route))
        .route("/api/install", post(start_install))
        .route("/api/status", get(get_status))
        .route("/api/logs", get(get_logs))
        .route("/api/install/stream", get(install_stream))
        .route("/api/reboot", post(trigger_reboot))
        .route("/api/network", get(get_network_info))
        .route("/api/update/check", get(check_update_route))
        .route("/api/update/apply", post(apply_update_route))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let bind_addr = format!("0.0.0.0:{}", port);
    println!("🚀 STEvE_OS NAS Web Installer démarré sur http://{}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// --- Handlers statiques (embarqués dans le binaire) ---

async fn serve_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))],
        include_str!("../frontend/index.html"),
    )
}

async fn serve_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"))],
        include_str!("../frontend/css/style.css"),
    )
}

async fn serve_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, HeaderValue::from_static("application/javascript; charset=utf-8"))],
        include_str!("../frontend/js/installer.js"),
    )
}

// --- Handlers API ---

async fn get_disks() -> Json<Vec<disks::DiskInfo>> {
    Json(disks::list_available_disks())
}

#[derive(Deserialize)]
struct ValidateUserReq {
    username: String,
}

#[derive(Serialize)]
struct ValidateUserResp {
    valid: bool,
    error: Option<String>,
}

async fn validate_user_route(Json(req): Json<ValidateUserReq>) -> Json<ValidateUserResp> {
    match validate_username(&req.username) {
        Ok(_) => Json(ValidateUserResp {
            valid: true,
            error: None,
        }),
        Err(e) => Json(ValidateUserResp {
            valid: false,
            error: Some(e),
        }),
    }
}

pub fn validate_username(username: &str) -> Result<(), String> {
    let u = username.trim();
    if u.is_empty() {
        return Err("Le nom d'utilisateur ne peut pas être vide.".into());
    }
    if u.len() < 2 {
        return Err("Le nom d'utilisateur doit comporter au moins 2 caractères.".into());
    }
    if u.len() > 32 {
        return Err("Le nom d'utilisateur ne doit pas dépasser 32 caractères.".into());
    }

    let first = u.chars().next().unwrap();
    if !first.is_ascii_lowercase() && first != '_' {
        return Err("Le nom d'utilisateur doit obligatoirement commencer par une lettre minuscule (a-z) ou un souligné (_).".into());
    }

    for c in u.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '-' && c != '_' {
            if c.is_ascii_uppercase() {
                return Err("Les majuscules sont interdites dans le nom d'utilisateur Linux.".into());
            }
            if c.is_whitespace() {
                return Err("Les espaces sont strictement interdits dans le nom d'utilisateur.".into());
            }
            return Err(format!("Le caractère '{}' est interdit. Seuls a-z, 0-9, tiret (-) et souligné (_) sont autorisés.", c));
        }
    }

    let reserved = [
        "root", "daemon", "bin", "sys", "sync", "games", "man", "lp", "mail", "news",
        "uucp", "proxy", "www-data", "backup", "list", "irc", "gnats", "nobody",
        "systemd-network", "systemd-resolve", "messagebus", "sshd", "nixbld", "docker",
        "storage", "jellyfin", "admin", "administrator", "guest"
    ];
    if reserved.contains(&u) {
        return Err(format!("Le nom d'utilisateur '{}' est réservé par le système.", u));
    }

    Ok(())
}

#[derive(Serialize)]
struct GenericResponse {
    ok: bool,
    message: String,
}

async fn start_install(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InstallRequest>,
) -> Response {
    if let Err(e) = validate_username(&req.username) {
        return (StatusCode::BAD_REQUEST, Json(GenericResponse { ok: false, message: e })).into_response();
    }
    if req.password.len() < 4 {
        return (StatusCode::BAD_REQUEST, Json(GenericResponse { ok: false, message: "Le mot de passe doit comporter au moins 4 caractères.".into() })).into_response();
    }
    if !req.disk_path.starts_with("/dev/") {
        return (StatusCode::BAD_REQUEST, Json(GenericResponse { ok: false, message: "Périphérique de disque invalide.".into() })).into_response();
    }

    let is_running = {
        let m = state.manager.lock().await;
        m.status == "running"
    };

    if is_running {
        return (StatusCode::CONFLICT, Json(GenericResponse { ok: false, message: "Une installation est déjà en cours d'exécution.".into() })).into_response();
    }

    let mgr = state.manager.clone();
    tokio::spawn(async move {
        run_installation(mgr, req).await;
    });

    (StatusCode::OK, Json(GenericResponse { ok: true, message: "Installation lancée avec succès.".into() })).into_response()
}

async fn get_status(State(state): State<Arc<AppState>>) -> Json<InstallStatusResponse> {
    let m = state.manager.lock().await;
    Json(InstallStatusResponse {
        status: m.status.clone(),
        progress: m.progress,
        step: m.step.clone(),
        error: m.error.clone(),
        logs_count: m.logs.len(),
    })
}

#[derive(Serialize)]
struct InstallLogsResponse {
    logs: Vec<String>,
}

async fn get_logs(State(state): State<Arc<AppState>>) -> Json<InstallLogsResponse> {
    let m = state.manager.lock().await;
    Json(InstallLogsResponse {
        logs: m.logs.clone(),
    })
}

async fn install_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(line) => Some(Ok(Event::default().data(line))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Serialize)]
struct NetworkInfo {
    ip: String,
    hostname: String,
}

pub fn get_primary_lan_ip() -> String {
    // 1. Détection via socket UDP vers 8.8.8.8 (ne transmet aucun paquet, interroge la table de routage du noyau Linux)
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(local_addr) = socket.local_addr() {
                let ip = local_addr.ip().to_string();
                if !ip.starts_with("127.") && !ip.starts_with("169.254.") {
                    return ip;
                }
            }
        }
    }

    // 2. Détection via ip route get 1.1.1.1
    if let Ok(output) = std::process::Command::new("ip")
        .args(["-4", "route", "get", "1.1.1.1"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parts: Vec<&str> = stdout.split_whitespace().collect();
            for i in 0..parts.len() {
                if parts[i] == "src" && i + 1 < parts.len() {
                    let ip = parts[i + 1].trim();
                    if !ip.starts_with("127.") && !ip.starts_with("169.254.") {
                        return ip.to_string();
                    }
                }
            }
        }
    }

    // 3. Détection via ip -4 -o addr show scope global
    if let Ok(output) = std::process::Command::new("ip")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for part in parts {
                    if part.contains('/') {
                        if let Some(ip) = part.split('/').next() {
                            let ip = ip.trim();
                            if !ip.starts_with("127.") && !ip.starts_with("169.254.") {
                                return ip.to_string();
                            }
                        }
                    }
                }
            }
        }
    }

    // 4. Détection via hostname -I
    if let Ok(output) = std::process::Command::new("hostname").arg("-I").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for part in stdout.split_whitespace() {
                let ip = part.trim();
                if !ip.is_empty() && !ip.starts_with("127.") && !ip.starts_with("169.254.") {
                    return ip.to_string();
                }
            }
        }
    }

    "127.0.0.1".to_string()
}

async fn get_network_info() -> Json<NetworkInfo> {
    let primary_ip = get_primary_lan_ip();
    let hostname = std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "steveos-nas".into());

    Json(NetworkInfo {
        ip: primary_ip,
        hostname,
    })
}

async fn trigger_reboot() -> Json<GenericResponse> {
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        let _ = tokio::process::Command::new("systemctl").arg("reboot").status().await;
    });

    Json(GenericResponse {
        ok: true,
        message: "Redémarrage initié. Le NAS redémarrera dans 3 secondes.".into(),
    })
}

async fn check_update_route() -> Json<updates::UpdateCheckResponse> {
    Json(updates::check_for_updates().await)
}

async fn apply_update_route() -> Response {
    match updates::apply_self_update().await {
        Ok(msg) => (StatusCode::OK, Json(GenericResponse { ok: true, message: msg })).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(GenericResponse { ok: false, message: err })).into_response(),
    }
}
