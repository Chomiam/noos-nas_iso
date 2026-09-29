use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub size_human: String,
    pub model: String,
    pub transport: String,
    pub is_rotational: bool,
    pub is_install_media: bool,
    pub partitions_count: usize,
    pub current_mounts: Vec<String>,
}

#[derive(Deserialize)]
struct LsblkOutput {
    #[serde(default)]
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    path: Option<String>,
    size: Option<u64>,
    model: Option<String>,
    #[serde(rename = "type")]
    device_type: Option<String>,
    tran: Option<String>,
    rota: Option<bool>,
    mountpoints: Option<Vec<Option<String>>>,
    children: Option<Vec<LsblkChild>>,
}

#[derive(Deserialize)]
struct LsblkChild {
    mountpoints: Option<Vec<Option<String>>>,
}

fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    const TIB: u64 = GIB * 1024;

    if bytes >= TIB {
        format!("{:.1} To ({:.1} TiB)", bytes as f64 / 1e12, bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.1} Go ({:.1} GiB)", bytes as f64 / 1e9, bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} Mo", bytes as f64 / 1e6)
    } else {
        format!("{} octets", bytes)
    }
}

pub fn list_available_disks() -> Vec<DiskInfo> {
    let output = match Command::new("lsblk")
        .args(["-J", "-b", "-o", "NAME,PATH,SIZE,MODEL,TYPE,TRAN,ROTA,MOUNTPOINTS"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    let parsed: LsblkOutput = match serde_json::from_slice(&output.stdout) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut disks = Vec::new();

    for dev in parsed.blockdevices {
        let name = dev.name;
        // Ignore loop, ram, zram devices
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") {
            continue;
        }

        // Only keep disk type
        if dev.device_type.as_deref() != Some("disk") {
            continue;
        }

        let path = dev.path.unwrap_or_else(|| format!("/dev/{}", name));
        let size_bytes = dev.size.unwrap_or(0);
        if size_bytes == 0 {
            continue;
        }

        // Collect mountpoints
        let mut mounts = Vec::new();
        if let Some(mps) = dev.mountpoints {
            for m in mps.into_iter().flatten() {
                if !m.is_empty() && !m.starts_with('[') {
                    mounts.push(m);
                }
            }
        }

        let mut is_install_media = false;
        let mut parts_count = 0;

        if let Some(children) = dev.children {
            parts_count = children.len();
            for c in children {
                if let Some(cmps) = c.mountpoints {
                    for m in cmps.into_iter().flatten() {
                        if !m.is_empty() {
                            // Check if mounted as live ISO root
                            if m == "/iso" || m == "/run/iso" || m == "/cdrom" || m.starts_with("/nix/.ro-store") {
                                is_install_media = true;
                            }
                            if !m.starts_with('[') {
                                mounts.push(m);
                            }
                        }
                    }
                }
            }
        }

        for m in &mounts {
            if m == "/iso" || m == "/run/iso" || m == "/cdrom" || m.starts_with("/nix/.ro-store") {
                is_install_media = true;
            }
        }

        let model = dev.model.unwrap_or_else(|| "Disque Générique".to_string()).trim().to_string();
        let transport = dev.tran.unwrap_or_else(|| "SATA/NVMe".to_string()).to_uppercase();
        let is_rotational = dev.rota.unwrap_or(false);

        disks.push(DiskInfo {
            name,
            path,
            size_bytes,
            size_human: format_size(size_bytes),
            model,
            transport,
            is_rotational,
            is_install_media,
            partitions_count: parts_count,
            current_mounts: mounts,
        });
    }

    disks
}
