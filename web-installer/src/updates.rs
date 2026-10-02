use serde::Serialize;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckResponse {
    pub update_available: bool,
    pub current_commit: String,
    pub remote_commit: String,
    pub current_short: String,
    pub remote_short: String,
    pub message: String,
}

pub fn get_local_commit() -> String {
    // 0. Priorité absolue au commit dynamique écrit en RAM lors d'un update
    for path in &["/run/noos-current-commit", "/run/steveos-current-commit"] {
        if let Ok(c) = std::fs::read_to_string(path) {
            let trimmed = c.trim().to_string();
            if !trimmed.is_empty() && trimmed != "unknown" {
                return trimmed;
            }
        }
    }

    // 1. Priorité au fichier etc écrit par le flake NixOS
    for path in &["/etc/noos-iso-commit", "/etc/steveos-iso-commit"] {
        if let Ok(c) = std::fs::read_to_string(path) {
            let trimmed = c.trim().to_string();
            if !trimmed.is_empty() && trimmed != "unknown" {
                return trimmed;
            }
        }
    }

    // 2. Détection via git local si dans un repo de dev
    if let Ok(out) = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    // 3. Fallback sur le commit initial
    "b870c77b9c614da03252827c616159db287167ef".to_string()
}

pub async fn check_for_updates() -> UpdateCheckResponse {
    let local_commit = get_local_commit();
    let remote_url = if Command::new("git").args(["ls-remote", "https://github.com/Chomiam/noos-nas_iso.git", "HEAD"]).output().await.map(|o| o.status.success()).unwrap_or(false) { "https://github.com/Chomiam/noos-nas_iso.git" } else { "https://github.com/Chomiam/steveos-nas_iso.git" };

    let remote_commit = match Command::new("git")
        .args(["ls-remote", remote_url, "refs/heads/main"])
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.split_whitespace().next().unwrap_or("").to_string()
        }
        _ => String::new(),
    };

    let current_short = local_commit[..7.min(local_commit.len())].to_string();
    let remote_short = if remote_commit.len() >= 7 {
        remote_commit[..7].to_string()
    } else {
        remote_commit.clone()
    };

    if remote_commit.is_empty() {
        return UpdateCheckResponse {
            update_available: false,
            current_commit: local_commit.clone(),
            remote_commit: "".into(),
            current_short,
            remote_short: "".into(),
            message: "Impossible de contacter GitHub pour vérifier les mises à jour (vérifiez la connexion Internet).".into(),
        };
    }

    let update_available = !local_commit.is_empty() && local_commit != remote_commit;

    let message = if update_available {
        format!("Mise à jour disponible : {} → {}", current_short, remote_short)
    } else {
        format!("Installateur à jour ({})", current_short)
    };

    UpdateCheckResponse {
        update_available,
        current_commit: local_commit,
        remote_commit,
        current_short,
        remote_short,
        message,
    }
}

pub async fn apply_self_update() -> Result<String, String> {
    let remote_url = if Command::new("git").args(["ls-remote", "https://github.com/Chomiam/noos-nas_iso.git", "HEAD"]).output().await.map(|o| o.status.success()).unwrap_or(false) { "https://github.com/Chomiam/noos-nas_iso.git" } else { "https://github.com/Chomiam/steveos-nas_iso.git" };
    let remote_commit = match Command::new("git")
        .args(["ls-remote", remote_url, "refs/heads/main"])
        .output()
        .await
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            text.split_whitespace().next().unwrap_or("").to_string()
        }
        _ => String::new(),
    };

    // 1. Construction du paquet à jour via nix
    let mut cmd = Command::new("nix");
    cmd.args([
        "build",
        "--refresh",
        if Command::new("git").args(["ls-remote", "https://github.com/Chomiam/noos-nas_iso.git", "HEAD"]).output().await.map(|o| o.status.success()).unwrap_or(false) { "github:Chomiam/noos-nas_iso#web-installer" } else { "github:Chomiam/steveos-nas_iso#web-installer" },
        "--out-link",
        "/run/current-web-installer",
    ]);

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) => return Err(format!("Erreur lors de l'exécution de nix build : {}", e)),
    };

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Échec de la compilation de la mise à jour : {}", err));
    }

    // Sauvegarder le commit pour briser toute boucle de détection
    if !remote_commit.is_empty() {
        let _ = std::fs::write("/run/noos-current-commit", &remote_commit);
        let _ = std::fs::write("/run/steveos-current-commit", &remote_commit);
    }

    // 2. Déclenchement du redémarrage du service ou du processus
    tokio::spawn(async {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        // Tenter de redémarrer le service systemd
        let status = Command::new("systemctl")
            .args(["restart", "noos-web-installer"])
            .status()
            .await;

        if let Ok(s) = status {
            if s.success() {
                return;
            }
        }

        // Sinon redémarrer le binaire directement
        if let Ok(current_exe) = std::env::current_exe() {
            let target_bin = if std::path::Path::new("/run/current-web-installer/bin/steveos-web-installer").exists() {
                std::path::PathBuf::from("/run/current-web-installer/bin/steveos-web-installer")
            } else {
                current_exe
            };
            let _ = std::process::Command::new(target_bin).spawn();
            std::process::exit(0);
        }
    });

    Ok("Mise à jour téléchargée avec succès. Redémarrage de l'installateur en cours...".into())
}
