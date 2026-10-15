use serde::{Deserialize, Serialize};
use std::os::unix::io::AsRawFd;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{broadcast, Mutex};

const BLKRRPART: u64 = 0x125f;
const BLKPG: u64 = 0x1269;
const BLKPG_ADD_PARTITION: i32 = 1;
const BLKPG_DEL_PARTITION: i32 = 2;
const BLKPG_RESIZE_PARTITION: i32 = 3;
const STOP_ARRAY: u64 = 0x0932;
const LOOP_CLR_FD: u64 = 0x4c01;

#[repr(C)]
struct BlkpgIoctlArg {
    op: i32,
    flags: i32,
    datalen: i32,
    data: *mut BlkpgPartition,
}

#[repr(C)]
struct BlkpgPartition {
    start: i64,
    length: i64,
    pno: i32,
    devname: [u8; 64],
    volname: [u8; 64],
}

const _: () = assert!(std::mem::size_of::<BlkpgPartition>() == 152);
const _: () = assert!(std::mem::size_of::<BlkpgIoctlArg>() == 24);

extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn swapoff(path: *const i8) -> i32;
}

fn blkpg_op(
    disk_path: &str,
    op: i32,
    pno: i32,
    start_sect_512: u64,
    len_sect_512: u64,
) -> Result<(), std::io::Error> {
    let file = std::fs::OpenOptions::new().read(true).open(disk_path)?;
    let mut part = BlkpgPartition {
        start: (start_sect_512 as i64) * 512,
        length: (len_sect_512 as i64) * 512,
        pno,
        devname: [0u8; 64],
        volname: [0u8; 64],
    };
    let mut arg = BlkpgIoctlArg {
        op,
        flags: 0,
        datalen: std::mem::size_of::<BlkpgPartition>() as i32,
        data: &mut part as *mut BlkpgPartition,
    };
    let ret = unsafe { ioctl(file.as_raw_fd(), BLKPG, &mut arg as *mut BlkpgIoctlArg) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn blkrrpart_ioctl(disk_path: &str) -> Result<(), std::io::Error> {
    let file = std::fs::OpenOptions::new().read(true).open(disk_path)?;
    let ret = unsafe { ioctl(file.as_raw_fd(), BLKRRPART) };
    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[derive(Debug, Clone)]
struct KernelPartInfo {
    name: String,
    pno: i32,
    start_sect: u64,
    size_sect: u64,
    dev_maj_min: String,
    holders: Vec<String>,
}

fn list_kernel_partitions(base_name: &str) -> Vec<KernelPartInfo> {
    let mut parts = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/block") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(base_name) && name != base_name {
                let sys_dir = format!("/sys/class/block/{}", name);
                let part_file = format!("{}/partition", sys_dir);
                if let Ok(pno_str) = std::fs::read_to_string(&part_file) {
                    if let Ok(pno) = pno_str.trim().parse::<i32>() {
                        let start_sect = std::fs::read_to_string(format!("{}/start", sys_dir))
                            .ok()
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .unwrap_or(0);
                        let size_sect = std::fs::read_to_string(format!("{}/size", sys_dir))
                            .ok()
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .unwrap_or(0);
                        let dev_maj_min = std::fs::read_to_string(format!("{}/dev", sys_dir))
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        let mut holders = Vec::new();
                        if let Ok(h_entries) = std::fs::read_dir(format!("{}/holders", sys_dir)) {
                            for h in h_entries.flatten() {
                                holders.push(h.file_name().to_string_lossy().to_string());
                            }
                        }
                        parts.push(KernelPartInfo {
                            name,
                            pno,
                            start_sect,
                            size_sect,
                            dev_maj_min,
                            holders,
                        });
                    }
                }
            }
        }
    }
    parts.sort_by_key(|p| p.pno);
    parts
}

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
    // Enrichir PATH avec tous les chemins système NixOS (utile en cas de mise à jour à chaud sur une ancienne ISO)
    let current_path = std::env::var("PATH").unwrap_or_default();
    let extra_paths = "/run/current-system/sw/bin:/run/current-system/sw/sbin:/run/wrappers/bin:/sbin:/usr/sbin:/bin:/usr/bin";
    if !current_path.contains("/run/current-system/sw/bin") {
        std::env::set_var("PATH", format!("{}:{}", current_path, extra_paths));
    }

    {
        let mut m = manager.lock().await;
        m.status = "running".to_string();
        m.progress = 5;
        m.step = "Validation des paramètres".to_string();
        m.error = None;
        m.logs.clear();
    }

    // Laisser 250ms au navigateur pour ouvrir le flux EventSource (/api/install/stream)
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    log(&manager, "🚀 Démarrage de l'installation de Noos NAS Edition...").await;
    log(
        &manager,
        &format!(
            "Cible : {} | Système de fichiers : {} | Utilisateur : {}",
            req.disk_path, req.filesystem, req.username
        ),
    )
    .await;

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
    log(&manager, "Inspection et libération des ressources du disque cible...").await;

    let base_name = req.disk_path.trim_start_matches("/dev/").to_string();
    let total_sectors: u64 = std::fs::read_to_string(format!("/sys/class/block/{}/size", base_name))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let logical_block_size: u64 = std::fs::read_to_string(format!(
        "/sys/class/block/{}/queue/logical_block_size",
        base_name
    ))
    .ok()
    .and_then(|s| s.trim().parse::<u64>().ok())
    .unwrap_or(512)
    .max(512);
    let sectors_per_lba: u64 = (logical_block_size / 512).max(1);

    let initial_parts = list_kernel_partitions(&base_name);
    if initial_parts.is_empty() {
        log(
            &manager,
            &format!(
                "[DIAG] Disque {} : {} secteurs (512o), bloc logique {}o, aucune partition active en mémoire noyau.",
                req.disk_path, total_sectors, logical_block_size
            ),
        )
        .await;
    } else {
        for p in &initial_parts {
            log(
                &manager,
                &format!(
                    "[DIAG] Partition noyau existante : /dev/{} (pno={}, start={}, size={}, dev={}, holders={:?})",
                    p.name, p.pno, p.start_sect, p.size_sect, p.dev_maj_min, p.holders
                ),
            )
            .await;
        }
    }

    // Collecter tous les major:minor associés au disque et à ses partitions/holders
    let mut target_maj_mins: Vec<String> = Vec::new();
    if let Ok(dm) = std::fs::read_to_string(format!("/sys/class/block/{}/dev", base_name)) {
        let s = dm.trim().to_string();
        if !s.is_empty() {
            target_maj_mins.push(s);
        }
    }
    for p in &initial_parts {
        if !p.dev_maj_min.is_empty() && !target_maj_mins.contains(&p.dev_maj_min) {
            target_maj_mins.push(p.dev_maj_min.clone());
        }
        for h in &p.holders {
            if let Ok(h_dm) = std::fs::read_to_string(format!("/sys/class/block/{}/dev", h)) {
                let s = h_dm.trim().to_string();
                if !s.is_empty() && !target_maj_mins.contains(&s) {
                    target_maj_mins.push(s);
                }
            }
        }
    }

    let _ = Command::new("umount").args(["-R", "-l", "/mnt"]).status().await;
    let _ = Command::new("swapoff").arg("-a").status().await;

    // 1a. Démontage dans TOUS les namespaces de montage (/proc/[0-9]*/mountinfo)
    let my_pid = std::process::id();
    if let Ok(proc_entries) = std::fs::read_dir("/proc") {
        for proc_entry in proc_entries.flatten() {
            if let Ok(pid) = proc_entry.file_name().to_string_lossy().parse::<u32>() {
                let minfo_path = format!("/proc/{}/mountinfo", pid);
                if let Ok(content) = std::fs::read_to_string(&minfo_path) {
                    let mut mounts_to_umount: Vec<String> = Vec::new();
                    for line in content.lines() {
                        let cols: Vec<&str> = line.split_whitespace().collect();
                        if cols.len() >= 5 {
                            let maj_min = cols[2];
                            let mp = cols[4];
                            let mut source = "";
                            if let Some(sep_idx) = cols.iter().position(|&c| c == "-") {
                                if sep_idx + 2 < cols.len() {
                                    source = cols[sep_idx + 2];
                                }
                            }
                            let matches_disk = target_maj_mins.iter().any(|m| m == maj_min)
                                || source.starts_with(&format!("/dev/{}", base_name))
                                || mp == "/mnt"
                                || mp.starts_with("/mnt/");
                            // Ne jamais démonter les montages vitaux du Live ISO s'ils ne sont pas sur /mnt
                            let is_live_vital = mp == "/"
                                || mp == "/iso"
                                || mp == "/nix"
                                || mp.starts_with("/nix/")
                                || mp == "/proc"
                                || mp == "/sys"
                                || mp == "/dev"
                                || mp == "/run";
                            if matches_disk && !is_live_vital {
                                if !mounts_to_umount.contains(&mp.to_string()) {
                                    mounts_to_umount.push(mp.to_string());
                                }
                            } else if matches_disk && is_live_vital {
                                if pid == 1 {
                                    log(
                                        &manager,
                                        &format!(
                                            "[DIAG] Attention : montage système {} détecté sur {} (maj:min={})",
                                            mp, source, maj_min
                                        ),
                                    )
                                    .await;
                                }
                            }
                        }
                    }
                    mounts_to_umount.sort_by(|a, b| b.len().cmp(&a.len()));
                    for mp in mounts_to_umount {
                        if pid == 1 || pid == my_pid {
                            log(&manager, &format!("[DIAG] Démontage de {}...", mp)).await;
                            let _ = Command::new("umount").args(["-R", "-l", &mp]).status().await;
                            let _ = Command::new("umount").args(["-f", &mp]).status().await;
                        } else {
                            let _ = Command::new("nsenter")
                                .args(["-t", &pid.to_string(), "-m", "umount", "-R", "-l", &mp])
                                .status()
                                .await;
                        }
                    }
                }
            }
        }
    }

    // 1b. Désactivation des swaps (via /proc/swaps et appel système swapoff direct)
    if let Ok(swaps_content) = std::fs::read_to_string("/proc/swaps") {
        for line in swaps_content.lines().skip(1) {
            if let Some(dev) = line.split_whitespace().next() {
                if dev.starts_with(&format!("/dev/{}", base_name)) {
                    log(&manager, &format!("[DIAG] Désactivation du swap sur {}...", dev)).await;
                    let _ = Command::new("swapoff").arg(dev).status().await;
                    if let Ok(c_dev) = std::ffi::CString::new(dev) {
                        unsafe {
                            let _ = swapoff(c_dev.as_ptr());
                        }
                    }
                }
            }
        }
    }
    for p in &initial_parts {
        let dev_str = format!("/dev/{}", p.name);
        if let Ok(c_dev) = std::ffi::CString::new(dev_str) {
            unsafe {
                let _ = swapoff(c_dev.as_ptr());
            }
        }
    }

    // 1c. Détachement des périphériques loop pointant vers le disque cible (sans toucher à /iso)
    if let Ok(block_entries) = std::fs::read_dir("/sys/class/block") {
        for entry in block_entries.flatten() {
            let l_name = entry.file_name().to_string_lossy().to_string();
            if l_name.starts_with("loop") {
                let backing_path = format!("/sys/class/block/{}/loop/backing_file", l_name);
                if let Ok(backing) = std::fs::read_to_string(&backing_path) {
                    let b_trim = backing.trim();
                    if (b_trim.starts_with(&format!("/dev/{}", base_name)) || b_trim.starts_with("/mnt"))
                        && !b_trim.starts_with("/iso")
                    {
                        let loop_dev = format!("/dev/{}", l_name);
                        log(&manager, &format!("[DIAG] Détachement de {} ({})...", loop_dev, b_trim)).await;
                        let _ = Command::new("umount").args(["-R", "-l", &loop_dev]).status().await;
                        if let Ok(lf) = std::fs::OpenOptions::new().read(true).open(&loop_dev) {
                            unsafe {
                                let _ = ioctl(lf.as_raw_fd(), LOOP_CLR_FD, 0);
                            }
                        }
                        let _ = Command::new("losetup").args(["-d", &loop_dev]).status().await;
                    }
                }
            }
        }
    }

    // 1d. Libération des pools ZFS, volumes LVM, conteneurs DM/LUKS et grappes RAID mdadm
    let _ = Command::new("zpool").args(["export", "-f", "-a"]).status().await;
    let _ = Command::new("vgchange").args(["-an"]).status().await;

    fn collect_holders_depth_first(blk_name: &str, out: &mut Vec<String>) {
        let holders_dir = format!("/sys/class/block/{}/holders", blk_name);
        if let Ok(entries) = std::fs::read_dir(&holders_dir) {
            for entry in entries.flatten() {
                let h_name = entry.file_name().to_string_lossy().to_string();
                collect_holders_depth_first(&h_name, out);
                if !out.contains(&h_name) {
                    out.push(h_name);
                }
            }
        }
    }

    let mut all_holders = Vec::new();
    for p in &initial_parts {
        collect_holders_depth_first(&p.name, &mut all_holders);
    }
    collect_holders_depth_first(&base_name, &mut all_holders);

    for h_name in &all_holders {
        let h_dev = format!("/dev/{}", h_name);
        log(&manager, &format!("[DIAG] Libération du holder noyau {}...", h_dev)).await;
        let _ = Command::new("umount").args(["-R", "-l", &h_dev]).status().await;
        let _ = Command::new("swapoff").arg(&h_dev).status().await;
        if h_name.starts_with("dm-") {
            let _ = Command::new("dmsetup").args(["remove", "-f", "--retry", h_name]).status().await;
        } else if h_name.starts_with("md") {
            if let Ok(mdf) = std::fs::OpenOptions::new().read(true).open(&h_dev) {
                unsafe {
                    let _ = ioctl(mdf.as_raw_fd(), STOP_ARRAY, 0);
                }
            }
            let _ = std::fs::write(format!("/sys/class/block/{}/md/array_state", h_name), "inactive\n");
            let _ = std::fs::write(format!("/sys/class/block/{}/md/array_state", h_name), "clear\n");
            let _ = Command::new("mdadm").args(["--stop", &h_dev]).status().await;
        }
    }

    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

    // 1e. Terminer tout processus tiers maintenant un descripteur ouvert sur /dev/<base_name>*
    if let Ok(proc_entries) = std::fs::read_dir("/proc") {
        for proc_entry in proc_entries.flatten() {
            if let Ok(pid) = proc_entry.file_name().to_string_lossy().parse::<u32>() {
                if pid <= 1 || pid == my_pid {
                    continue;
                }
                let fd_dir = format!("/proc/{}/fd", pid);
                if let Ok(fds) = std::fs::read_dir(&fd_dir) {
                    let mut held_target = None;
                    for fd in fds.flatten() {
                        if let Ok(target) = std::fs::read_link(fd.path()) {
                            let target_str = target.to_string_lossy().to_string();
                            if target_str.starts_with(&format!("/dev/{}", base_name)) {
                                held_target = Some(target_str);
                                break;
                            }
                        }
                    }
                    if let Some(target_str) = held_target {
                        let comm = std::fs::read_to_string(format!("/proc/{}/comm", pid))
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        let status_str = std::fs::read_to_string(format!("/proc/{}/status", pid))
                            .unwrap_or_default();
                        let is_root_udevd = comm.contains("udevd")
                            && status_str.lines().any(|l| l.starts_with("PPid:\t1"));
                        log(
                            &manager,
                            &format!(
                                "[DIAG] Processus {} (PID {}) détient {} ouvert.",
                                comm, pid, target_str
                            ),
                        )
                        .await;
                        if !is_root_udevd && !comm.contains("noos") {
                            let _ = Command::new("kill").args(["-9", &pid.to_string()]).status().await;
                        }
                    }
                }
            }
        }
    }

    // 1f. Effacement des signatures des anciennes partitions
    for p in &initial_parts {
        let part_dev = format!("/dev/{}", p.name);
        let _ = Command::new("wipefs").args(["-a", "-f", &part_dev]).status().await;
    }
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

    // Suspension temporaire de l'exécution des workers udev pendant le repartitionnement
    // afin d'éviter qu'un worker udev (blkid/btrfs/mdadm) ne verrouille /dev/sda1 pendant BLKRRPART/BLKPG
    let _ = Command::new("udevadm").args(["control", "--stop-exec-queue"]).status().await;

    // Suppression explicite des anciennes partitions dans la table du noyau Linux (via ioctl BLKPG_DEL_PARTITION direct)
    for p in &initial_parts {
        let _ = blkpg_op(&req.disk_path, BLKPG_DEL_PARTITION, p.pno, 0, 0);
    }
    for pno in 1..=128 {
        let _ = blkpg_op(&req.disk_path, BLKPG_DEL_PARTITION, pno, 0, 0);
    }
    let _ = blkrrpart_ioctl(&req.disk_path);

    log(&manager, &format!("Effacement des anciennes signatures sur {}", req.disk_path)).await;
    let mut cmd = Command::new("wipefs");
    cmd.args(["-a", "-f", &req.disk_path]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        log(&manager, &format!("Avertissement wipefs : {}", e)).await;
    }

    let mut zap_cmd = Command::new("sgdisk");
    zap_cmd.args(["-Z", &req.disk_path]);
    if let Err(e) = exec_cmd(manager.clone(), zap_cmd).await {
        log(&manager, &format!("Avertissement sgdisk -Z : {}", e)).await;
    }

    // Nouvelle tentative de suppression noyau après wipefs/zap
    for pno in 1..=128 {
        let _ = blkpg_op(&req.disk_path, BLKPG_DEL_PARTITION, pno, 0, 0);
    }
    let _ = blkrrpart_ioctl(&req.disk_path);

    // Vérifier s'il reste des partitions récalcitrantes dans /sys/class/block/<base_name>*
    let surviving = list_kernel_partitions(&base_name);
    let boot_size_sectors: u64 = 2_097_152; // 1024 MiB en secteurs de 512 octets
    let usable_end: u64 = if total_sectors > 4096 {
        ((total_sectors - 2048) / 2048) * 2048
    } else {
        0
    };

    if usable_end <= boot_size_sectors + 4096 {
        let _ = Command::new("udevadm").args(["control", "--start-exec-queue"]).status().await;
        fail_install(
            manager,
            format!("Capacité du disque {} insuffisante ({} secteurs).", req.disk_path, total_sectors),
        )
        .await;
        return;
    }

    let mut boot_pno: i32 = 1;
    let mut boot_start: u64 = 2048;
    let boot_sectors: u64 = boot_size_sectors;
    let mut root_pno: i32 = 2;
    let mut root_start: u64 = 2048 + boot_size_sectors;
    let mut root_sectors: u64 = usable_end - root_start;

    if !surviving.is_empty() {
        for sp in &surviving {
            log(
                &manager,
                &format!(
                    "[DIAG] Partition maintenue par le noyau : /dev/{} (pno={}, start={}, size={}) -> réduction à chaud via BLKPG_RESIZE_PARTITION...",
                    sp.name, sp.pno, sp.start_sect, sp.size_sect
                ),
            )
            .await;
            // Passe 1 : Réduire toute partition survivante à 8 secteurs (4 Kio) à son offset de départ
            let min_len = sp.size_sect.min(8).max(1);
            if let Err(e) = blkpg_op(&req.disk_path, BLKPG_RESIZE_PARTITION, sp.pno, sp.start_sect, min_len) {
                log(
                    &manager,
                    &format!(
                        "[DIAG] Avertissement BLKPG_RESIZE_PARTITION sur pno={} : {}",
                        sp.pno, e
                    ),
                )
                .await;
            }
        }

        // Relire l'état réel des partitions survivantes dans /sys/class/block après réduction
        let surviving = list_kernel_partitions(&base_name);

        // Peut-on réutiliser pno=1 directement comme partition BOOT de 1024 Mio ?
        let p1_reusable = surviving.iter().find(|sp| sp.pno == 1).and_then(|sp1| {
            if sp1.start_sect >= 34
                && sp1.start_sect <= 4_194_304
                && (sp1.start_sect % sectors_per_lba == 0)
                && surviving.iter().all(|other| {
                    other.pno == 1
                        || other.start_sect >= sp1.start_sect + boot_size_sectors
                        || other.start_sect + other.size_sect <= sp1.start_sect
                })
            {
                Some(sp1.start_sect)
            } else {
                None
            }
        });

        let align_up_2048 = |v: u64| -> u64 { ((v + 2047) / 2048) * 2048 };
        let align_down_2048 = |v: u64| -> u64 { (v / 2048) * 2048 };

        if let Some(s1) = p1_reusable {
            boot_pno = 1;
            boot_start = s1;
            let _ = blkpg_op(&req.disk_path, BLKPG_RESIZE_PARTITION, 1, boot_start, boot_sectors);

            // Trouver le plus grand intervalle libre après boot_start + boot_sectors évitant les autres partitions survivantes
            let search_start = align_up_2048(boot_start + boot_sectors);
            let mut obstacles: Vec<(u64, u64)> = surviving
                .iter()
                .filter(|sp| sp.pno != 1 && sp.start_sect + sp.size_sect > search_start && sp.start_sect < usable_end)
                .map(|sp| (sp.start_sect, sp.start_sect + sp.size_sect))
                .collect();
            obstacles.sort_by_key(|o| o.0);

            let mut best_start = search_start;
            let mut best_end = usable_end;
            let mut best_len = 0u64;
            let mut cur = search_start;
            for (obs_s, obs_e) in &obstacles {
                let seg_e = align_down_2048(*obs_s);
                if seg_e > cur && seg_e - cur > best_len {
                    best_start = cur;
                    best_end = seg_e;
                    best_len = seg_e - cur;
                }
                cur = align_up_2048(*obs_e);
            }
            if usable_end > cur && usable_end - cur > best_len {
                best_start = cur;
                best_end = usable_end;
            }

            root_start = best_start;
            root_sectors = best_end.saturating_sub(best_start);
            root_pno = (2..=128)
                .find(|cand| !surviving.iter().any(|sp| sp.pno == *cand))
                .unwrap_or(2);
        } else {
            // Trouver le plus grand intervalle libre dans [2048, usable_end] évitant toutes les partitions survivantes
            let mut obstacles: Vec<(u64, u64)> = surviving
                .iter()
                .filter(|sp| sp.start_sect + sp.size_sect > 2048 && sp.start_sect < usable_end)
                .map(|sp| (sp.start_sect, sp.start_sect + sp.size_sect))
                .collect();
            obstacles.sort_by_key(|o| o.0);

            let mut best_start = 2048u64;
            let mut best_end = usable_end;
            let mut best_len = 0u64;
            let mut cur = 2048u64;
            for (obs_s, obs_e) in &obstacles {
                let seg_e = align_down_2048(*obs_s);
                if seg_e > cur && seg_e - cur > best_len {
                    best_start = cur;
                    best_end = seg_e;
                    best_len = seg_e - cur;
                }
                cur = align_up_2048(*obs_e);
            }
            if usable_end > cur && usable_end - cur > best_len {
                best_start = cur;
                best_end = usable_end;
            }

            boot_start = best_start;
            root_start = best_start + boot_sectors;
            root_sectors = best_end.saturating_sub(root_start);
            boot_pno = (1..=128)
                .find(|cand| !surviving.iter().any(|sp| sp.pno == *cand))
                .unwrap_or(1);
            root_pno = ((boot_pno + 1)..=128)
                .find(|cand| !surviving.iter().any(|sp| sp.pno == *cand))
                .unwrap_or(2);
        }
    }

    let boot_lba_start = boot_start / sectors_per_lba;
    let boot_lba_end = (boot_start + boot_sectors) / sectors_per_lba - 1;
    let root_lba_start = root_start / sectors_per_lba;
    let root_lba_end = (root_start + root_sectors) / sectors_per_lba - 1;

    log(
        &manager,
        &format!(
            "Création de la table de partition GPT sur {} (Boot: pno={} LBA {}..{}, Root: pno={} LBA {}..{})",
            req.disk_path, boot_pno, boot_lba_start, boot_lba_end, root_pno, root_lba_start, root_lba_end
        ),
    )
    .await;

    let mut cmd = Command::new("sgdisk");
    cmd.args([
        "-o",
        "-a",
        "1",
        "-n",
        &format!("{}:{}:{}", boot_pno, boot_lba_start, boot_lba_end),
        "-t",
        &format!("{}:ef00", boot_pno),
        "-c",
        &format!("{}:BOOT", boot_pno),
        "-n",
        &format!("{}:{}:{}", root_pno, root_lba_start, root_lba_end),
        "-t",
        &format!("{}:8300", root_pno),
        "-c",
        &format!("{}:ROOT", root_pno),
        &req.disk_path,
    ]);
    if let Err(e) = exec_cmd(manager.clone(), cmd).await {
        let _ = Command::new("udevadm").args(["control", "--start-exec-queue"]).status().await;
        fail_install(manager, e).await;
        return;
    }

    // Détermination des chemins et noms courts de partition
    let part_prefix = if req.disk_path.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("{}p", req.disk_path)
    } else {
        req.disk_path.clone()
    };
    let boot_part = format!("{}{}", part_prefix, boot_pno);
    let root_part = format!("{}{}", part_prefix, root_pno);
    let boot_name = boot_part.trim_start_matches("/dev/").to_string();
    let root_name = root_part.trim_start_matches("/dev/").to_string();

    log(&manager, "Actualisation des partitions auprès du noyau Linux...").await;
    let _ = Command::new("sync").status().await;

    // Enregistrement direct des partitions dans le noyau via ioctl BLKRRPART + BLKPG
    let _ = blkrrpart_ioctl(&req.disk_path);

    for (pno, p_name, p_start, p_sectors) in [
        (boot_pno, boot_name.as_str(), boot_start, boot_sectors),
        (root_pno, root_name.as_str(), root_start, root_sectors),
    ] {
        let sys_path = format!("/sys/class/block/{}", p_name);
        if std::path::Path::new(&sys_path).exists() {
            match blkpg_op(&req.disk_path, BLKPG_RESIZE_PARTITION, pno, p_start, p_sectors) {
                Ok(()) => {
                    log(
                        &manager,
                        &format!("[DIAG] BLKPG_RESIZE_PARTITION réussi pour /dev/{} (start={}, sectors={})", p_name, p_start, p_sectors),
                    )
                    .await;
                }
                Err(e) => {
                    log(
                        &manager,
                        &format!("[DIAG] BLKPG_RESIZE_PARTITION /dev/{} : {}", p_name, e),
                    )
                    .await;
                }
            }
        } else {
            match blkpg_op(&req.disk_path, BLKPG_ADD_PARTITION, pno, p_start, p_sectors) {
                Ok(()) => {
                    log(
                        &manager,
                        &format!("[DIAG] BLKPG_ADD_PARTITION réussi pour /dev/{} (start={}, sectors={})", p_name, p_start, p_sectors),
                    )
                    .await;
                }
                Err(e) => {
                    log(
                        &manager,
                        &format!("[DIAG] Erreur BLKPG_ADD_PARTITION /dev/{} : {}", p_name, e),
                    )
                    .await;
                }
            }
        }
    }

    // Réactiver la file d'exécution udev maintenant que la table de partitions du noyau est à jour
    let _ = Command::new("udevadm").args(["control", "--start-exec-queue"]).status().await;

    // Boucle de vérification et création de noeuds bloc (/dev/<part>)
    let mut partitions_ready = false;
    for attempt in 1..=15 {
        let _ = Command::new("udevadm")
            .args(["trigger", "--subsystem-match=block", "--action=add"])
            .status()
            .await;
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        for (pno, p_name, p_start, p_sectors) in [
            (boot_pno, boot_name.as_str(), boot_start, boot_sectors),
            (root_pno, root_name.as_str(), root_start, root_sectors),
        ] {
            let sys_dev = format!("/sys/class/block/{}/dev", p_name);
            let dev_node = format!("/dev/{}", p_name);
            if !std::path::Path::new(&sys_dev).exists() {
                let _ = blkpg_op(&req.disk_path, BLKPG_ADD_PARTITION, pno, p_start, p_sectors);
            }
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

        if std::path::Path::new(&boot_part).exists() && std::path::Path::new(&root_part).exists() {
            partitions_ready = true;
            break;
        }
        log(
            &manager,
            &format!(
                "Attente de la création des périphériques {} (existe={}) et {} (existe={}) par le noyau (tentative {}/15)...",
                boot_part,
                std::path::Path::new(&boot_part).exists(),
                root_part,
                std::path::Path::new(&root_part).exists(),
                attempt
            ),
        )
        .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    if !partitions_ready {
        fail_install(
            manager,
            format!(
                "Les partitions {} et {} n'ont pas pu être créées par le noyau Linux.",
                boot_part, root_part
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
