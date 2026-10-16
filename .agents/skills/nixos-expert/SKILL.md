---
name: nixos-expert
description: "Expert guide for NixOS (25.11/26.05+), Nix Flakes, nixos-generate-config host hardware isolation, nixos-install Live ISO engineering, declarative module system (mkOption/mkDefault/mkForce), systemd hardening, and NAS/server architectures / Panduan ahli untuk NixOS, Nix Flakes, isolasi perangkat keras host nixos-generate-config, rekayasa Live ISO nixos-install, dan arsitektur server/NAS deklaratif."
author: "Roedy Rustam"
version: "4.2.1"
---

# NixOS & Declarative Linux Systems Expert (2026 Edition)

[English](#english) | [Bahasa Indonesia](#bahasa-indonesia) | [Français / Conventions Noos NAS](#francais)

---

<a name="english"></a>
## English

### Orchestration & Integration
Connects and orchestrates with `rust-programming-expert`, `ci-cd-devops-architect`, `linux-kernel-storage`, `brainstorming`, `zero-to-prod-orchestrator`, and `production-ready-hardener` to design, build, install, and maintain reproducible NixOS systems, Live ISOs, and NAS appliances.

### 1. Golden Rule of `hardware-configuration.nix` (Host Hardware Isolation)
**NEVER treat `hardware-configuration.nix` as a static file shared across machines.** Every physical or virtual machine has a unique hardware topology:
- **CPU & Microcode**: Intel (`hardware.cpu.intel.updateMicrocode`) vs AMD (`hardware.cpu.amd.updateMicrocode`) and host platform (`nixpkgs.hostPlatform`).
- **Stage-1 Initrd & Kernel Modules**: `boot.initrd.availableKernelModules` (`nvme`, `ahci`, `xhci_pci`, `virtio_pci`, `virtio_scsi`, `mpt3sas`, `usbhid`, `sd_mod`) and `boot.kernelModules` (`kvm-intel`, `kvm-amd`) are discovered by scanning `/sys/bus/*` and `/sys/class/*` on the actual host. Missing a storage controller module in `boot.initrd.availableKernelModules` renders the system unbootable (cannot mount `/` in stage-1).
- **Persistent Block UUIDs & Subvolumes**: `fileSystems."/"`, `fileSystems."/boot"`, and Btrfs subvolumes (`@`, `@home`, `@nix`, `@snapshots`) must bind to the exact `/dev/disk/by-uuid/<UUID>` created during formatting on that specific machine.

#### Mandatory Generation Workflow During Installation (`nixos-install`)
1. Partition, format, and mount all target filesystems under `/mnt` (`/mnt`, `/mnt/boot`, and for Btrfs: `/mnt/home`, `/mnt/nix`, `/mnt/.snapshots`).
2. Deploy the configuration repository (e.g., `noos-nas`) into `/mnt/etc/nixos` **first**, leaving any Git-tracked generic fallback (`hosts/nas/hardware-configuration.nix`) completely untouched.
3. Run `nixos-generate-config` **on the target host** to probe `/sys`, `/proc/cpuinfo`, and `/proc/mounts`:
   ```bash
   # Option A: Generate hardware-configuration.nix directly to stdout without creating a dummy configuration.nix
   nixos-generate-config --root /mnt --show-hardware-config > /mnt/etc/nixos/hardware-configuration.nix

   # Option B: Standard invocation (writes /mnt/etc/nixos/hardware-configuration.nix)
   nixos-generate-config --root /mnt
   ```
4. Ensure `pciutils` (`lspci`), `usbutils` (`lsusb`), `kmod`, `util-linux` (`blkid`/`findmnt`), and `btrfs-progs` are present in the installer's `PATH` so `nixos-generate-config` detects all PCI/NVMe/VirtIO controllers and Btrfs subvolumes accurately.
5. **Never overwrite the Git-tracked fallback** (`hosts/<host>/hardware-configuration.nix`). Store the host-generated file at `/mnt/etc/nixos/hardware-configuration.nix` (and local untracked backups such as `hosts/<host>/hardware.local.nix` and `.hardware-configuration.nix.backup`).

---

### 2. Nix Flakes Fetchers (`git+file://` vs `path:`) & `.gitignore` Semantics
Understanding how Nix evaluates Flake URIs is critical when combining a Git-tracked NixOS repository with host-generated files:

| Flake URI Scheme | Example | How Nix Fetches Source Tree | Behavior with `.gitignore`d Host Files (`hardware-configuration.nix`, `*.local.nix`) |
| :--- | :--- | :--- | :--- |
| **`git+file://` (Default when `.git` exists)** | `/etc/nixos#nas` or `.#nas` | Queries Git index (`git ls-files`) | **Ignores untracked files listed in `.gitignore`!** `builtins.pathExists ../../hardware-configuration.nix` silently returns `false` unless the file is registered in `.git/index`. |
| **`path:` (Explicit Directory Scheme)** | `path:/etc/nixos#nas` | Copies directory tree directly from filesystem | **Includes all files on disk** regardless of `.gitignore` or Git tracking state. |

#### Best Practices for Hybrid Git + Host-Generated Flakes
1. **Conditional Import Chain in `configuration.nix`**:
   ```nix
   imports = [
     (if builtins.pathExists ../../hardware-configuration.nix
      then ../../hardware-configuration.nix
      else if builtins.pathExists ./hardware.local.nix
      then ./hardware.local.nix
      else ./hardware-configuration.nix)
   ];
   ```
   > ⚠️ **Import Pitfall**: Because the expression above is an `if ... else if ... else`, when `../../hardware-configuration.nix` exists, `./hardware.local.nix` is **not** imported alongside it unless imported separately via `++ lib.optional (builtins.pathExists ./hardware.local.nix) ./hardware.local.nix`. Any host-specific hardware override (such as `boot.loader.efi.canTouchEfiVariables = lib.mkForce false;`) must be injected directly into the active `hardware-configuration.nix` (and mirrored to `hardware.local.nix`).
2. **Compatibility with Both `path:` and `git+file://`**:
   - During installation, invoke `nixos-install --flake path:/mnt/etc/nixos#nas --no-root-password --impure` (or `/mnt/etc/nixos#nas`).
   - Because post-install tools (`nixos-rebuild switch --flake /etc/nixos#nas` or `nh os switch`) use the implicit `git+file://` scheme when `.git` is present, register ONLY the host-local files in the local Git index (`git -C /mnt/etc/nixos add -f hardware-configuration.nix hosts/nas/hardware.local.nix vars.nix vars.local.nix`) **without touching `hosts/nas/hardware-configuration.nix`**.

---

### 3. NixOS Module System & Priority Hierarchy
When authoring NixOS modules (`modules/**/*.nix`), enforce strict separation between **options** (interface) and **config** (implementation):

| Priority | Helper Function | Numeric Priority | Use Case |
| :--- | :--- | :--- | :--- |
| **Option Default** | `mkOption { default = ...; }` | `1500` | Base fallback defined in `modules/options.nix`. |
| **Module Default** | `lib.mkDefault value` | `1000` | Safe default in `hardware-configuration.nix` or reusable profiles that user config can override without conflict. |
| **Standard Definition** | `option = value;` | `100` | Normal assignment in `configuration.nix` or `vars.nix` mapping. |
| **Forced Override** | `lib.mkForce value` | `50` | Hard override (e.g., disabling `services.getty.autologinUser = lib.mkForce null;` or `canTouchEfiVariables = lib.mkForce false;` when NVRAM is locked). |

---

### 4. Live ISO & Installer Engineering (`installation-cd-minimal.nix`)
1. **Block Device & Partition Table Synchronization**:
   - Linux returns `EBUSY` on `BLKRRPART` if any partition on the target disk is mounted, held by swap/LVM/mdadm/ZFS/loop, or temporarily opened by `systemd-udevd` workers.
   - Always unmount across all mount namespaces (`/proc/[0-9]*/mountinfo`), disable swaps, stop `md`/`dm` holders, freeze the udev execution queue (`udevadm control --stop-exec-queue`) during `wipefs` + `sgdisk`, synchronize kernel partitions via `BLKPG` ioctls (`BLKPG_DEL_PARTITION`, `BLKPG_RESIZE_PARTITION`, `BLKPG_ADD_PARTITION`), and resume udev (`udevadm control --start-exec-queue && udevadm trigger --subsystem-match=block --action=add && udevadm settle`).
2. **UEFI vs Legacy BIOS / Locked NVRAM Resilience**:
   - Check `/sys/firmware/efi` before running `nixos-install`.
   - If `/sys/firmware/efi` is absent or `bootctl` fails to write EFI NVRAM variables, inject `boot.loader.efi.canTouchEfiVariables = lib.mkForce false;` into `/mnt/etc/nixos/hardware-configuration.nix` so `systemd-boot` installs the standard UEFI fallback binary (`/boot/EFI/BOOT/BOOTX64.EFI`) without failing on NVRAM writes.
3. **Binary Caches & Low-Power Hardware Protection**:
   - Configure `nix.settings.substituters` (`https://cache.nixos.org`, `https://nix-community.cachix.org`) and `nix.settings.auto-optimise-store = true` to avoid compiling heavy derivations from source on NAS CPUs.

---

<a name="bahasa-indonesia"></a>
## Bahasa Indonesia

### Integrasi Orkestrasi
Terhubung dan mengorkestrasi dengan `rust-programming-expert`, `ci-cd-devops-architect`, `brainstorming`, `zero-to-prod-orchestrator`, dan `production-ready-hardener` untuk merancang, membangun, menginstal, dan memelihara sistem NixOS yang dapat direproduksi, Live ISO, serta server NAS.

### 1. Aturan Emas `hardware-configuration.nix` (Isolasi Perangkat Keras Host)
**JANGAN PERNAH memperlakukan `hardware-configuration.nix` sebagai file statis yang dibagi antar-mesin.** Setiap mesin fisik maupun virtual memiliki topologi perangkat keras yang unik:
- **CPU & Microcode**: Intel (`hardware.cpu.intel.updateMicrocode`) vs AMD (`hardware.cpu.amd.updateMicrocode`).
- **Modul Kernel Stage-1 Initrd**: `boot.initrd.availableKernelModules` (`nvme`, `ahci`, `xhci_pci`, `virtio_pci`, `sd_mod`) dipindai langsung dari `/sys` pada mesin target oleh `nixos-generate-config --root /mnt`.
- **UUID Partisi & Subvolume**: `fileSystems."/"` dan `fileSystems."/boot"` wajib menggunakan `/dev/disk/by-uuid/<UUID>` hasil pemformatan di mesin target.
- **Jangan Menimpa Fallback Git**: Biarkan `hosts/nas/hardware-configuration.nix` tetap murni sesuai repositori Git; simpan hasil `nixos-generate-config --root /mnt` hanya di `/mnt/etc/nixos/hardware-configuration.nix` (serta cadangan lokal `hosts/nas/hardware.local.nix` dan `.hardware-configuration.nix.backup`).

### 2. Semantik Nix Flakes (`git+file://` vs `path:`)
- Saat direktori memiliki `.git`, Nix menggunakan `git+file://` secara default dan mengabaikan file di `.gitignore` yang tidak terdaftar di index Git.
- Gunakan `nixos-generate-config --root /mnt` setelah repositori disalin ke `/mnt/etc/nixos`, daftarkan file lokal host (`hardware-configuration.nix`, `vars.local.nix`) ke index lokal tanpa mengubah `hosts/nas/hardware-configuration.nix`, dan dukung evaluasi `path:/mnt/etc/nixos#nas`.

---

<a name="francais"></a>
## Français — Architecture NixOS & Conventions Noos NAS

### 1. Génération hôte de `hardware-configuration.nix`
- Lors de l'installation d'une machine NixOS, `hardware-configuration.nix` **doit toujours être généré dynamiquement par l'hôte cible** via `nixos-generate-config --root /mnt` (ou `nixos-generate-config --root /mnt --show-hardware-config`) une fois les partitions formatées et montées sous `/mnt` (`/mnt`, `/mnt/boot`, et les sous-volumes Btrfs `@`, `@home`, `@nix`, `@snapshots`).
- **Séparation stricte** :
  - `/mnt/etc/nixos/hardware-configuration.nix` : Configuration matérielle **propre à la machine hôte** (modules noyau `initrd`, microcode CPU Intel/AMD, UUIDs réels des partitions et sous-volumes), générée par `nixos-generate-config --root /mnt`.
  - `/mnt/etc/nixos/hosts/nas/hardware-configuration.nix` : Fichier générique de repli suivi par Git dans le dépôt `Chomiam/noos-nas`. **L'installateur ne doit JAMAIS l'écraser.**
  - `/mnt/etc/nixos/hosts/nas/hardware.local.nix` & `/mnt/etc/nixos/.hardware-configuration.nix.backup` : Sauvegardes locales de la configuration matérielle de l'hôte (ignorées par Git en amont et préservées lors des mises à jour du Dashboard).
