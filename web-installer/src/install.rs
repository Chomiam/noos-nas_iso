use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRequest {
    pub username: String,
    pub password: String,
    pub disk_path: String,
    pub filesystem: String, // "btrfs" or "ext4"
    #[serde(default = "default_hostname")]
    pub hostname: String,
}

fn default_hostname() -> String {
    "steveos-nas".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallStatusResponse {
    pub status: String, // "idle", "running", "success", "error"
    pub progress: u32,
    pub step: String,
    pub error: Option<String>,
    pub logs_count: usize,
}

pub struct InstallManager {
    pub status: String,
    pub progress: u32,
    pub step: String,
    pub error: Option<String>,
    pub logs: Vec<String>,
    pub tx: broadcast::Sender<String>,
}

impl InstallManager {
    pub fn new() -> (Self, broadcast::Sender<String>) {
        let (tx, _) = broadcast::channel(1024);
        (
            Self {
                status: "idle".to_string(),
                progress: 0,
                step: "En attente de configuration".to_string(),
                error: None,
                logs: Vec::new(),
                tx: tx.clone(),
            },
            tx,
        )
    }

    pub fn add_log(&mut self, line: String) {
        self.logs.push(line.clone());
        let _ = self.tx.send(line);
    }
}

async fn log(mgr: &Arc<Mutex<InstallManager>>, msg: &str) {
    let mut m = mgr.lock().await;
    m.add_log(msg.to_string());
}

async fn set_step(mgr: &Arc<Mutex<InstallManager>>, step: &str, progress: u32) {
    let mut m = mgr.lock().await;
    m.step = step.to_string();
    m.progress = progress;
    let line = format!("[PROGRESS] {}% - {}", progress, step);
    m.add_log(line);
}

pub async fn run_installation(
    manager: Arc<Mutex<InstallManager>>,
    req: InstallRequest,
) {
    {
        let mut m = manager.lock().await;
        m.status = "running".to_string();
        m.progress = 5;
        m.step = "Validation des paramètres".to_string();
        m.error = None;
        m.logs.clear();
        m.add_log("🚀 Démarrage de l'installation de STEvE_OS NAS Edition...".into());
        m.add_log(format!("Cible : {} | Système de fichiers : {} | Utilisateur : {}", req.disk_path, req.filesystem, req.username));
    }

    // Helper to run a command and stream stdout/stderr
    async fn exec_cmd(mgr: Arc<Mutex<InstallManager>>, mut cmd: Command) -> Result<(), String> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("❌ Erreur d'exécution de la commande : {}", e);
                log(&mgr, &err_msg).await;
                return Err(err_msg);
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mgr_out = mgr.clone();
        let out_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut m = mgr_out.lock().await;
                m.add_log(line);
            }
        });

        let mgr_err = mgr.clone();
        let err_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let mut m = mgr_err.lock().await;
                m.add_log(format!("[STDERR] {}", line));
            }
        });

        let (status, _, _) = tokio::join!(child.wait(), out_task, err_task);
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Commande terminée avec code d'erreur : {}", s.code().unwrap_or(-1))),
            Err(e) => Err(format!("Erreur lors de l'attente du processus : {}", e)),
        }
    }

    // Helper spécialisé pour nixos-install avec calcul dynamique de la progression (75% -> 95%)
    async fn exec_nixos_install(mgr: Arc<Mutex<InstallManager>>, mut cmd: Command) -> Result<(), String> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("❌ Erreur d'exécution de nixos-install : {}", e);
                log(&mgr, &err_msg).await;
                return Err(err_msg);
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        struct NixProgressState {
            total_items: u32,
            completed_items: u32,
            current_progress: u32,
        }

        let state = Arc::new(Mutex::new(NixProgressState {
            total_items: 0,
            completed_items: 0,
            current_progress: 20,
        }));

        async fn handle_line(mgr: &Arc<Mutex<InstallManager>>, state: &Arc<Mutex<NixProgressState>>, raw_line: &str, is_stderr: bool) {
            let line = raw_line.trim();

            // Détection du nombre total de paquets annoncés par Nix
            // ex: "these 18 derivations will be built:"
            // ex: "these 245 paths will be fetched (312.4 MiB):"
            if line.contains("derivations will be built") || line.contains("paths will be fetched") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                for (i, p) in parts.iter().enumerate() {
                    if *p == "these" && i + 1 < parts.len() {
                        if let Ok(count) = parts[i + 1].parse::<u32>() {
                            let mut s = state.lock().await;
                            s.total_items += count;
                        }
                    }
                }
            }

            let is_build = line.contains("building '");
            let is_copy = line.contains("copying path '");
            let is_fetch = line.contains("fetching path '");
            let is_unpack = line.contains("unpacking '");

            if is_build || is_copy || is_fetch || is_unpack {
                let mut s = state.lock().await;
                s.completed_items += 1;

                let total = if s.total_items > 0 {
                    s.total_items.max(s.completed_items)
                } else {
                    120.max(s.completed_items + 5)
                };

                let ratio = (s.completed_items as f32 / total as f32).min(1.0);
                let target_prog = (20.0 + ratio * 75.0).round() as u32;
                let target_prog = target_prog.clamp(20, 95);

                let pkg_name = if let Some(start) = line.find('\'') {
                    if let Some(end) = line[start + 1..].find('\'') {
                        let path = &line[start + 1..start + 1 + end];
                        let file_name = path.rsplit('/').next().unwrap_or(path);
                        if file_name.len() > 33 && file_name.as_bytes()[32] == b'-' {
                            file_name[33..].to_string()
                        } else {
                            file_name.to_string()
                        }
                    } else {
                        "paquet".to_string()
                    }
                } else {
                    "paquet".to_string()
                };

                let action = if is_build {
                    "Compilation"
                } else if is_copy {
                    "Copie"
                } else if is_fetch {
                    "Téléchargement"
                } else {
                    "Dépaquetage"
                };

                let step_desc = format!(
                    "{} de {} ({}/{})",
                    action, pkg_name, s.completed_items, total
                );

                s.current_progress = target_prog;

                let mut m = mgr.lock().await;
                m.progress = target_prog;
                m.step = step_desc.clone();
                let log_entry = if is_stderr {
                    format!("[STDERR] {}", raw_line)
                } else {
                    raw_line.to_string()
                };
                m.add_log(log_entry);
                m.add_log(format!("[PROGRESS] {}% - {}", target_prog, step_desc));
                return;
            }

            if line.contains("setting up /etc") {
                let mut s = state.lock().await;
                s.current_progress = 95;
                let step_desc = "Configuration du système et des services (/etc)";
                let mut m = mgr.lock().await;
                m.progress = 95;
                m.step = step_desc.to_string();
                m.add_log(format!("[PROGRESS] 95% - {}", step_desc));
            } else if line.contains("installing the boot loader") || line.contains("installing boot loader") {
                let mut s = state.lock().await;
                s.current_progress = 96;
                let step_desc = "Installation du chargeur de démarrage UEFI (systemd-boot)";
                let mut m = mgr.lock().await;
                m.progress = 96;
                m.step = step_desc.to_string();
                m.add_log(format!("[PROGRESS] 96% - {}", step_desc));
            } else if line.contains("installation finished") {
                let mut s = state.lock().await;
                s.current_progress = 97;
                let step_desc = "Installation du système STEvE_OS réussie";
                let mut m = mgr.lock().await;
                m.progress = 97;
                m.step = step_desc.to_string();
                m.add_log(format!("[PROGRESS] 97% - {}", step_desc));
            }

            let mut m = mgr.lock().await;
            if is_stderr {
                m.add_log(format!("[STDERR] {}", raw_line));
            } else {
                m.add_log(raw_line.to_string());
            }
        }

        let mgr_out = mgr.clone();
        let state_out = state.clone();
        let out_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                handle_line(&mgr_out, &state_out, &line, false).await;
            }
        });

        let mgr_err = mgr.clone();
        let state_err = state.clone();
        let err_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                handle_line(&mgr_err, &state_err, &line, true).await;
            }
        });

        let (status, _, _) = tokio::join!(child.wait(), out_task, err_task);
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Commande terminée avec code d'erreur : {}", s.code().unwrap_or(-1))),
            Err(e) => Err(format!("Erreur lors de l'attente du processus : {}", e)),
        }
    }

    // 1. Démontage préventif et nettoyage
    set_step(&manager, "Nettoyage et partitionnement du disque", 5).await;
    log(&manager, "Démontage préventif des anciens points de montage sur /mnt...").await;
    let _ = Command::new("umount").args(["-R", "/mnt"]).status().await;
    let _ = Command::new("swapoff").arg("-a").status().await;

    // Démontage forcé de toutes les partitions du disque cible
    if let Ok(entries) = std::fs::read_dir("/dev") {
        let base_name = req.disk_path.trim_start_matches("/dev/");
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(base_name) && name != base_name {
                let part_dev = format!("/dev/{}", name);
                let _ = Command::new("umount").args(["-f", &part_dev]).status().await;
            }
        }
    }
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    log(&manager, &format!("Effacement des anciennes signatures sur {}", req.disk_path)).await;
    let mut cmd = Command::new("wipefs");
    cmd.args(["-a", "-f", &req.disk_path]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        log(&manager, &format!("Avertissement wipefs : {}", e)).await;
    }

    log(&manager, &format!("Création de la table de partition GPT sur {}", req.disk_path)).await;
    let mut cmd = Command::new("sgdisk");
    cmd.args([
        "-Z",
        "-n", "1:0:+1024M", "-t", "1:ef00", "-c", "1:BOOT",
        "-n", "2:0:0", "-t", "2:8300", "-c", "2:ROOT",
        &req.disk_path,
    ]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        fail_install(manager, e).await;
        return;
    }

    log(&manager, "Actualisation des partitions auprès du noyau Linux...").await;
    let _ = Command::new("partprobe").arg(&req.disk_path).status().await;
    let _ = Command::new("udevadm").args(["settle"]).status().await;
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // Détermination des chemins de partition
    let (boot_part, root_part) = if req.disk_path.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        (format!("{}p1", req.disk_path), format!("{}p2", req.disk_path))
    } else {
        (format!("{}1", req.disk_path), format!("{}2", req.disk_path))
    };

    log(&manager, &format!("Partitions détectées : Boot={} | Racine={}", boot_part, root_part)).await;

    // 2. Formatage
    set_step(&manager, "Formatage des partitions", 8).await;
    let _ = Command::new("umount").args(["-f", &boot_part]).status().await;
    let _ = Command::new("umount").args(["-f", &root_part]).status().await;
    log(&manager, &format!("Formatage FAT32 de la partition EFI {}", boot_part)).await;
    let mut cmd = Command::new("mkfs.vfat");
    cmd.args(["-F", "32", "-n", "BOOT", &boot_part]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        fail_install(manager, e).await;
        return;
    }

    if req.filesystem == "btrfs" {
        log(&manager, &format!("Formatage Btrfs de la partition racine {}", root_part)).await;
        let mut cmd = Command::new("mkfs.btrfs");
        cmd.args(["-f", "-L", "ROOT", &root_part]);
        if let Err(e) = exec_cmd(manager.clone(), cmd).await {
            fail_install(manager, e).await;
            return;
        }

        // Création des sous-volumes Btrfs
        set_step(&manager, "Création des sous-volumes Btrfs (@, @home, @nix, @snapshots)", 11).await;
        let _ = Command::new("mkdir").args(["-p", "/mnt"]).status().await;
        let _ = Command::new("mount").args(["-t", "btrfs", &root_part, "/mnt"]).status().await;

        let _ = Command::new("btrfs").args(["subvolume", "create", "/mnt/@"]).status().await;
        let _ = Command::new("btrfs").args(["subvolume", "create", "/mnt/@home"]).status().await;
        let _ = Command::new("btrfs").args(["subvolume", "create", "/mnt/@nix"]).status().await;
        let _ = Command::new("btrfs").args(["subvolume", "create", "/mnt/@snapshots"]).status().await;

        let _ = Command::new("umount").arg("/mnt").status().await;

        // Montage définitif
        set_step(&manager, "Montage des systèmes de fichiers", 13).await;
        let _ = Command::new("mount").args(["-o", "subvol=@,compress=zstd,noatime", &root_part, "/mnt"]).status().await;
        let _ = Command::new("mkdir").args(["-p", "/mnt/home", "/mnt/nix", "/mnt/.snapshots", "/mnt/boot"]).status().await;
        let _ = Command::new("mount").args(["-o", "subvol=@home,compress=zstd", &root_part, "/mnt/home"]).status().await;
        let _ = Command::new("mount").args(["-o", "subvol=@nix,compress=zstd,noatime", &root_part, "/mnt/nix"]).status().await;
        let _ = Command::new("mount").args(["-o", "subvol=@snapshots,compress=zstd", &root_part, "/mnt/.snapshots"]).status().await;
        let _ = Command::new("mount").args([&boot_part, "/mnt/boot"]).status().await;
    } else {
        log(&manager, &format!("Formatage Ext4 de la partition racine {}", root_part)).await;
        let mut cmd = Command::new("mkfs.ext4");
        cmd.args(["-F", "-L", "ROOT", &root_part]);
        if let Err(e) = exec_cmd(manager.clone(), cmd).await {
            fail_install(manager, e).await;
            return;
        }

        set_step(&manager, "Montage des systèmes de fichiers", 13).await;
        let _ = Command::new("mkdir").args(["-p", "/mnt"]).status().await;
        let _ = Command::new("mount").args([&root_part, "/mnt"]).status().await;
        let _ = Command::new("mkdir").args(["-p", "/mnt/boot"]).status().await;
        let _ = Command::new("mount").args([&boot_part, "/mnt/boot"]).status().await;
    }

    // 3. Génération et injection de la configuration NixOS
    set_step(&manager, "Génération de la configuration matérielle STEvE_OS", 15).await;
    let mut cmd = Command::new("nixos-generate-config");
    cmd.args(["--root", "/mnt"]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        fail_install(manager, e).await;
        return;
    }

    set_step(&manager, "Préparation de la configuration STEvE_OS NAS Edition", 18).await;
    log(&manager, "Copie et personnalisation des fichiers de configuration...").await;

    // Préparation de /mnt/etc/nixos (la configuration NixOS est installée DIRECTEMENT dans /etc/nixos/)
    let target_cfg_dir = "/mnt/etc/nixos";
    let _ = Command::new("mkdir").args(["-p", target_cfg_dir]).status().await;

    // Sauvegarde temporaire du hardware-configuration.nix généré par nixos-generate-config
    let hw_config_src = "/mnt/etc/nixos/hardware-configuration.nix";
    let hw_config_backup = "/tmp/hw-config-backup.nix";
    if std::path::Path::new(hw_config_src).exists() {
        let _ = std::fs::copy(hw_config_src, hw_config_backup);
    }

    let source_dir = "/etc/steveos-nas-source";
    if std::path::Path::new(source_dir).exists() {
        log(&manager, "Copie de la configuration locale embarquée vers /mnt/etc/nixos...").await;
        let mut cmd = Command::new("cp");
        cmd.args(["-a", &format!("{}/.", source_dir), target_cfg_dir]);
        let _ = exec_cmd(manager.clone(), cmd).await;
    } else {
        log(&manager, "Clonage du dépôt officiel GitHub Chomiam/steve_os-nix vers /mnt/etc/nixos...").await;
        let tmp_clone_dir = "/tmp/steveos-nas-repo";
        let _ = Command::new("rm").args(["-rf", tmp_clone_dir]).status().await;

        let mut clone_cmd = Command::new("git");
        clone_cmd.args(["clone", "--depth=1", "https://github.com/Chomiam/steve_os-nix.git", tmp_clone_dir]);
        if let Err(e) = exec_cmd(manager.clone(), clone_cmd).await {
            fail_install(manager, e).await;
            return;
        }

        // Déploiement de l'arbre Git complet (y compris .git, flake.nix, etc.) directement dans /mnt/etc/nixos/
        let mut cp_repo = Command::new("cp");
        cp_repo.args(["-a", &format!("{}/.", tmp_clone_dir), target_cfg_dir]);
        let _ = exec_cmd(manager.clone(), cp_repo).await;
        let _ = Command::new("rm").args(["-rf", tmp_clone_dir]).status().await;
    }

    // Restauration de hardware-configuration.nix à la racine et dans hosts/nas/
    let _ = Command::new("mkdir").args(["-p", &format!("{}/hosts/nas", target_cfg_dir)]).status().await;
    if std::path::Path::new(hw_config_backup).exists() {
        let _ = std::fs::copy(hw_config_backup, &format!("{}/hardware-configuration.nix", target_cfg_dir));
        let _ = std::fs::copy(hw_config_backup, &format!("{}/hosts/nas/hardware-configuration.nix", target_cfg_dir));
        let _ = std::fs::remove_file(hw_config_backup);
    }

    // Création d'un lien symbolique de rétrocompatibilité /mnt/etc/nixos/steveos-nas -> .
    let _ = Command::new("ln").args(["-sfn", ".", &format!("{}/steveos-nas", target_cfg_dir)]).status().await;

    // Génération du mot de passe haché
    let password_hash = match Command::new("openssl")
        .args(["passwd", "-6", &req.password])
        .output()
        .await
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => {
            log(&manager, "Avertissement : utilisation de mkpasswd pour le hash...").await;
            let o = Command::new("mkpasswd").args(["-m", "sha-512", &req.password]).output().await;
            o.map(|res| String::from_utf8_lossy(&res.stdout).trim().to_string()).unwrap_or_default()
        }
    };

    // Écriture du fichier vars.nix personnalisé
    let vars_content = format!(
r#"# Variables générées automatiquement par l'installateur STEvE_OS NAS
{{
  hostName = "{}";
  timeZone = "Europe/Paris";
  defaultLocale = "fr_FR.UTF-8";

  user = {{
    username = "{}";
    description = "Administrateur STEvE_OS NAS";
    hashedPassword = "{}";
  }};

  network = {{
    dashboardPort = 9339;
    firewall = {{
      enable = true;
      preset = "nas";
    }};
  }};

  services = {{
    jellyfin = {{
      enable = false;
      openFirewall = true;
    }};
    samba = {{
      enable = true;
    }};
    nfs = {{
      enable = false;
    }};
  }};
}}
"#,
        req.hostname, req.username, password_hash
    );

    let _ = std::fs::write(format!("{}/vars.nix", target_cfg_dir), &vars_content);
    log(&manager, "Fichier /mnt/etc/nixos/vars.nix configuré avec succès.").await;

    // Suivre les fichiers dans git pour que Nix Flake les prenne en compte
    let mut git_add = Command::new("git");
    git_add.args(["-C", target_cfg_dir, "add", "-A"]);
    let _ = exec_cmd(manager.clone(), git_add).await;

    // 4. Lancement de nixos-install avec le Flake STEvE_OS
    set_step(&manager, "Compilation et installation du système STEvE_OS", 20).await;
    log(&manager, "Installation du système STEvE_OS en cours... Cette étape peut prendre quelques minutes.").await;

    let mut cmd = Command::new("nixos-install");
    cmd.args([
        "--flake",
        &format!("{}#nas", target_cfg_dir),
        "--no-root-password",
        "--impure",
    ]);
    if let Err(e) = exec_nixos_install(manager.clone(), cmd).await {
        fail_install(manager, e).await;
        return;
    }

    // 5. Finalisation
    set_step(&manager, "Finalisation de l'installation et synchronisation des disques", 98).await;
    let _ = Command::new("sync").status().await;

    {
        let mut m = manager.lock().await;
        m.status = "success".to_string();
        m.progress = 100;
        m.step = "Installation terminée avec succès !".to_string();
        m.add_log("🎉 ================================================================= 🎉".into());
        m.add_log("🎉 Félicitations ! STEvE_OS NAS Edition est maintenant installé sur votre disque !".into());
        m.add_log("🎉 Retirez la clé USB / support d'installation et redémarrez votre NAS.".into());
        m.add_log("🎉 Le tableau de bord web apparaîtra automatiquement sur le port 9339.".into());
        m.add_log("🎉 ================================================================= 🎉".into());
    }
}

async fn fail_install(manager: Arc<Mutex<InstallManager>>, err: String) {
    let mut m = manager.lock().await;
    m.status = "error".to_string();
    m.error = Some(err.clone());
    m.add_log(format!("💥 ÉCHEC DE L'INSTALLATION : {}", err));
}
