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
pub struct KernelPartInfo {
    pub name: String,
    pub pno: i32,
    pub start_sect: u64,
    pub size_sect: u64,
    pub dev_maj_min: String,
    pub holders: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionLayout {
    pub boot_pno: i32,
    pub boot_start: u64,
    pub boot_sectors: u64,
    pub boot_lba_start: u64,
    pub boot_lba_end: u64,
    pub root_pno: i32,
    pub root_start: u64,
    pub root_sectors: u64,
    pub root_lba_start: u64,
    pub root_lba_end: u64,
    pub reused_p1: bool,
}

#[derive(Debug, Clone)]
pub struct ErrorDiagnosis {
    pub summary: String,
    pub explanation: String,
    pub suggestion: String,
    pub technical_detail: String,
}

/// Échappe une chaîne pour l'insérer en toute sécurité entre guillemets dans un fichier Nix
pub fn escape_nix_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            out.push_str("\\\\");
        } else if c == '"' {
            out.push_str("\\\"");
        } else if c == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            out.push_str("\\${");
            i += 1;
        } else if c == '\n' {
            out.push_str("\\n");
        } else if c == '\r' {
            out.push_str("\\r");
        } else if c == '\t' {
            out.push_str("\\t");
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Valide un nom d'hôte RFC 1123 (1 à 63 caractères, alphanumérique + tirets, sans tiret au début/fin)
pub fn validate_hostname(hostname: &str) -> bool {
    if hostname.is_empty() || hostname.len() > 63 {
        return false;
    }
    if hostname.starts_with('-') || hostname.ends_with('-') {
        return false;
    }
    hostname
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
}

/// Calcule l'emplacement optimal et sans chevauchement des partitions BOOT et ROOT.
/// Garantit que la partition BOOT (pno=1, 1024 Mio) démarre toujours au secteur 2048 (1 Mio)
/// et que la partition ROOT (pno=2) occupe immédiatement tout le reste du disque jusqu'à la fin
/// alignée, sans jamais laisser d'espace non alloué en début ou milieu de disque.
pub fn compute_partition_layout(
    total_sectors: u64,
    logical_block_size: u64,
    surviving: &[KernelPartInfo],
) -> Result<PartitionLayout, String> {
    let sectors_per_lba: u64 = (logical_block_size.max(512) / 512).max(1);
    let boot_size_sectors: u64 = 2_097_152; // 1024 Mio en secteurs de 512 octets
    let min_root_sectors: u64 = 12_582_912; // 6 Gio minimum pour la racine NixOS

    let usable_end: u64 = if total_sectors > 4096 {
        ((total_sectors - 2048) / 2048) * 2048
    } else {
        0
    };

    if usable_end <= boot_size_sectors + min_root_sectors + 2048 {
        let gib = (total_sectors as f64 * 512.0) / 1_073_741_824.0;
        return Err(format!(
            "Capacité du disque insuffisante ({:.2} Gio détectés, minimum 8 Gio requis pour Noos NAS).",
            gib
        ));
    }

    let boot_pno: i32 = 1;
    let boot_start: u64 = 2048;
    let boot_sectors: u64 = boot_size_sectors;
    let root_pno: i32 = 2;
    let root_start: u64 = boot_start + boot_sectors;
    let root_sectors: u64 = usable_end.saturating_sub(root_start);
    let reused_p1: bool = surviving
        .iter()
        .any(|sp| sp.pno == 1 && sp.start_sect == boot_start);

    if root_sectors < min_root_sectors {
        return Err(format!(
            "Espace contigu insuffisant pour la partition racine ({} secteurs disponibles, {} requis).",
            root_sectors, min_root_sectors
        ));
    }

    let boot_lba_start = boot_start / sectors_per_lba;
    let boot_lba_end = (boot_start + boot_sectors) / sectors_per_lba - 1;
    let root_lba_start = root_start / sectors_per_lba;
    let root_lba_end = (root_start + root_sectors) / sectors_per_lba - 1;

    Ok(PartitionLayout {
        boot_pno,
        boot_start,
        boot_sectors,
        boot_lba_start,
        boot_lba_end,
        root_pno,
        root_start,
        root_sectors,
        root_lba_start,
        root_lba_end,
        reused_p1,
    })
}

/// Injecte proprement un attribut Nix (ex: `boot.loader.efi.canTouchEfiVariables = lib.mkForce false;`)
/// juste avant l'accolade fermante finale d'un module Nix (`hardware-configuration.nix`) généré par l'hôte,
/// sans écraser les définitions de points de montage (`fileSystems`) ni les modules noyau (`initrd`).
pub fn inject_nix_module_attr(nix_module: &str, attr_line: &str) -> String {
    let trimmed_attr = attr_line.trim();
    if nix_module.contains(trimmed_attr) {
        return nix_module.to_string();
    }
    if let Some(last_brace) = nix_module.rfind('}') {
        let mut out = String::with_capacity(nix_module.len() + trimmed_attr.len() + 8);
        out.push_str(&nix_module[..last_brace]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("  ");
        out.push_str(trimmed_attr);
        out.push('\n');
        out.push_str(&nix_module[last_brace..]);
        out
    } else {
        format!("{}\n{}\n", nix_module, trimmed_attr)
    }
}

/// Extrait un résumé lisible des paramètres matériels détectés par `nixos-generate-config` sur l'hôte
pub fn summarize_generated_hardware_config(hw_nix: &str) -> String {
    let mut initrd_mods = String::from("standard");
    let mut cpu_microcode = String::from("générique");
    let mut fs_count = 0usize;

    for line in hw_nix.lines() {
        let t = line.trim();
        if t.starts_with("boot.initrd.availableKernelModules") {
            if let (Some(l), Some(r)) = (t.find('['), t.rfind(']')) {
                let mods = t[l + 1..r].trim();
                if !mods.is_empty() {
                    initrd_mods = mods.replace('"', "");
                }
            }
        } else if t.contains("hardware.cpu.intel.updateMicrocode") {
            cpu_microcode = "Intel".to_string();
        } else if t.contains("hardware.cpu.amd.updateMicrocode") {
            cpu_microcode = "AMD".to_string();
        } else if t.starts_with("fileSystems.\"") {
            fs_count += 1;
        }
    }

    format!(
        "CPU={} | Modules initrd=[{}] | Points de montage déclarés={}",
        cpu_microcode, initrd_mods, fs_count
    )
}

/// Traduit et verbalise clairement en français une erreur d'installation avec sa cause et sa solution
pub fn verbalize_error(
    step_label: &str,
    cmd_context: &str,
    raw_err: &str,
    recent_stderr: &[String],
) -> ErrorDiagnosis {
    let combined = format!(
        "{} {} {}",
        cmd_context,
        raw_err,
        recent_stderr.join(" | ")
    )
    .to_lowercase();

    let (summary, explanation, suggestion) = if (combined.contains("no such file or directory")
        || combined.contains("n'existe pas"))
        && (combined.contains("/dev/") || combined.contains("stat") || combined.contains("fichier"))
    {
        (
            format!("Échec à {} : Périphérique de partition introuvable dans /dev", step_label),
            "Le noyau Linux n'a pas exposé le nœud de partition attendu dans /dev avant l'opération de formatage ou de montage.".to_string(),
            "Vérifiez qu'aucun câble SATA/USB/NVMe ne s'est déconnecté, ou redémarrez l'ISO Live pour réinitialiser la table du noyau.".to_string(),
        )
    } else if combined.contains("device or resource busy") || combined.contains("périphérique ou ressource occupé") || combined.contains("is apparently in use by the system") {
        (
            format!("Échec à {} : Le disque ou la partition cible est verrouillé par le système", step_label),
            "Un processus système, un pool ZFS/RAID ou un montage actif maintient un verrou exclusif sur le disque cible.".to_string(),
            "Assurez-vous de ne pas sélectionner le support sur lequel l'ISO Live a démarré, puis relancez l'installation.".to_string(),
        )
    } else if combined.contains("could not resolve host")
        || combined.contains("network is unreachable")
        || combined.contains("connection timed out")
        || combined.contains("failed to connect to github.com")
        || combined.contains("unable to access 'https://github.com")
    {
        (
            format!("Échec à {} : Connexion réseau ou accès à GitHub indisponible", step_label),
            "L'installateur n'a pas pu joindre les serveurs distants (GitHub ou cache binaire NixOS) pour télécharger les fichiers requis.".to_string(),
            "Vérifiez que le câble Ethernet est bien branché, que votre routeur attribue une adresse IP par DHCP et que l'accès Internet fonctionne.".to_string(),
        )
    } else if combined.contains("no space left on device") || combined.contains("aucun espace disponible sur le périphérique") {
        (
            format!("Échec à {} : Espace disque ou mémoire saturé", step_label),
            "La partition cible (ou le système de fichiers temporaire en RAM) n'a plus assez d'espace libre pour poursuivre l'installation.".to_string(),
            "Utilisez un disque d'au moins 16 Go et vérifiez que la machine dispose d'au moins 2 Go de mémoire vive (RAM).".to_string(),
        )
    } else if combined.contains("input/output error") || combined.contains("erreur d'entrée/sortie") || combined.contains("i/o error") {
        (
            format!("Échec à {} : Erreur matérielle d'entrée/sortie (I/O) sur le disque", step_label),
            "Le contrôleur de stockage a renvoyé une erreur physique lors de la lecture ou de l'écriture sur le disque cible.".to_string(),
            "Vérifiez l'état de santé S.M.A.R.T. du disque, le câble SATA/alimentation ou changez de port sur la carte mère.".to_string(),
        )
    } else if combined.contains("bootctl")
        || combined.contains("efivarfs")
        || combined.contains("efi variables")
        || combined.contains("failed to write efi")
    {
        (
            format!("Échec à {} : Impossible d'enregistrer l'entrée de démarrage UEFI", step_label),
            "Le firmware de la carte mère (BIOS Legacy/CSM ou NVRAM UEFI verrouillée) a refusé l'écriture des variables EFI.".to_string(),
            "Activez le mode UEFI dans le BIOS de votre machine et désactivez Secure Boot, puis réessayez.".to_string(),
        )
    } else if combined.contains("nixos-install") || combined.contains("flake") || combined.contains("evaluation warning") {
        (
            format!("Échec à {} : Erreur lors du déploiement du système NixOS", step_label),
            "La commande nixos-install a rencontré une erreur lors de l'évaluation du Flake ou du téléchargement des paquets du store Nix.".to_string(),
            "Consultez les dernières lignes [STDERR] ci-dessous pour identifier le paquet ou l'option Nix en cause, puis relancez l'installation.".to_string(),
        )
    } else {
        (
            format!("Échec à {} ({})", step_label, cmd_context),
            format!("Une commande système s'est terminée de manière inattendue : {}", raw_err),
            "Consultez les détails techniques ci-dessous. Vous pouvez modifier la configuration et relancer l'installation sans redémarrer.".to_string(),
        )
    };

    let last_stderr = if recent_stderr.is_empty() {
        raw_err.to_string()
    } else {
        let start = recent_stderr.len().saturating_sub(4);
        recent_stderr[start..].join(" | ")
    };

    ErrorDiagnosis {
        summary,
        explanation,
        suggestion,
        technical_detail: format!("Contexte : {} — {}", cmd_context, last_stderr),
    }
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

async fn log_stage_banner(
    mgr: &Arc<Mutex<InstallManager>>,
    stage_num: u32,
    total_stages: u32,
    title: &str,
    objective: &str,
    progress: u32,
) {
    let mut m = mgr.lock().await;
    let step_title = format!("Étape {}/{} : {}", stage_num, total_stages, title);
    m.step = step_title.clone();
    m.progress = progress;
    m.add_log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
    m.add_log(format!(
        "🔷 [ÉTAPE {}/{}] {}",
        stage_num,
        total_stages,
        title.to_uppercase()
    ));
    m.add_log(format!("   Objectif : {}", objective));
    m.add_log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
    m.add_log(format!("[PROGRESS] {}% - {}", progress, step_title));
}

async fn log_stage_success(
    mgr: &Arc<Mutex<InstallManager>>,
    stage_num: u32,
    total_stages: u32,
    summary: &str,
) {
    log(
        mgr,
        &format!("✅ [ÉTAPE {}/{} VALIDÉE] {}", stage_num, total_stages, summary),
    )
    .await;
}

async fn log_safety_check(mgr: &Arc<Mutex<InstallManager>>, msg: &str) {
    log(mgr, &format!("🛡️ [SÉCURITÉ] {}", msg)).await;
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
        m.progress = 2;
        m.step = "Étape 1/6 : Vérifications pré-vol et sécurité".to_string();
        m.error = None;
        m.logs.clear();
    }

    // Laisser 250ms au navigateur pour ouvrir le flux EventSource (/api/install/stream)
    tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;

    log(&manager, "🚀 Démarrage de l'installation de Noos NAS Edition (v0.2.5)...").await;
    log(
        &manager,
        &format!(
            "📋 Récapitulatif : Disque={} | Système de fichiers={} | Administrateur={} | Nom d'hôte={}",
            req.disk_path,
            req.filesystem.to_uppercase(),
            req.username,
            req.hostname
        ),
    )
    .await;

    // Helper d'exécution avec capture des dernières lignes STDERR et verbalisation d'erreur
    async fn exec_cmd_named(
        mgr: Arc<Mutex<InstallManager>>,
        step_label: &str,
        cmd_desc: &str,
        mut cmd: Command,
    ) -> Result<(), ErrorDiagnosis> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let raw = format!("Impossible de lancer la commande ({}) : {}", cmd_desc, e);
                return Err(verbalize_error(step_label, cmd_desc, &raw, &[]));
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
        let stderr_lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let stderr_lines_clone = stderr_lines.clone();
        let err_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                {
                    let mut s = stderr_lines_clone.lock().await;
                    s.push(line.clone());
                }
                let mut m = mgr_err.lock().await;
                m.add_log(format!("[STDERR] {}", line));
            }
        });

        let (status, _, _) = tokio::join!(child.wait(), out_task, err_task);
        let collected_stderr = stderr_lines.lock().await.clone();
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => {
                let raw = format!(
                    "Commande '{}' terminée avec le code d'erreur {}",
                    cmd_desc,
                    s.code().unwrap_or(-1)
                );
                Err(verbalize_error(step_label, cmd_desc, &raw, &collected_stderr))
            }
            Err(e) => {
                let raw = format!("Erreur lors de l'attente de '{}' : {}", cmd_desc, e);
                Err(verbalize_error(step_label, cmd_desc, &raw, &collected_stderr))
            }
        }
    }

    // Helper spécialisé pour nixos-install avec calcul dynamique de la progression (22% -> 96%)
    async fn exec_nixos_install(
        mgr: Arc<Mutex<InstallManager>>,
        mut cmd: Command,
    ) -> Result<(), (String, Vec<String>)> {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("Impossible de lancer nixos-install : {}", e);
                log(&mgr, &err_msg).await;
                return Err((err_msg, Vec::new()));
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
            current_progress: 22,
        }));
        let recent_stderr = Arc::new(Mutex::new(Vec::<String>::new()));

        async fn handle_line(
            mgr: &Arc<Mutex<InstallManager>>,
            state: &Arc<Mutex<NixProgressState>>,
            recent_stderr: &Arc<Mutex<Vec<String>>>,
            raw_line: &str,
            is_stderr: bool,
        ) {
            let line = raw_line.trim();
            if is_stderr && !line.is_empty() {
                let mut err_buf = recent_stderr.lock().await;
                err_buf.push(line.to_string());
                if err_buf.len() > 25 {
                    err_buf.remove(0);
                }
            }

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
                let target_prog = (22.0 + ratio * 72.0).round() as u32;
                let target_prog = target_prog.clamp(22, 94);

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
                    "Étape 5/6 : {} de {} ({}/{})",
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
                let step_desc = "Étape 5/6 : Configuration du système et des services (/etc)";
                let mut m = mgr.lock().await;
                m.progress = 95;
                m.step = step_desc.to_string();
                m.add_log(format!("[PROGRESS] 95% - {}", step_desc));
            } else if line.contains("installing the boot loader") || line.contains("installing boot loader") {
                let mut s = state.lock().await;
                s.current_progress = 96;
                let step_desc = "Étape 5/6 : Installation du chargeur d'amorçage (systemd-boot)";
                let mut m = mgr.lock().await;
                m.progress = 96;
                m.step = step_desc.to_string();
                m.add_log(format!("[PROGRESS] 96% - {}", step_desc));
            } else if line.contains("installation finished") {
                let mut s = state.lock().await;
                s.current_progress = 97;
                let step_desc = "Étape 5/6 : Déploiement NixOS terminé";
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
        let err_out = recent_stderr.clone();
        let out_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                handle_line(&mgr_out, &state_out, &err_out, &line, false).await;
            }
        });

        let mgr_err = mgr.clone();
        let state_err = state.clone();
        let err_err = recent_stderr.clone();
        let err_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                handle_line(&mgr_err, &state_err, &err_err, &line, true).await;
            }
        });

        let (status, _, _) = tokio::join!(child.wait(), out_task, err_task);
        let collected = recent_stderr.lock().await.clone();
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err((
                format!(
                    "nixos-install s'est terminé avec le code d'erreur {}",
                    s.code().unwrap_or(-1)
                ),
                collected,
            )),
            Err(e) => Err((format!("Erreur d'attente de nixos-install : {}", e), collected)),
        }
    }

    // =========================================================================
    // ÉTAPE 1/6 : VÉRIFICATIONS PRÉ-VOL ET SÉCURITÉ (PRE-FLIGHT CHECKS)
    // =========================================================================
    log_stage_banner(
        &manager,
        1,
        6,
        "Vérifications pré-vol et sécurité",
        "Valider les paramètres, l'intégrité du disque cible, les outils système et le réseau avant toute modification du disque.",
        3,
    )
    .await;

    // 1.1 Validation des paramètres utilisateur
    if req.filesystem != "btrfs" && req.filesystem != "ext4" {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 1/6 : Système de fichiers invalide".to_string(),
                explanation: format!(
                    "Le système de fichiers '{}' n'est pas pris en charge par Noos NAS.",
                    req.filesystem
                ),
                suggestion: "Sélectionnez 'btrfs' (recommandé pour les snapshots NAS) ou 'ext4'.".to_string(),
                technical_detail: format!("filesystem={}", req.filesystem),
            },
        )
        .await;
        return;
    }

    if !validate_hostname(&req.hostname) {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 1/6 : Nom d'hôte (hostname) invalide".to_string(),
                explanation: format!(
                    "Le nom d'hôte '{}' ne respecte pas le standard réseau RFC 1123.",
                    req.hostname
                ),
                suggestion: "Utilisez uniquement des lettres sans accent, des chiffres et des tirets (ex: noos-nas).".to_string(),
                technical_detail: format!("hostname={}", req.hostname),
            },
        )
        .await;
        return;
    }
    log_safety_check(&manager, "Paramètres utilisateur et nom d'hôte validés.").await;

    // 1.2 Vérification du périphérique bloc cible et protection du support Live ISO
    if !req.disk_path.starts_with("/dev/") || req.disk_path.contains("..") {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 1/6 : Chemin de disque cible invalide".to_string(),
                explanation: format!("Le chemin '{}' n'est pas un périphérique /dev/ valide.", req.disk_path),
                suggestion: "Sélectionnez un disque physique valide dans la liste déroulante.".to_string(),
                technical_detail: format!("disk_path={}", req.disk_path),
            },
        )
        .await;
        return;
    }

    if !std::path::Path::new(&req.disk_path).exists() {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 1/6 : Disque cible introuvable".to_string(),
                explanation: format!("Le périphérique {} n'existe pas sur le système.", req.disk_path),
                suggestion: "Actualisez la liste des disques et vérifiez le branchement matériel.".to_string(),
                technical_detail: format!("{} absent de /dev", req.disk_path),
            },
        )
        .await;
        return;
    }

    if crate::disks::is_disk_install_media(&req.disk_path) {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 1/6 : Protection anti-écrasement du support d'installation Live ISO".to_string(),
                explanation: format!(
                    "Le disque {} héberge actuellement le système Live ISO en cours d'exécution (/iso ou /nix/.ro-store).",
                    req.disk_path
                ),
                suggestion: "Choisissez un autre disque de destination pour installer Noos NAS.".to_string(),
                technical_detail: format!("{} est monté comme support Live ISO", req.disk_path),
            },
        )
        .await;
        return;
    }
    log_safety_check(
        &manager,
        &format!(
            "Le disque cible {} est distinct du support d'installation Live ISO.",
            req.disk_path
        ),
    )
    .await;

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
    let disk_gib = (total_sectors as f64 * 512.0) / 1_073_741_824.0;

    if total_sectors < 16_777_216 {
        // Moins de 8 Gio
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 1/6 : Capacité du disque cible insuffisante".to_string(),
                explanation: format!(
                    "Le disque {} ne mesure que {:.2} Gio ({} secteurs), alors qu'un minimum de 8 Gio est requis.",
                    req.disk_path, disk_gib, total_sectors
                ),
                suggestion: "Sélectionnez un disque ou un volume virtuel d'au moins 8 Gio (16 Gio ou plus recommandés).".to_string(),
                technical_detail: format!("total_sectors={}", total_sectors),
            },
        )
        .await;
        return;
    }
    log_safety_check(
        &manager,
        &format!(
            "Capacité matérielle vérifiée : {:.2} Gio ({} secteurs de 512o, bloc logique {}o).",
            disk_gib, total_sectors, logical_block_size
        ),
    )
    .await;

    // 1.3 Vérification de la présence des utilitaires système indispensables
    let required_bins = if req.filesystem == "btrfs" {
        vec![
            "sgdisk",
            "wipefs",
            "mkfs.vfat",
            "mkfs.btrfs",
            "btrfs",
            "mount",
            "umount",
            "nixos-generate-config",
            "nixos-install",
            "git",
            "udevadm",
        ]
    } else {
        vec![
            "sgdisk",
            "wipefs",
            "mkfs.vfat",
            "mkfs.ext4",
            "mount",
            "umount",
            "nixos-generate-config",
            "nixos-install",
            "git",
            "udevadm",
        ]
    };

    for bin in &required_bins {
        let check = Command::new("which").arg(bin).output().await;
        let found = check.map(|o| o.status.success()).unwrap_or(false);
        if !found {
            fail_install_with_diagnosis(
                manager,
                ErrorDiagnosis {
                    summary: format!("Échec à l'Étape 1/6 : Outil système '{}' manquant", bin),
                    explanation: format!(
                        "L'exécutable requis '{}' est introuvable dans le PATH de l'environnement Live.",
                        bin
                    ),
                    suggestion: "Vérifiez que vous utilisez une image ISO Noos NAS complète et non altérée.".to_string(),
                    technical_detail: format!("which {} a échoué (PATH={})", bin, std::env::var("PATH").unwrap_or_default()),
                },
            )
            .await;
            return;
        }
    }
    log_safety_check(&manager, "Tous les outils système requis (sgdisk, mkfs, nixos-install, git, udevadm) sont présents.").await;

    // 1.4 Détection du mode d'amorçage (UEFI vs BIOS Legacy/CSM)
    let is_efi_booted = std::path::Path::new("/sys/firmware/efi").exists();
    if is_efi_booted {
        log_safety_check(&manager, "Mode d'amorçage détecté : UEFI natif (/sys/firmware/efi présent).").await;
    } else {
        log(
            &manager,
            "⚠️ [AVERTISSEMENT] /sys/firmware/efi absent (machine démarrée en mode BIOS Legacy/CSM ou sans variables NVRAM EFI). L'installateur activera automatiquement 'canTouchEfiVariables = false' pour éviter un échec de bootctl.",
        )
        .await;
    }

    // 1.5 Vérification préventive de la source de configuration Noos NAS (locale ou GitHub) AVANT de toucher au disque
    let embedded_source = if std::path::Path::new("/etc/noos-nas-source/flake.nix").exists() {
        Some("/etc/noos-nas-source")
    } else {
        None
    };

    if let Some(src) = embedded_source {
        log_safety_check(
            &manager,
            &format!("Source Noos NAS locale embarquée détectée dans {}.", src),
        )
        .await;
    } else {
        log(
            &manager,
            "Vérification de la connectivité réseau vers GitHub (https://github.com/Chomiam/noos-nas.git) avant formatage...",
        )
        .await;
        let mut net_ok = false;
        let mut last_net_err = String::new();
        for attempt in 1..=3 {
            let check = Command::new("git")
                .args([
                    "ls-remote",
                    "--heads",
                    "https://github.com/Chomiam/noos-nas.git",
                ])
                .output()
                .await;
            match check {
                Ok(o) if o.status.success() => {
                    net_ok = true;
                    break;
                }
                Ok(o) => {
                    last_net_err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                }
                Err(e) => {
                    last_net_err = e.to_string();
                }
            }
            log(
                &manager,
                &format!(
                    "Tentative de connexion à GitHub ({}/3) en attente...",
                    attempt
                ),
            )
            .await;
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        if !net_ok {
            fail_install_with_diagnosis(
                manager,
                verbalize_error(
                    "l'Étape 1/6 (Vérification réseau pré-formatage)",
                    "git ls-remote https://github.com/Chomiam/noos-nas.git",
                    "Impossible de joindre GitHub (aucune modification n'a été apportée à votre disque)",
                    &[last_net_err],
                ),
            )
            .await;
            return;
        }
        log_safety_check(&manager, "Connectivité vers le dépôt GitHub Chomiam/noos-nas vérifiée avec succès.").await;
    }

    log_stage_success(
        &manager,
        1,
        6,
        "Tous les contrôles de sécurité pré-vol sont conformes.",
    )
    .await;

    // =========================================================================
    // ÉTAPE 2/6 : LIBÉRATION ET PARTITIONNEMENT GPT DU DISQUE
    // =========================================================================
    log_stage_banner(
        &manager,
        2,
        6,
        "Libération et partitionnement GPT du disque",
        "Libérer tous les verrous noyau sur le disque cible, créer la table GPT (BOOT EFI 1 Gio + ROOT) et synchroniser les périphériques bloc.",
        5,
    )
    .await;

    let initial_parts = list_kernel_partitions(&base_name);
    if initial_parts.is_empty() {
        log(
            &manager,
            &format!(
                "[DIAG] Disque {} : aucune partition antérieure active en mémoire noyau.",
                req.disk_path
            ),
        )
        .await;
    } else {
        for p in &initial_parts {
            log(
                &manager,
                &format!(
                    "[DIAG] Partition noyau existante détectée : /dev/{} (pno={}, start={}, size={}, dev={}, holders={:?})",
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

    let my_pid = std::process::id();

    // 2.0 Terminer tout processus tiers ayant un fichier ouvert, cwd, root ou mmap dans /mnt ou sur /dev/<base_name>*
    let _ = Command::new("fuser").args(["-k", "-9", "-m", "/mnt"]).status().await;
    if let Ok(proc_entries) = std::fs::read_dir("/proc") {
        for proc_entry in proc_entries.flatten() {
            if let Ok(pid) = proc_entry.file_name().to_string_lossy().parse::<u32>() {
                if pid <= 1 || pid == my_pid {
                    continue;
                }
                let mut holds_mnt_or_disk = false;
                for link_name in ["cwd", "root", "exe"] {
                    if let Ok(target) = std::fs::read_link(format!("/proc/{}/{}", pid, link_name)) {
                        let t = target.to_string_lossy();
                        if t == "/mnt" || t.starts_with("/mnt/") {
                            holds_mnt_or_disk = true;
                            break;
                        }
                    }
                }
                if !holds_mnt_or_disk {
                    if let Ok(fds) = std::fs::read_dir(format!("/proc/{}/fd", pid)) {
                        for fd in fds.flatten() {
                            if let Ok(target) = std::fs::read_link(fd.path()) {
                                let t = target.to_string_lossy();
                                if t == "/mnt"
                                    || t.starts_with("/mnt/")
                                    || t.starts_with(&format!("/dev/{}", base_name))
                                {
                                    holds_mnt_or_disk = true;
                                    break;
                                }
                            }
                        }
                    }
                }
                if holds_mnt_or_disk {
                    let comm = std::fs::read_to_string(format!("/proc/{}/comm", pid))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let status_str = std::fs::read_to_string(format!("/proc/{}/status", pid))
                        .unwrap_or_default();
                    let is_root_udevd = comm.contains("udevd")
                        && status_str.lines().any(|l| l.starts_with("PPid:\t1"));
                    if !is_root_udevd && !comm.contains("noos") {
                        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status().await;
                    }
                }
            }
        }
    }

    let _ = Command::new("sync").status().await;
    // Démontage synchrone d'abord (libère immédiatement le superbloc Btrfs/Ext4), puis repli sur -f / -l
    if Command::new("umount")
        .args(["-R", "/mnt"])
        .status()
        .await
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        let _ = Command::new("umount").args(["-R", "-f", "/mnt"]).status().await;
        let _ = Command::new("umount").args(["-R", "-l", "/mnt"]).status().await;
    }
    let _ = Command::new("swapoff").arg("-a").status().await;

    // 2.1 Démontage dans TOUS les namespaces de montage (/proc/[0-9]*/mountinfo)
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
                            } else if matches_disk && is_live_vital && pid == 1 {
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
                    mounts_to_umount.sort_by(|a, b| b.len().cmp(&a.len()));
                    for mp in mounts_to_umount {
                        if pid == 1 || pid == my_pid {
                            log(&manager, &format!("[DIAG] Démontage de {}...", mp)).await;
                            if Command::new("umount")
                                .args(["-R", &mp])
                                .status()
                                .await
                                .map(|s| !s.success())
                                .unwrap_or(true)
                            {
                                let _ = Command::new("umount").args(["-f", &mp]).status().await;
                                let _ = Command::new("umount").args(["-R", "-l", &mp]).status().await;
                            }
                        } else {
                            let _ = Command::new("nsenter")
                                .args(["-t", &pid.to_string(), "-m", "umount", "-R", &mp])
                                .status()
                                .await;
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

    let _ = Command::new("sync").status().await;

    // 2.2 Désactivation des swaps (via /proc/swaps et appel système swapoff direct)
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

    // 2.3 Détachement des périphériques loop pointant vers le disque cible (sans toucher à /iso)
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
                        let _ = Command::new("umount").args(["-R", &loop_dev]).status().await;
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

    // 2.4 Libération des pools ZFS, volumes LVM, conteneurs DM/LUKS et grappes RAID mdadm
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
        let _ = Command::new("umount").args(["-R", &h_dev]).status().await;
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

    // 2.5 Suspendre la file d'exécution udev AVANT wipefs pour empêcher udevd/blkid de rouvrir les partitions
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;
    let _ = Command::new("udevadm").args(["control", "--stop-exec-queue"]).status().await;

    // 2.6 Effacement des signatures des anciennes partitions et suppression dans la table noyau
    for p in &initial_parts {
        let part_dev = format!("/dev/{}", p.name);
        let _ = Command::new("wipefs").args(["-a", "-f", &part_dev]).status().await;
    }
    let _ = Command::new("sync").status().await;

    log(&manager, &format!("Effacement des anciennes signatures sur {}...", req.disk_path)).await;
    let mut wipe_cmd = Command::new("wipefs");
    wipe_cmd.args(["-a", "-f", &req.disk_path]);
    if let Err(diag) = exec_cmd_named(manager.clone(), "l'Étape 2/6", "wipefs -a -f", wipe_cmd).await {
        log(&manager, &format!("Avertissement wipefs : {}", diag.technical_detail)).await;
    }

    let mut zap_cmd = Command::new("sgdisk");
    zap_cmd.args(["-Z", &req.disk_path]);
    if let Err(diag) = exec_cmd_named(manager.clone(), "l'Étape 2/6", "sgdisk -Z", zap_cmd).await {
        log(&manager, &format!("Avertissement sgdisk -Z : {}", diag.technical_detail)).await;
    }

    // Boucle de purge des anciennes partitions dans le noyau (avec relâchement des descripteurs transitoires)
    for _attempt in 1..=10 {
        let _ = Command::new("sync").status().await;
        let _ = Command::new("partx").args(["-d", &req.disk_path]).status().await;
        for p in &initial_parts {
            let _ = blkpg_op(&req.disk_path, BLKPG_DEL_PARTITION, p.pno, 0, 0);
        }
        for pno in 1..=128 {
            let _ = blkpg_op(&req.disk_path, BLKPG_DEL_PARTITION, pno, 0, 0);
        }
        let _ = blkrrpart_ioctl(&req.disk_path);

        let cur_surviving = list_kernel_partitions(&base_name);
        if cur_surviving.is_empty() {
            break;
        }

        // Terminer tout processus tiers maintenant encore un descripteur ouvert sur /dev/<base_name>*
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
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    }

    let surviving = list_kernel_partitions(&base_name);
    let layout = match compute_partition_layout(total_sectors, logical_block_size, &surviving) {
        Ok(l) => l,
        Err(msg) => {
            let _ = Command::new("udevadm").args(["control", "--start-exec-queue"]).status().await;
            fail_install_with_diagnosis(
                manager,
                ErrorDiagnosis {
                    summary: "Échec à l'Étape 2/6 : Calcul de la géométrie des partitions impossible".to_string(),
                    explanation: msg.clone(),
                    suggestion: "Vérifiez la taille du disque cible ou redémarrez l'ISO Live.".to_string(),
                    technical_detail: msg,
                },
            )
            .await;
            return;
        }
    };

    if layout.reused_p1 {
        let _ = blkpg_op(
            &req.disk_path,
            BLKPG_RESIZE_PARTITION,
            1,
            layout.boot_start,
            layout.boot_sectors,
        );
    }

    log(
        &manager,
        &format!(
            "Création de la table de partition GPT sur {} (Boot: pno={} LBA {}..{}, Root: pno={} LBA {}..{})",
            req.disk_path,
            layout.boot_pno,
            layout.boot_lba_start,
            layout.boot_lba_end,
            layout.root_pno,
            layout.root_lba_start,
            layout.root_lba_end
        ),
    )
    .await;

    let mut sgdisk_cmd = Command::new("sgdisk");
    sgdisk_cmd.args([
        "-o",
        "-a",
        "1",
        "-n",
        &format!("{}:{}:{}", layout.boot_pno, layout.boot_lba_start, layout.boot_lba_end),
        "-t",
        &format!("{}:ef00", layout.boot_pno),
        "-c",
        &format!("{}:BOOT", layout.boot_pno),
        "-n",
        &format!("{}:{}:{}", layout.root_pno, layout.root_lba_start, layout.root_lba_end),
        "-t",
        &format!("{}:8300", layout.root_pno),
        "-c",
        &format!("{}:ROOT", layout.root_pno),
        &req.disk_path,
    ]);
    if let Err(diag) = exec_cmd_named(manager.clone(), "l'Étape 2/6 (Partitionnement GPT)", "sgdisk", sgdisk_cmd).await {
        let _ = Command::new("udevadm").args(["control", "--start-exec-queue"]).status().await;
        fail_install_with_diagnosis(manager, diag).await;
        return;
    }

    let part_prefix = if req.disk_path.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("{}p", req.disk_path)
    } else {
        req.disk_path.clone()
    };
    let boot_part = format!("{}{}", part_prefix, layout.boot_pno);
    let root_part = format!("{}{}", part_prefix, layout.root_pno);
    let boot_name = boot_part.trim_start_matches("/dev/").to_string();
    let root_name = root_part.trim_start_matches("/dev/").to_string();

    log(&manager, "Actualisation des partitions auprès du noyau Linux...").await;
    let _ = Command::new("sync").status().await;
    let _ = blkrrpart_ioctl(&req.disk_path);

    for (pno, p_name, p_start, p_sectors) in [
        (layout.boot_pno, boot_name.as_str(), layout.boot_start, layout.boot_sectors),
        (layout.root_pno, root_name.as_str(), layout.root_start, layout.root_sectors),
    ] {
        let sys_path = format!("/sys/class/block/{}", p_name);
        if std::path::Path::new(&sys_path).exists() {
            match blkpg_op(&req.disk_path, BLKPG_RESIZE_PARTITION, pno, p_start, p_sectors) {
                Ok(()) => {
                    log(
                        &manager,
                        &format!(
                            "[DIAG] BLKPG_RESIZE_PARTITION réussi pour /dev/{} (start={}, sectors={})",
                            p_name, p_start, p_sectors
                        ),
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
                        &format!(
                            "[DIAG] BLKPG_ADD_PARTITION réussi pour /dev/{} (start={}, sectors={})",
                            p_name, p_start, p_sectors
                        ),
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

    let _ = Command::new("udevadm").args(["control", "--start-exec-queue"]).status().await;

    let mut partitions_ready = false;
    for attempt in 1..=15 {
        let _ = Command::new("udevadm")
            .args(["trigger", "--subsystem-match=block", "--action=add"])
            .status()
            .await;
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        for (pno, p_name, p_start, p_sectors) in [
            (layout.boot_pno, boot_name.as_str(), layout.boot_start, layout.boot_sectors),
            (layout.root_pno, root_name.as_str(), layout.root_start, layout.root_sectors),
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
        fail_install_with_diagnosis(
            manager,
            verbalize_error(
                "l'Étape 2/6 (Synchronisation des partitions noyau)",
                "BLKPG / udevadm",
                &format!(
                    "Les partitions {} et {} n'ont pas pu être créées dans /dev",
                    boot_part, root_part
                ),
                &[],
            ),
        )
        .await;
        return;
    }

    // Filet de sécurité post-Étape 2 : Vérifier la taille effective des partitions dans /sys/class/block
    let boot_actual_sectors: u64 = std::fs::read_to_string(format!("/sys/class/block/{}/size", boot_name))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let root_actual_sectors: u64 = std::fs::read_to_string(format!("/sys/class/block/{}/size", root_name))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    if boot_actual_sectors == 0 || root_actual_sectors == 0 {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 2/6 : Taille de partition nulle signalée par le noyau".to_string(),
                explanation: format!(
                    "Les partitions {} ({} secteurs) ou {} ({} secteurs) ont une taille invalide dans /sys/class/block.",
                    boot_part, boot_actual_sectors, root_part, root_actual_sectors
                ),
                suggestion: "Redémarrez le système Live ISO et relancez l'installation.".to_string(),
                technical_detail: format!("boot_sectors={}, root_sectors={}", boot_actual_sectors, root_actual_sectors),
            },
        )
        .await;
        return;
    }

    log_safety_check(
        &manager,
        &format!(
            "Périphériques bloc validés : Boot={} ({:.0} Mio) | Racine={} ({:.2} Gio).",
            boot_part,
            (boot_actual_sectors as f64 * 512.0) / 1_048_576.0,
            root_part,
            (root_actual_sectors as f64 * 512.0) / 1_073_741_824.0
        ),
    )
    .await;
    log_stage_success(
        &manager,
        2,
        6,
        &format!("Table GPT et nœuds {} + {} prêts.", boot_part, root_part),
    )
    .await;

    // =========================================================================
    // ÉTAPE 3/6 : FORMATAGE ET MONTAGE DES SYSTÈMES DE FICHIERS
    // =========================================================================
    log_stage_banner(
        &manager,
        3,
        6,
        "Formatage et montage des systèmes de fichiers",
        &format!(
            "Formater {} en FAT32 (EFI) et {} en {}, puis monter l'arborescence cible sous /mnt.",
            boot_part,
            root_part,
            req.filesystem.to_uppercase()
        ),
        8,
    )
    .await;

    let _ = Command::new("umount").args(["-R", "-l", &boot_part]).status().await;
    let _ = Command::new("umount").args(["-R", "-l", &root_part]).status().await;
    let _ = Command::new("wipefs").args(["-a", "-f", &boot_part]).status().await;
    let _ = Command::new("wipefs").args(["-a", "-f", &root_part]).status().await;
    let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

    log(&manager, &format!("Formatage FAT32 de la partition EFI {}...", boot_part)).await;
    let mut mkfs_vfat = Command::new("mkfs.vfat");
    mkfs_vfat.args(["-F", "32", "-n", "BOOT", &boot_part]);
    if let Err(diag) = exec_cmd_named(
        manager.clone(),
        "l'Étape 3/6 (Formatage EFI FAT32)",
        &format!("mkfs.vfat -F 32 -n BOOT {}", boot_part),
        mkfs_vfat,
    )
    .await
    {
        fail_install_with_diagnosis(manager, diag).await;
        return;
    }

    if req.filesystem == "btrfs" {
        log(&manager, &format!("Formatage Btrfs de la partition racine {}...", root_part)).await;
        let mut mkfs_btrfs = Command::new("mkfs.btrfs");
        mkfs_btrfs.args(["-f", "-L", "ROOT", &root_part]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Formatage Btrfs)",
            &format!("mkfs.btrfs -f -L ROOT {}", root_part),
            mkfs_btrfs,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        set_step(
            &manager,
            "Étape 3/6 : Création des sous-volumes Btrfs (@, @home, @nix, @snapshots)",
            11,
        )
        .await;
        let _ = Command::new("mkdir").args(["-p", "/mnt"]).status().await;
        let mut mount_root = Command::new("mount");
        mount_root.args(["-t", "btrfs", &root_part, "/mnt"]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Montage Btrfs initial)",
            &format!("mount -t btrfs {} /mnt", root_part),
            mount_root,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }

        for subvol in ["/mnt/@", "/mnt/@home", "/mnt/@nix", "/mnt/@snapshots"] {
            let mut sub_cmd = Command::new("btrfs");
            sub_cmd.args(["subvolume", "create", subvol]);
            if let Err(diag) = exec_cmd_named(
                manager.clone(),
                "l'Étape 3/6 (Création sous-volume Btrfs)",
                &format!("btrfs subvolume create {}", subvol),
                sub_cmd,
            )
            .await
            {
                fail_install_with_diagnosis(manager, diag).await;
                return;
            }
        }

        // Définir '@' comme sous-volume Btrfs principal par défaut de la partition
        let set_def_direct = Command::new("btrfs")
            .args(["subvolume", "set-default", "/mnt/@"])
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if !set_def_direct {
            if let Ok(list_out) = Command::new("btrfs")
                .args(["subvolume", "list", "/mnt"])
                .output()
                .await
            {
                let list_str = String::from_utf8_lossy(&list_out.stdout);
                for line in list_str.lines() {
                    let cols: Vec<&str> = line.split_whitespace().collect();
                    if cols.len() >= 9 && cols[0] == "ID" && cols.last() == Some(&"@") {
                        let sub_id = cols[1];
                        let _ = Command::new("btrfs")
                            .args(["subvolume", "set-default", sub_id, "/mnt"])
                            .status()
                            .await;
                        break;
                    }
                }
            }
        }

        let mut umount_initial = Command::new("umount");
        umount_initial.arg("/mnt");
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Démontage Btrfs temporaire)",
            "umount /mnt",
            umount_initial,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }

        set_step(
            &manager,
            "Étape 3/6 : Montage des sous-volumes Btrfs et de /mnt/boot",
            13,
        )
        .await;
        let mut m_at = Command::new("mount");
        m_at.args(["-o", "subvol=@,compress=zstd,noatime", &root_part, "/mnt"]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Montage sous-volume @)",
            &format!("mount -o subvol=@ {} /mnt", root_part),
            m_at,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }

        let _ = Command::new("mkdir")
            .args(["-p", "/mnt/home", "/mnt/nix", "/mnt/.snapshots", "/mnt/boot"])
            .status()
            .await;

        for (subvol_opt, target_mp) in [
            ("subvol=@home,compress=zstd", "/mnt/home"),
            ("subvol=@nix,compress=zstd,noatime", "/mnt/nix"),
            ("subvol=@snapshots,compress=zstd", "/mnt/.snapshots"),
        ] {
            let mut m_sub = Command::new("mount");
            m_sub.args(["-o", subvol_opt, &root_part, target_mp]);
            if let Err(diag) = exec_cmd_named(
                manager.clone(),
                "l'Étape 3/6 (Montage sous-volume Btrfs)",
                &format!("mount -o {} {} {}", subvol_opt, root_part, target_mp),
                m_sub,
            )
            .await
            {
                fail_install_with_diagnosis(manager, diag).await;
                return;
            }
        }

        let mut m_boot = Command::new("mount");
        m_boot.args([&boot_part, "/mnt/boot"]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Montage partition EFI /mnt/boot)",
            &format!("mount {} /mnt/boot", boot_part),
            m_boot,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
    } else {
        log(&manager, &format!("Formatage Ext4 de la partition racine {}...", root_part)).await;
        let mut mkfs_ext4 = Command::new("mkfs.ext4");
        mkfs_ext4.args(["-F", "-L", "ROOT", &root_part]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Formatage Ext4)",
            &format!("mkfs.ext4 -F -L ROOT {}", root_part),
            mkfs_ext4,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
        let _ = Command::new("udevadm").args(["settle", "--timeout=5"]).status().await;

        set_step(&manager, "Étape 3/6 : Montage des partitions Ext4 et EFI", 13).await;
        let _ = Command::new("mkdir").args(["-p", "/mnt"]).status().await;
        let mut m_root = Command::new("mount");
        m_root.args([&root_part, "/mnt"]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Montage racine Ext4 /mnt)",
            &format!("mount {} /mnt", root_part),
            m_root,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
        let _ = Command::new("mkdir").args(["-p", "/mnt/boot"]).status().await;
        let mut m_boot = Command::new("mount");
        m_boot.args([&boot_part, "/mnt/boot"]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 3/6 (Montage partition EFI /mnt/boot)",
            &format!("mount {} /mnt/boot", boot_part),
            m_boot,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
    }

    // Filet de sécurité post-Étape 3 : Vérifier /proc/mounts et tester l'écriture sur /mnt et /mnt/boot
    let proc_mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();
    let mnt_mounted = proc_mounts.lines().any(|l| {
        let mut cols = l.split_whitespace();
        let _src = cols.next();
        cols.next() == Some("/mnt")
    });
    let boot_mounted = proc_mounts.lines().any(|l| {
        let mut cols = l.split_whitespace();
        let _src = cols.next();
        cols.next() == Some("/mnt/boot")
    });

    if !mnt_mounted || !boot_mounted {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 3/6 : Point de montage /mnt ou /mnt/boot absent de /proc/mounts".to_string(),
                explanation: format!(
                    "Après le montage, /mnt (monté={}) ou /mnt/boot (monté={}) n'est pas actif dans la table du noyau.",
                    mnt_mounted, boot_mounted
                ),
                suggestion: "Vérifiez l'état du disque cible et réessayez.".to_string(),
                technical_detail: format!("mnt_mounted={}, boot_mounted={}", mnt_mounted, boot_mounted),
            },
        )
        .await;
        return;
    }

    for test_file in ["/mnt/.noos_write_probe", "/mnt/boot/.noos_write_probe"] {
        if let Err(e) = std::fs::write(test_file, b"ok") {
            fail_install_with_diagnosis(
                manager,
                ErrorDiagnosis {
                    summary: "Échec à l'Étape 3/6 : Système de fichiers monté en lecture seule ou inaccessible".to_string(),
                    explanation: format!("Le test d'écriture sur '{}' a échoué : {}.", test_file, e),
                    suggestion: "Vérifiez que le disque cible n'est pas protégé en écriture.".to_string(),
                    technical_detail: format!("write {} -> {}", test_file, e),
                },
            )
            .await;
            return;
        }
        let _ = std::fs::remove_file(test_file);
    }

    log_safety_check(&manager, "Montages /mnt et /mnt/boot actifs et vérifiés en lecture/écriture.").await;
    log_stage_success(
        &manager,
        3,
        6,
        &format!(
            "Systèmes de fichiers {} et FAT32 formatés et montés sous /mnt.",
            req.filesystem.to_uppercase()
        ),
    )
    .await;

    // =========================================================================
    // ÉTAPE 4/6 : GÉNÉRATION MATÉRIELLE HÔTE ET CONFIGURATION NIXOS
    // =========================================================================
    log_stage_banner(
        &manager,
        4,
        6,
        "Génération matérielle hôte et configuration NixOS",
        "Déployer le dépôt Noos NAS, générer dynamiquement hardware-configuration.nix par l'hôte (CPU, modules initrd, UUIDs) et configurer vars.nix.",
        15,
    )
    .await;

    let target_cfg_dir = "/mnt/etc/nixos";
    let _ = Command::new("mkdir").args(["-p", target_cfg_dir]).status().await;

    // 4.1 Déploiement initial du dépôt Noos NAS dans /mnt/etc/nixos
    // (hosts/nas/hardware-configuration.nix reste le fichier générique de repli suivi par Git et n'est JAMAIS écrasé)
    if let Some(source_dir) = embedded_source {
        log(
            &manager,
            &format!("Copie de la configuration locale embarquée ({}) vers /mnt/etc/nixos...", source_dir),
        )
        .await;
        let mut cp_cmd = Command::new("cp");
        cp_cmd.args(["-a", &format!("{}/.", source_dir), target_cfg_dir]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 4/6 (Copie de la source locale)",
            &format!("cp -a {}/. {}", source_dir, target_cfg_dir),
            cp_cmd,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
    } else {
        log(
            &manager,
            "Clonage du dépôt officiel GitHub Chomiam/noos-nas vers /mnt/etc/nixos...",
        )
        .await;
        let tmp_clone_dir = "/tmp/noos-nas-repo";
        let mut clone_ok = false;
        let mut last_clone_diag = None;

        for attempt in 1..=3 {
            let _ = Command::new("rm").args(["-rf", tmp_clone_dir]).status().await;
            let mut clone_cmd = Command::new("git");
            clone_cmd.args([
                "clone",
                "--depth=1",
                "https://github.com/Chomiam/noos-nas.git",
                tmp_clone_dir,
            ]);
            match exec_cmd_named(
                manager.clone(),
                "l'Étape 4/6 (Clonage GitHub Noos NAS)",
                "git clone --depth=1 https://github.com/Chomiam/noos-nas.git",
                clone_cmd,
            )
            .await
            {
                Ok(()) => {
                    clone_ok = true;
                    break;
                }
                Err(diag) => {
                    last_clone_diag = Some(diag);
                    if attempt < 3 {
                        log(
                            &manager,
                            &format!(
                                "⚠️ Nouvelle tentative de clonage GitHub ({}/3) dans 3 secondes...",
                                attempt + 1
                            ),
                        )
                        .await;
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    }
                }
            }
        }

        if !clone_ok {
            if let Some(diag) = last_clone_diag {
                fail_install_with_diagnosis(manager, diag).await;
            }
            return;
        }

        let mut cp_repo = Command::new("cp");
        cp_repo.args(["-a", &format!("{}/.", tmp_clone_dir), target_cfg_dir]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 4/6 (Copie du dépôt vers /mnt/etc/nixos)",
            "cp -a /tmp/noos-nas-repo/. /mnt/etc/nixos",
            cp_repo,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
        let _ = Command::new("rm").args(["-rf", tmp_clone_dir]).status().await;
    }

    if !std::path::Path::new(&format!("{}/flake.nix", target_cfg_dir)).exists() {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 4/6 : Fichier flake.nix introuvable dans /mnt/etc/nixos".to_string(),
                explanation: "Le dépôt de configuration Noos NAS ne contient pas de fichier flake.nix valide.".to_string(),
                suggestion: "Vérifiez la connexion réseau ou l'intégrité du dépôt Chomiam/noos-nas.".to_string(),
                technical_detail: "/mnt/etc/nixos/flake.nix absent".to_string(),
            },
        )
        .await;
        return;
    }

    // 4.2 Génération dynamique de hardware-configuration.nix par la machine hôte
    set_step(
        &manager,
        "Étape 4/6 : Détection matérielle de l'hôte (nixos-generate-config)",
        17,
    )
    .await;
    log(
        &manager,
        "Inspection du matériel de la machine hôte (CPU, bus PCI/NVMe/SATA/USB, UUIDs des partitions montées sous /mnt)...",
    )
    .await;

    // --show-hardware-config génère uniquement hardware-configuration.nix sans créer de configuration.nix parasite à la racine
    let show_hw_out = Command::new("nixos-generate-config")
        .args(["--root", "/mnt", "--show-hardware-config"])
        .output()
        .await;

    let hw_config_dest = format!("{}/hardware-configuration.nix", target_cfg_dir);
    let mut hw_content = String::new();

    if let Ok(out) = show_hw_out {
        if out.status.success() {
            let stdout_str = String::from_utf8_lossy(&out.stdout).to_string();
            if stdout_str.contains("fileSystems.\"/\"") {
                hw_content = stdout_str;
            }
        }
    }

    // Repli sur `nixos-generate-config --root /mnt` classique si `--show-hardware-config` n'a pas suffi
    if hw_content.is_empty() {
        let mut gen_cmd = Command::new("nixos-generate-config");
        gen_cmd.args(["--root", "/mnt"]);
        if let Err(diag) = exec_cmd_named(
            manager.clone(),
            "l'Étape 4/6 (Détection matérielle nixos-generate-config)",
            "nixos-generate-config --root /mnt",
            gen_cmd,
        )
        .await
        {
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
        // Supprimer le fichier /mnt/etc/nixos/configuration.nix monolithique par défaut généré par nixos-generate-config
        // car le Flake utilise hosts/nas/configuration.nix
        let _ = std::fs::remove_file(format!("{}/configuration.nix", target_cfg_dir));
        hw_content = std::fs::read_to_string(&hw_config_dest).unwrap_or_default();
    }

    if !hw_content.contains("fileSystems.\"/\"") || !hw_content.contains("fileSystems.\"/boot\"") {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 4/6 : Fichier hardware-configuration.nix généré par l'hôte invalide ou incomplet".to_string(),
                explanation: "nixos-generate-config n'a pas détecté les points de montage / et /boot dans /mnt/etc/nixos/hardware-configuration.nix.".to_string(),
                suggestion: "Relancez l'installation pour régénérer la détection matérielle sur l'hôte.".to_string(),
                technical_detail: "hardware-configuration.nix sans fileSystems.\"/\" ou fileSystems.\"/boot\"".to_string(),
            },
        )
        .await;
        return;
    }

    // Si la machine n'a pas démarré en mode UEFI (/sys/firmware/efi absent), injecter canTouchEfiVariables = false
    // directement dans le hardware-configuration.nix généré par l'hôte (sans écraser ses modules ni ses fileSystems)
    if !is_efi_booted {
        hw_content = inject_nix_module_attr(
            &hw_content,
            "boot.loader.efi.canTouchEfiVariables = lib.mkForce false;",
        );
        log_safety_check(
            &manager,
            "Option 'boot.loader.efi.canTouchEfiVariables = lib.mkForce false' injectée dans hardware-configuration.nix de l'hôte.",
        )
        .await;
    }

    // Écriture de la configuration matérielle générée par l'hôte UNIQUEMENT dans les emplacements propres à la machine :
    // - /mnt/etc/nixos/hardware-configuration.nix (importé en priorité par hosts/nas/configuration.nix)
    // - /mnt/etc/nixos/hosts/nas/hardware.local.nix (miroir local ignoré par Git en amont)
    // - /mnt/etc/nixos/.hardware-configuration.nix.backup (sauvegarde de résilience du Dashboard et de users.nix)
    // IMPORTANT : On ne touche JAMAIS à /mnt/etc/nixos/hosts/nas/hardware-configuration.nix (générique suivi par Git).
    let _ = Command::new("mkdir")
        .args(["-p", &format!("{}/hosts/nas", target_cfg_dir)])
        .status()
        .await;
    for dest in [
        hw_config_dest.clone(),
        format!("{}/hosts/nas/hardware.local.nix", target_cfg_dir),
        format!("{}/.hardware-configuration.nix.backup", target_cfg_dir),
    ] {
        if let Err(e) = std::fs::write(&dest, &hw_content) {
            fail_install_with_diagnosis(
                manager,
                ErrorDiagnosis {
                    summary: "Échec à l'Étape 4/6 : Impossible d'écrire hardware-configuration.nix".to_string(),
                    explanation: format!("L'écriture de '{}' a échoué : {}.", dest, e),
                    suggestion: "Vérifiez l'espace disque sur /mnt.".to_string(),
                    technical_detail: e.to_string(),
                },
            )
            .await;
            return;
        }
    }

    let hw_summary = summarize_generated_hardware_config(&hw_content);
    log_safety_check(
        &manager,
        &format!(
            "hardware-configuration.nix généré par l'hôte ({}) et isolé de hosts/nas/hardware-configuration.nix.",
            hw_summary
        ),
    )
    .await;

    // Création du lien symbolique de sous-répertoire
    let _ = Command::new("ln")
        .args(["-sfn", ".", &format!("{}/noos-nas", target_cfg_dir)])
        .status()
        .await;

    // Hachage sécurisé du mot de passe via STDIN (évite tout problème avec un mot de passe commençant par '-')
    let mut password_hash = String::new();
    if let Ok(mut child) = Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(format!("{}\n", req.password).as_bytes()).await;
        }
        if let Ok(out) = child.wait_with_output().await {
            if out.status.success() {
                password_hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }

    if !password_hash.starts_with("$6$") && !password_hash.starts_with("$y$") {
        log(&manager, "Utilisation de mkpasswd en repli pour le hachage SHA-512...").await;
        if let Ok(mut child) = Command::new("mkpasswd")
            .args(["-m", "sha-512", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(format!("{}\n", req.password).as_bytes()).await;
            }
            if let Ok(out) = child.wait_with_output().await {
                if out.status.success() {
                    password_hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
                }
            }
        }
    }

    if !password_hash.starts_with("$6$") && !password_hash.starts_with("$y$") {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 4/6 : Impossible de générer l'empreinte chiffrée du mot de passe".to_string(),
                explanation: "Les utilitaires openssl et mkpasswd n'ont pas produit d'empreinte SHA-512 valide.".to_string(),
                suggestion: "Essayez un mot de passe standard et relancez l'installation.".to_string(),
                technical_detail: format!("hash généré invalide (len={})", password_hash.len()),
            },
        )
        .await;
        return;
    }
    log_safety_check(&manager, "Empreinte SHA-512 du mot de passe administrateur générée et vérifiée.").await;

    let raw_full_name = req
        .full_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("Administrateur Noos NAS");

    let safe_hostname = escape_nix_string(&req.hostname);
    let safe_username = escape_nix_string(&req.username);
    let safe_full_name = escape_nix_string(raw_full_name);
    let safe_hash = escape_nix_string(&password_hash);

    // Écriture du fichier vars.nix conforme au schéma complet de Chomiam/noos-nas (vars-defaults.nix)
    let vars_content = format!(
        r#"# Variables générées automatiquement par l'installateur Noos NAS
{{
  hostName = "{safe_hostname}";
  timeZone = "Europe/Paris";
  defaultLocale = "fr_FR.UTF-8";

  user = {{
    username = "{safe_username}";
    fullName = "{safe_full_name}";
    homeDirectory = "/home/{safe_username}";
    initialHashedPassword = "{safe_hash}";
    hashedPassword = "{safe_hash}";
  }};

  firewall = {{
    enable = true;
    strictLanOnly = false;
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
    docker = {{
      enable = true;
    }};
    wireguard = {{
      enable = true;
    }};
    virtualisation = {{
      enable = true;
    }};
    nixLd = {{
      enable = true;
    }};
  }};
}}
"#
    );

    for path in [
        format!("{}/vars.nix", target_cfg_dir),
        format!("{}/vars.local.nix", target_cfg_dir),
        format!("{}/.vars.nix.backup", target_cfg_dir),
    ] {
        if let Err(e) = std::fs::write(&path, &vars_content) {
            fail_install_with_diagnosis(
                manager,
                ErrorDiagnosis {
                    summary: "Échec à l'Étape 4/6 : Écriture de vars.nix impossible".to_string(),
                    explanation: format!("Impossible d'écrire le fichier '{}' : {}.", path, e),
                    suggestion: "Vérifiez l'espace disponible sur la partition racine /mnt.".to_string(),
                    technical_detail: e.to_string(),
                },
            )
            .await;
            return;
        }
    }

    let _ = Command::new("mkdir").args(["-p", "/mnt/var/lib/noos"]).status().await;
    let _ = std::fs::write("/mnt/var/lib/noos/update_channel", "stable\n");

    // Enregistrer les fichiers propres à l'hôte (hardware-configuration.nix, hosts/nas/hardware.local.nix, vars.nix, vars.local.nix)
    // dans l'index Git local pour que Nix Flake (en mode git+file:// comme en mode path:) les inclue dans le store Nix,
    // tout en gardant hosts/nas/hardware-configuration.nix intact par rapport au dépôt GitHub.
    if !std::path::Path::new(&format!("{}/.git", target_cfg_dir)).exists() {
        let _ = Command::new("git")
            .args(["-C", target_cfg_dir, "init"])
            .status()
            .await;
    }

    let mut git_add = Command::new("git");
    git_add.args([
        "-C",
        target_cfg_dir,
        "add",
        "-f",
        "hardware-configuration.nix",
        "hosts/nas/hardware.local.nix",
        "vars.nix",
        "vars.local.nix",
    ]);
    if let Err(diag) = exec_cmd_named(
        manager.clone(),
        "l'Étape 4/6 (Enregistrement des fichiers hôte pour Nix Flake)",
        "git -C /mnt/etc/nixos add -f hardware-configuration.nix hosts/nas/hardware.local.nix vars.nix vars.local.nix",
        git_add,
    )
    .await
    {
        fail_install_with_diagnosis(manager, diag).await;
        return;
    }

    // Filet de sécurité post-Étape 4 : Vérifier que git ls-files contient bien hardware-configuration.nix et vars.local.nix
    let ls_files_out = Command::new("git")
        .args(["-C", target_cfg_dir, "ls-files"])
        .output()
        .await
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    if !ls_files_out.lines().any(|l| l.trim() == "hardware-configuration.nix")
        || !ls_files_out.lines().any(|l| l.trim() == "vars.local.nix")
    {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 4/6 : Fichiers de configuration hôte non pris en compte par le Flake Nix".to_string(),
                explanation: "Les fichiers hardware-configuration.nix ou vars.local.nix ne figurent pas dans l'index du Flake /mnt/etc/nixos.".to_string(),
                suggestion: "Relancez l'installation.".to_string(),
                technical_detail: format!("git ls-files: {}", ls_files_out.replace('\n', ", ")),
            },
        )
        .await;
        return;
    }

    log_safety_check(
        &manager,
        "Configuration hôte vérifiée : /mnt/etc/nixos/hardware-configuration.nix (généré par la machine) et vars.local.nix sont prêts pour l'évaluation Flake.",
    )
    .await;
    log_stage_success(
        &manager,
        4,
        6,
        "Configuration matérielle hôte et Flake Noos NAS prêts pour le déploiement.",
    )
    .await;

    // =========================================================================
    // ÉTAPE 5/6 : INSTALLATION DU SYSTÈME NIXOS ET CHARGEUR D'AMORÇAGE
    // =========================================================================
    log_stage_banner(
        &manager,
        5,
        6,
        "Installation du système NixOS et du chargeur d'amorçage",
        "Assembler la dérivation NixOS (#nas), copier la clôture système dans /mnt/nix/store et installer systemd-boot dans /mnt/boot.",
        20,
    )
    .await;
    log(
        &manager,
        "⏳ Déploiement de Noos NAS en cours... Cette étape dure généralement quelques minutes selon votre connexion et la vitesse du disque.",
    )
    .await;

    let mut nix_cmd = Command::new("nixos-install");
    nix_cmd.args([
        "--flake",
        &format!("{}#nas", target_cfg_dir),
        "--no-root-password",
        "--impure",
    ]);

    if let Err((raw_err, stderr_lines)) = exec_nixos_install(manager.clone(), nix_cmd).await {
        let joined_err = stderr_lines.join(" ").to_lowercase();
        let is_efi_nvram_err = joined_err.contains("bootctl")
            || joined_err.contains("efivarfs")
            || joined_err.contains("efi variables")
            || joined_err.contains("failed to write efi");

        if is_efi_nvram_err {
            log(
                &manager,
                "⚠️ [AUTO-RÉPARATION] Le firmware UEFI a refusé l'écriture NVRAM (bootctl). Injection de 'canTouchEfiVariables = lib.mkForce false' dans hardware-configuration.nix de l'hôte et relance...",
            )
            .await;
            let patched_hw = inject_nix_module_attr(
                &hw_content,
                "boot.loader.efi.canTouchEfiVariables = lib.mkForce false;",
            );
            for dest in [
                hw_config_dest.clone(),
                format!("{}/hosts/nas/hardware.local.nix", target_cfg_dir),
                format!("{}/.hardware-configuration.nix.backup", target_cfg_dir),
            ] {
                let _ = std::fs::write(&dest, &patched_hw);
            }
            let _ = Command::new("git")
                .args([
                    "-C",
                    target_cfg_dir,
                    "add",
                    "-f",
                    "hardware-configuration.nix",
                    "hosts/nas/hardware.local.nix",
                ])
                .status()
                .await;

            let mut retry_cmd = Command::new("nixos-install");
            retry_cmd.args([
                "--flake",
                &format!("{}#nas", target_cfg_dir),
                "--no-root-password",
                "--impure",
            ]);
            if let Err((retry_raw, retry_stderr)) = exec_nixos_install(manager.clone(), retry_cmd).await {
                let diag = verbalize_error(
                    "l'Étape 5/6 (Installation du système NixOS)",
                    "nixos-install --flake /mnt/etc/nixos#nas",
                    &retry_raw,
                    &retry_stderr,
                );
                fail_install_with_diagnosis(manager, diag).await;
                return;
            }
        } else {
            let diag = verbalize_error(
                "l'Étape 5/6 (Installation du système NixOS)",
                "nixos-install --flake /mnt/etc/nixos#nas",
                &raw_err,
                &stderr_lines,
            );
            fail_install_with_diagnosis(manager, diag).await;
            return;
        }
    }

    // Filet de sécurité post-Étape 5 : Vérifier que /mnt/nix/store et le chargeur d'amorçage dans /mnt/boot sont peuplés
    let nix_store_ok = std::fs::read_dir("/mnt/nix/store")
        .map(|mut d| d.next().is_some())
        .unwrap_or(false);
    let boot_loader_ok = std::path::Path::new("/mnt/boot/EFI").exists()
        || std::path::Path::new("/mnt/boot/loader").exists()
        || std::path::Path::new("/mnt/boot/kernels").exists();

    if !nix_store_ok || !boot_loader_ok {
        fail_install_with_diagnosis(
            manager,
            ErrorDiagnosis {
                summary: "Échec à l'Étape 5/6 : Vérification d'intégrité post-installation incomplète".to_string(),
                explanation: format!(
                    "Le store Nix (/mnt/nix/store peuplé={}) ou le chargeur d'amorçage (/mnt/boot/EFI présent={}) est incomplet.",
                    nix_store_ok, boot_loader_ok
                ),
                suggestion: "Relancez l'installation et vérifiez les logs ci-dessus.".to_string(),
                technical_detail: format!("nix_store_ok={}, boot_loader_ok={}", nix_store_ok, boot_loader_ok),
            },
        )
        .await;
        return;
    }

    log_safety_check(&manager, "Intégrité de /mnt/nix/store et du chargeur d'amorçage /mnt/boot/EFI vérifiée.").await;
    log_stage_success(&manager, 5, 6, "Système NixOS et chargeur systemd-boot installés avec succès.").await;

    // =========================================================================
    // ÉTAPE 6/6 : FINALISATION, SYNCHRONISATION ET DÉMONTAGE PROPRE
    // =========================================================================
    log_stage_banner(
        &manager,
        6,
        6,
        "Finalisation, synchronisation et démontage propre",
        "Vider les tampons d'écriture (sync) et démonter proprement les partitions (/mnt/boot, /mnt) avant le redémarrage.",
        98,
    )
    .await;

    let _ = Command::new("sync").status().await;

    log(&manager, "Démontage propre des partitions installées (/mnt)...").await;
    if let Ok(status) = Command::new("umount").args(["-R", "/mnt"]).status().await {
        if !status.success() {
            let _ = Command::new("umount").args(["-R", "-l", "/mnt"]).status().await;
        }
    }
    let _ = Command::new("sync").status().await;

    log_safety_check(&manager, "Tampons disque synchronisés et systèmes de fichiers démontés proprement.").await;
    log_stage_success(&manager, 6, 6, "Installation 100% terminée et vérifiée.").await;

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

async fn fail_install_with_diagnosis(manager: Arc<Mutex<InstallManager>>, diag: ErrorDiagnosis) {
    let mut m = manager.lock().await;
    m.status = "error".to_string();
    let full_error = format!(
        "{} — {} Solution : {}",
        diag.summary, diag.explanation, diag.suggestion
    );
    m.error = Some(full_error);
    m.add_log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
    m.add_log(format!("💥 ÉCHEC DE L'INSTALLATION : {}", diag.summary));
    m.add_log(format!("🔍 Cause probable : {}", diag.explanation));
    m.add_log(format!("💡 Action recommandée : {}", diag.suggestion));
    m.add_log(format!("🔧 Détail technique : {}", diag.technical_detail));
    m.add_log("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_nix_string() {
        assert_eq!(escape_nix_string("simple"), "simple");
        assert_eq!(escape_nix_string("Jean \"Admin\""), "Jean \\\"Admin\\\"");
        assert_eq!(escape_nix_string("a\\b"), "a\\\\b");
        assert_eq!(escape_nix_string("pwn ${pkgs.bash}"), "pwn \\${pkgs.bash}");
        assert_eq!(escape_nix_string("$6$rounds$hash"), "$6$rounds$hash");
    }

    #[test]
    fn test_validate_hostname() {
        assert!(validate_hostname("noos-nas"));
        assert!(validate_hostname("nas01"));
        assert!(!validate_hostname(""));
        assert!(!validate_hostname("-noos"));
        assert!(!validate_hostname("noos-"));
        assert!(!validate_hostname("noos_nas"));
        assert!(!validate_hostname("noos nas"));
    }

    #[test]
    fn test_compute_partition_layout_clean_disk() {
        // Disque de 20 Gio (41_943_040 secteurs de 512o)
        let layout = compute_partition_layout(41_943_040, 512, &[]).expect("doit réussir");
        assert_eq!(layout.boot_pno, 1);
        assert_eq!(layout.boot_start, 2048);
        assert_eq!(layout.boot_sectors, 2_097_152);
        assert_eq!(layout.root_pno, 2);
        assert_eq!(layout.root_start, 2048 + 2_097_152);
        assert!(!layout.reused_p1);
    }

    #[test]
    fn test_compute_partition_layout_surviving_p1_reused() {
        let surviving = vec![KernelPartInfo {
            name: "sda1".to_string(),
            pno: 1,
            start_sect: 2048,
            size_sect: 8,
            dev_maj_min: "8:1".to_string(),
            holders: vec![],
        }];
        let layout = compute_partition_layout(41_943_040, 512, &surviving).expect("doit réussir");
        assert!(layout.reused_p1);
        assert_eq!(layout.boot_pno, 1);
        assert_eq!(layout.boot_start, 2048);
        assert_eq!(layout.root_pno, 2);
        assert_eq!(layout.root_start, 2048 + 2_097_152);
    }

    #[test]
    fn test_compute_partition_layout_never_leaves_leading_unallocated_gap() {
        // Même si des partitions résiduelles sont signalées, le partitionnement complet du disque
        // doit toujours placer BOOT (pno=1) à 2048 et ROOT (pno=2) juste après, sans trou non alloué.
        let surviving = vec![
            KernelPartInfo {
                name: "sda3".to_string(),
                pno: 3,
                start_sect: 6_399_992,
                size_sect: 8,
                dev_maj_min: "8:3".to_string(),
                holders: vec![],
            },
        ];
        let layout = compute_partition_layout(488_397_168, 512, &surviving).expect("doit réussir");
        assert!(!layout.reused_p1);
        assert_eq!(layout.boot_pno, 1);
        assert_eq!(layout.boot_start, 2048);
        assert_eq!(layout.boot_sectors, 2_097_152);
        assert_eq!(layout.root_pno, 2);
        assert_eq!(layout.root_start, 2048 + 2_097_152);
        assert_eq!(layout.root_sectors, 488_394_752 - (2048 + 2_097_152));
    }

    #[test]
    fn test_compute_partition_layout_4kn_disk() {
        // Disque 4Kn (logical_block_size = 4096 -> 8 secteurs de 512o par LBA)
        let layout = compute_partition_layout(41_943_040, 4096, &[]).expect("doit réussir");
        assert_eq!(layout.boot_lba_start, 2048 / 8);
        assert_eq!(layout.boot_lba_end, (2048 + 2_097_152) / 8 - 1);
    }

    #[test]
    fn test_compute_partition_layout_disk_too_small_fails() {
        // Disque de 2 Gio seulement -> doit échouer proprement
        let res = compute_partition_layout(4_194_304, 512, &[]);
        assert!(res.is_err());
    }

    #[test]
    fn test_verbalize_error_translations() {
        let diag_net = verbalize_error(
            "l'Étape 1/6",
            "git ls-remote",
            "exit 128",
            &["fatal: unable to access 'https://github.com/Chomiam/noos-nas.git/': Could not resolve host: github.com".to_string()],
        );
        assert!(diag_net.summary.contains("Connexion réseau"));

        let diag_dev = verbalize_error(
            "l'Étape 3/6",
            "mkfs.ext4",
            "exit 1",
            &["Le fichier /dev/sda2 n'existe pas et aucune taille n'a été spécifiée.".to_string()],
        );
        assert!(diag_dev.summary.contains("Périphérique de partition introuvable"));
    }

    #[test]
    fn test_inject_nix_module_attr_preserves_host_hardware_config() {
        let sample_hw = r#"{ config, lib, pkgs, modulesPath, ... }:

{
  imports =
    [ (modulesPath + "/installer/scan/not-detected.nix")
    ];

  boot.initrd.availableKernelModules = [ "xhci_pci" "ahci" "nvme" "usbhid" "sd_mod" ];
  boot.kernelModules = [ "kvm-intel" ];

  fileSystems."/" =
    { device = "/dev/disk/by-uuid/1234-5678";
      fsType = "btrfs";
    };

  fileSystems."/boot" =
    { device = "/dev/disk/by-uuid/ABCD-EF01";
      fsType = "vfat";
    };

  hardware.cpu.intel.updateMicrocode = lib.mkDefault config.hardware.enableRedistributableFirmware;
}
"#;
        let patched = inject_nix_module_attr(
            sample_hw,
            "boot.loader.efi.canTouchEfiVariables = lib.mkForce false;",
        );
        assert!(patched.contains("fileSystems.\"/\""));
        assert!(patched.contains("fileSystems.\"/boot\""));
        assert!(patched.contains("boot.loader.efi.canTouchEfiVariables = lib.mkForce false;"));
        assert!(patched.trim_end().ends_with('}'));

        // Idempotence : un second appel ne duplique pas la ligne
        let patched_twice = inject_nix_module_attr(
            &patched,
            "boot.loader.efi.canTouchEfiVariables = lib.mkForce false;",
        );
        assert_eq!(patched, patched_twice);

        let summary = summarize_generated_hardware_config(sample_hw);
        assert!(summary.contains("CPU=Intel"));
        assert!(summary.contains("xhci_pci ahci nvme usbhid sd_mod"));
        assert!(summary.contains("Points de montage déclarés=2"));
    }
}


