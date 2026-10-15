use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRequest {
    pub username: String,
    #[serde(default)]
    pub full_name: Option<String>,
    pub password: String,
    pub disk_path: String,
    pub filesystem: String, // "btrfs" or "ext4"
    #[serde(default = "default_hostname")]
    pub hostname: String,
}

fn default_hostname() -> String {
    "noos-nas".to_string()
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
        m.add_log("🚀 Démarrage de l'installation de Noos NAS Edition...".into());
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
                let step_desc = "Installation du système Noos NAS réussie";
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

    // 1. Démontage préventif et nettoyage approfondi des ressources disque
    set_step(&manager, "Nettoyage et partitionnement du disque", 5).await;
    log(&manager, "Démontage préventif des anciens points de montage et libération du disque...").await;
    let _ = Command::new("umount").args(["-R", "/mnt"]).status().await;
    let _ = Command::new("umount").args(["-R", "-l", "/mnt"]).status().await;
    let _ = Command::new("swapoff").arg("-a").status().await;

    let base_name = req.disk_path.trim_start_matches("/dev/").to_string();

    // 1a. Démontage de tous les points de montage actifs liés au disque cible via /proc/mounts
    if let Ok(mounts_content) = std::fs::read_to_string("/proc/mounts") {
        let mut target_mounts: Vec<(String, String)> = Vec::new();
        for line in mounts_content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let dev = parts[0];
                let mp = parts[1];
                if dev.starts_with(&format!("/dev/{}", base_name))
                    || mp == "/mnt"
                    || mp.starts_with("/mnt/")
                {
                    target_mounts.push((dev.to_string(), mp.to_string()));
                }
            }
        }
        // Démonter d'abord les sous-montages les plus profonds
        target_mounts.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        for (dev, mp) in target_mounts {
            let _ = Command::new("umount").args(["-R", "-l", &mp]).status().await;
            let _ = Command::new("umount").args(["-f", &dev]).status().await;
        }
    }

    // 1b. Désactivation des swaps spécifiques au disque via /proc/swaps
    if let Ok(swaps_content) = std::fs::read_to_string("/proc/swaps") {
        for line in swaps_content.lines().skip(1) {
            if let Some(dev) = line.split_whitespace().next() {
                if dev.starts_with(&format!("/dev/{}", base_name)) {
                    let _ = Command::new("swapoff").arg(dev).status().await;
                }
            }
        }
    }

    // 1c. Libération des pools ZFS, volumes LVM, conteneurs DM/LUKS et grappes RAID mdadm éventuels
    let _ = Command::new("zpool").args(["export", "-a"]).status().await;
    let _ = Command::new("vgchange").args(["-an"]).status().await;

    // Collecter toutes les partitions existantes du disque depuis /sys/class/block et /dev
    let mut existing_parts: Vec<String> = Vec::new();
    for dir in &["/sys/class/block", "/dev"] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&base_name) && name != base_name && !existing_parts.contains(&name) {
                    existing_parts.push(name);
                }
            }
        }
    }
    existing_parts.sort();

    // Libérer les holders noyau (/sys/class/block/<part>/holders : dm-*, md*)
    let mut check_blocks = existing_parts.clone();
    check_blocks.push(base_name.clone());
    for blk in &check_blocks {
        let holders_dir = format!("/sys/class/block/{}/holders", blk);
        if let Ok(holders) = std::fs::read_dir(&holders_dir) {
            for holder in holders.flatten() {
                let h_name = holder.file_name().to_string_lossy().to_string();
                let h_dev = format!("/dev/{}", h_name);
                let _ = Command::new("umount").args(["-R", "-l", &h_dev]).status().await;
                let _ = Command::new("swapoff").arg(&h_dev).status().await;
                if h_name.starts_with("dm-") {
                    let _ = Command::new("dmsetup").args(["remove", "-f", &h_name]).status().await;
                } else if h_name.starts_with("md") {
                    let _ = Command::new("mdadm").args(["--stop", &h_dev]).status().await;
                    let _ = std::fs::write(format!("/sys/class/block/{}/md/array_state", h_name), "clear\n");
                }
            }
        }
    }

    // 1d. Terminer tout processus tiers maintenant un descripteur ouvert sur /dev/<base_name>*
    let my_pid = std::process::id();
    if let Ok(proc_entries) = std::fs::read_dir("/proc") {
        for proc_entry in proc_entries.flatten() {
            if let Ok(pid) = proc_entry.file_name().to_string_lossy().parse::<u32>() {
                if pid <= 1 || pid == my_pid {
                    continue;
                }
                let fd_dir = format!("/proc/{}/fd", pid);
                if let Ok(fds) = std::fs::read_dir(&fd_dir) {
                    let mut holds_disk = false;
                    for fd in fds.flatten() {
                        if let Ok(target) = std::fs::read_link(fd.path()) {
                            let target_str = target.to_string_lossy();
                            if target_str.starts_with(&format!("/dev/{}", base_name)) {
                                holds_disk = true;
                                break;
                            }
                        }
                    }
                    if holds_disk {
                        let comm = std::fs::read_to_string(format!("/proc/{}/comm", pid))
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        // Ne pas tuer udevd ni nixos-install
                        if !comm.contains("udevd") && !comm.contains("systemd") && !comm.contains("noos") {
                            log(&manager, &format!("Fermeture du processus {} (PID {}) utilisant {}...", comm, pid, req.disk_path)).await;
                            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status().await;
                        }
                    }
                }
            }
        }
    }

    // 1e. Démontage, effacement des signatures de chaque partition existante et suppression en mémoire noyau
    for part_name in &existing_parts {
        let part_dev = format!("/dev/{}", part_name);
        let _ = Command::new("umount").args(["-R", "-l", &part_dev]).status().await;
        let _ = Command::new("umount").args(["-f", &part_dev]).status().await;
        let _ = Command::new("swapoff").arg(&part_dev).status().await;
        let _ = Command::new("wipefs").args(["-a", "-f", &part_dev]).status().await;
    }

    // Suppression explicite des anciennes partitions dans la table du noyau Linux (BLKPG_DEL_PARTITION)
    let _ = Command::new("partx").args(["-d", &req.disk_path]).status().await;
    for pno in 1..=16 {
        let _ = Command::new("delpart").args([&req.disk_path, &pno.to_string()]).status().await;
    }
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    log(&manager, &format!("Effacement des anciennes signatures sur {}", req.disk_path)).await;
    let mut cmd = Command::new("wipefs");
    cmd.args(["-a", "-f", &req.disk_path]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        log(&manager, &format!("Avertissement wipefs : {}", e)).await;
    }

    // Destruction propre des anciennes tables GPT/MBR (séparée de la création pour éviter la collision udev)
    let mut zap_cmd = Command::new("sgdisk");
    zap_cmd.args(["-Z", &req.disk_path]);
    if let Err(e) = exec_cmd(manager.clone(), zap_cmd).await {
        log(&manager, &format!("Avertissement sgdisk -Z : {}", e)).await;
    }
    let _ = Command::new("partx").args(["-d", &req.disk_path]).status().await;
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    log(&manager, &format!("Création de la table de partition GPT sur {}", req.disk_path)).await;
    let mut cmd = Command::new("sgdisk");
    cmd.args([
        "-o",
        "-n", "1:0:+1024M", "-t", "1:ef00", "-c", "1:BOOT",
        "-n", "2:0:0", "-t", "2:8300", "-c", "2:ROOT",
        &req.disk_path,
    ]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        fail_install(manager, e).await;
        return;
    }

    // Détermination des chemins et noms courts de partition
    let (boot_part, root_part) = if req.disk_path.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        (format!("{}p1", req.disk_path), format!("{}p2", req.disk_path))
    } else {
        (format!("{}1", req.disk_path), format!("{}2", req.disk_path))
    };
    let boot_name = boot_part.trim_start_matches("/dev/").to_string();
    let root_name = root_part.trim_start_matches("/dev/").to_string();

    log(&manager, "Actualisation des partitions auprès du noyau Linux...").await;
    let _ = Command::new("sync").status().await;
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

    // Synchronisation multi-outils de la table de partitions auprès du noyau (BLKRRPART + BLKPG)
    async fn sync_kernel_partitions(disk_path: &str, boot_name: &str, root_name: &str) {
        let _ = Command::new("partprobe").arg(disk_path).status().await;
        let _ = Command::new("blockdev").args(["--rereadpt", disk_path]).status().await;
        let _ = Command::new("partx").args(["-u", disk_path]).status().await;
        let _ = Command::new("partx").args(["-a", disk_path]).status().await;

        // Lecture directe de la géométrie GPT sur disque via `partx -g -o NR,START,SECTORS`
        // pour forcer BLKPG_RESIZE_PARTITION (resizepart) et BLKPG_ADD_PARTITION (addpart)
        // même si BLKRRPART a échoué avec EBUSY.
        if let Ok(out) = Command::new("partx")
            .args(["-g", "-o", "NR,START,SECTORS", disk_path])
            .output()
            .await
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() == 3 {
                        let (nr, start, sectors) = (cols[0], cols[1], cols[2]);
                        let part_sys = if nr == "1" {
                            format!("/sys/class/block/{}", boot_name)
                        } else if nr == "2" {
                            format!("/sys/class/block/{}", root_name)
                        } else {
                            continue;
                        };

                        if std::path::Path::new(&part_sys).exists() {
                            // Si la partition existe déjà dans le noyau avec une ancienne taille (ex: sda1 250Go),
                            // la redimensionner à chaud libère l'espace pour permettre l'ajout de sda2 !
                            let _ = Command::new("resizepart").args([disk_path, nr, sectors]).status().await;
                        } else {
                            let _ = Command::new("addpart").args([disk_path, nr, start, sectors]).status().await;
                            let _ = Command::new("partx").args(["-a", "--nr", nr, disk_path]).status().await;
                            let _ = Command::new("partx").args(["-u", "--nr", nr, disk_path]).status().await;
                        }
                    }
                }
            }
        }

        let _ = Command::new("udevadm")
            .args(["trigger", "--subsystem-match=block", "--action=change"])
            .status()
            .await;
        let _ = Command::new("udevadm")
            .args(["trigger", "--subsystem-match=block", "--action=add"])
            .status()
            .await;
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        // Si le noyau connaît la partition dans /sys/class/block/<part>/dev mais que le noeud /dev/<part>
        // n'a pas encore été créé par udevd, création directe du noeud bloc via mknod.
        for part_short in [boot_name, root_name] {
            let dev_node = format!("/dev/{}", part_short);
            let sys_dev = format!("/sys/class/block/{}/dev", part_short);
            if !std::path::Path::new(&dev_node).exists() {
                if let Ok(maj_min) = std::fs::read_to_string(&sys_dev) {
                    let parts: Vec<&str> = maj_min.trim().split(':').collect();
                    if parts.len() == 2 {
                        let _ = Command::new("mknod")
                            .args(["-m", "0660", &dev_node, "b", parts[0], parts[1]])
                            .status()
                            .await;
                    }
                }
            }
        }
    }

    // Boucle de vérification et d'auto-réparation jusqu'à apparition effective de boot_part et root_part
    let mut partitions_ready = false;
    for attempt in 1..=15 {
        sync_kernel_partitions(&req.disk_path, &boot_name, &root_name).await;
        if std::path::Path::new(&boot_part).exists() && std::path::Path::new(&root_part).exists() {
            partitions_ready = true;
            break;
        }
        log(
            &manager,
            &format!(
                "Attente de la création des périphériques {} et {} par le noyau (tentative {}/15)...",
                boot_part, root_part, attempt
            ),
        )
        .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    if !partitions_ready {
        fail_install(
            manager,
            format!(
                "Les partitions {} et {} n'ont pas pu être créées par le noyau Linux (le disque {} est peut-être verrouillé ou utilisé par le système).",
                boot_part, root_part, req.disk_path
            ),
        )
        .await;
        return;
    }

    log(&manager, &format!("Partitions détectées : Boot={} | Racine={}", boot_part, root_part)).await;

    // 2. Formatage
    set_step(&manager, "Formatage des partitions", 8).await;
    let _ = Command::new("umount").args(["-R", "-l", &boot_part]).status().await;
    let _ = Command::new("umount").args(["-R", "-l", &root_part]).status().await;
    let _ = Command::new("wipefs").args(["-a", "-f", &boot_part]).status().await;
    let _ = Command::new("wipefs").args(["-a", "-f", &root_part]).status().await;
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

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
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        // Création des sous-volumes Btrfs
        set_step(&manager, "Création des sous-volumes Btrfs (@, @home, @nix, @snapshots)", 11).await;
        let _ = Command::new("mkdir").args(["-p", "/mnt"]).status().await;
        let mut mount_root = Command::new("mount");
        mount_root.args(["-t", "btrfs", &root_part, "/mnt"]);
        if let Err(e) = exec_cmd(manager.clone(), mount_root).await {
            fail_install(manager, e).await;
            return;
        }

        for subvol in ["/mnt/@", "/mnt/@home", "/mnt/@nix", "/mnt/@snapshots"] {
            let mut sub_cmd = Command::new("btrfs");
            sub_cmd.args(["subvolume", "create", subvol]);
            if let Err(e) = exec_cmd(manager.clone(), sub_cmd).await {
                fail_install(manager, e).await;
                return;
            }
        }

        let _ = Command::new("umount").arg("/mnt").status().await;

        // Montage définitif
        set_step(&manager, "Montage des systèmes de fichiers", 13).await;
        let mut m_at = Command::new("mount");
        m_at.args(["-o", "subvol=@,compress=zstd,noatime", &root_part, "/mnt"]);
        if let Err(e) = exec_cmd(manager.clone(), m_at).await {
            fail_install(manager, e).await;
            return;
        }
        let _ = Command::new("mkdir").args(["-p", "/mnt/home", "/mnt/nix", "/mnt/.snapshots", "/mnt/boot"]).status().await;
        let _ = Command::new("mount").args(["-o", "subvol=@home,compress=zstd", &root_part, "/mnt/home"]).status().await;
        let _ = Command::new("mount").args(["-o", "subvol=@nix,compress=zstd,noatime", &root_part, "/mnt/nix"]).status().await;
        let _ = Command::new("mount").args(["-o", "subvol=@snapshots,compress=zstd", &root_part, "/mnt/.snapshots"]).status().await;
        let mut m_boot = Command::new("mount");
        m_boot.args([&boot_part, "/mnt/boot"]);
        if let Err(e) = exec_cmd(manager.clone(), m_boot).await {
            fail_install(manager, e).await;
            return;
        }
    } else {
        log(&manager, &format!("Formatage Ext4 de la partition racine {}", root_part)).await;
        let mut cmd = Command::new("mkfs.ext4");
        cmd.args(["-F", "-L", "ROOT", &root_part]);
        if let Err(e) = exec_cmd(manager.clone(), cmd).await {
            fail_install(manager, e).await;
            return;
        }
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        set_step(&manager, "Montage des systèmes de fichiers", 13).await;
        let _ = Command::new("mkdir").args(["-p", "/mnt"]).status().await;
        let mut m_root = Command::new("mount");
        m_root.args([&root_part, "/mnt"]);
        if let Err(e) = exec_cmd(manager.clone(), m_root).await {
            fail_install(manager, e).await;
            return;
        }
        let _ = Command::new("mkdir").args(["-p", "/mnt/boot"]).status().await;
        let mut m_boot = Command::new("mount");
        m_boot.args([&boot_part, "/mnt/boot"]);
        if let Err(e) = exec_cmd(manager.clone(), m_boot).await {
            fail_install(manager, e).await;
            return;
        }
    }

    // 3. Génération et injection de la configuration NixOS
    set_step(&manager, "Génération de la configuration matérielle Noos NAS", 15).await;
    let mut cmd = Command::new("nixos-generate-config");
    cmd.args(["--root", "/mnt"]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        fail_install(manager, e).await;
        return;
    }

    set_step(&manager, "Préparation de la configuration Noos NAS Edition", 18).await;
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

    let source_dir = if std::path::Path::new("/etc/noos-nas-source").exists() { "/etc/noos-nas-source" } else { "/etc/steveos-nas-source" };
    if std::path::Path::new(source_dir).exists() {
        log(&manager, "Copie de la configuration locale embarquée vers /mnt/etc/nixos...").await;
        let mut cmd = Command::new("cp");
        cmd.args(["-a", &format!("{}/.", source_dir), target_cfg_dir]);
        let _ = exec_cmd(manager.clone(), cmd).await;
    } else {
        log(&manager, "Clonage du dépôt officiel GitHub Chomiam/steve_os-nix vers /mnt/etc/nixos...").await;
        let tmp_clone_dir = "/tmp/noos-nas-repo";
        let _ = Command::new("rm").args(["-rf", tmp_clone_dir]).status().await;

        let mut clone_cmd = Command::new("git");
        clone_cmd.args(["clone", "--depth=1", "https://github.com/Chomiam/noos-nas.git", tmp_clone_dir]);
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
    let _ = Command::new("ln").args(["-sfn", ".", &format!("{}/noos-nas", target_cfg_dir)]).status().await;
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

    let full_name = req
        .full_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("Administrateur Noos NAS");

    // Écriture du fichier vars.nix personnalisé
    let vars_content = format!(
r#"# Variables générées automatiquement par l'installateur Noos NAS
{{
  hostName = "{}";
  timeZone = "Europe/Paris";
  defaultLocale = "fr_FR.UTF-8";

  user = {{
    username = "{}";
    fullName = "{}";
    homeDirectory = "/home/{}";
    initialHashedPassword = "{}";
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
    dashboard = {{
      enable = true;
      port = 9339;
    }};
    jellyfin = {{
      enable = false;
      openFirewall = true;
    }};
    samba = {{
      enable = true;
    }};
    sftp = {{
      enable = true;
      port = 22;
    }};
    nfs = {{
      enable = false;
    }};
  }};
}}
"#,
        req.hostname, req.username, full_name, req.username, password_hash, password_hash
    );

    let _ = std::fs::write(format!("{}/vars.nix", target_cfg_dir), &vars_content);
    let _ = std::fs::write(format!("{}/vars.local.nix", target_cfg_dir), &vars_content);
    let _ = std::fs::write(format!("{}/.vars.nix.backup", target_cfg_dir), &vars_content);
    log(&manager, "Fichier /mnt/etc/nixos/vars.nix et sauvegardes configurés avec succès.").await;

    // Configuration explicite du canal de mise à jour initial sur "stable"
    let _ = Command::new("mkdir").args(["-p", "/mnt/var/lib/noos"]).status().await;
    let _ = std::fs::write("/mnt/var/lib/noos/update_channel", "stable\n");
    log(&manager, "Canal de mise à jour initial configuré sur 'stable' (/var/lib/noos/update_channel).").await;

    // Suivre les fichiers dans git pour que Nix Flake les prenne en compte
    let mut git_add = Command::new("git");
    git_add.args(["-C", target_cfg_dir, "add", "-A"]);
    let _ = exec_cmd(manager.clone(), git_add).await;

    // 4. Lancement de nixos-install avec le Flake Noos NAS
    set_step(&manager, "Compilation et installation du système Noos NAS", 20).await;
    log(&manager, "Installation du système Noos NAS en cours... Cette étape peut prendre quelques minutes.").await;

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
        m.add_log("🎉 Félicitations ! Noos NAS Edition est maintenant installé sur votre disque !".into());
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
