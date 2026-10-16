{ config, pkgs, lib, self ? null, ... }:

let
  noos-grub-theme = pkgs.callPackage ./theme { };
in
{
  imports = [
    ./banner.nix
    ./installer-service.nix
  ];

  # =========================================================================
  # 🎨 BRANDING DE L'IMAGE ISO : NOOS NAS EDITION
  # =========================================================================

  # Remplacement des mentions du système d'exploitation par Noos
  system.nixos.distroName = "Noos";
  system.nixos.distroId = "noos";
  system.nixos.label = "";
  system.nixos.vendorName = "Noos Project";
  system.nixos.vendorId = "noos";
  system.nixos.variantName = "NAS Edition";

  # Libellé du menu de boot : "Noos nas edition installer"
  isoImage.prependToMenuLabel = "";
  isoImage.appendToMenuLabel = "nas edition installer";

  # Thème de Bootloader GRUB Catppuccin Mocha personnalisé avec logo Noos
  isoImage.grubTheme = noos-grub-theme;

  # Fond d'écran de démarrage BIOS (Syslinux/Isolinux) Catppuccin Mocha
  isoImage.splashImage = ./theme/assets/bios-background.png;
  isoImage.efiSplashImage = ./theme/assets/background.png;

  # Empreinte du commit ISO pour vérification des mises à jour en ligne
  environment.etc."noos-iso-commit".text =
    if self != null && self ? rev then self.rev
    else if self != null && self ? dirtyRev then self.dirtyRev
    else "b870c77b9c614da03252827c616159db287167ef";

  # Paramètres de l'image ISO
  image.fileName = lib.mkForce "noos-nas-installer.iso";
  isoImage.volumeID = lib.mkForce "NOOS_NAS";
  isoImage.makeEfiBootable = true;
  isoImage.makeUsbBootable = true;
  boot.zfs.forceImportRoot = false;

  # =========================================================================
  # 🔧 COMPATIBILITÉ BOOT USB UNIVERSELLE (Rufus ISO FAT32/exFAT/NTFS & DD ISO9660)
  # =========================================================================
  # En NixOS 26.05 (commit nixpkgs f8ed5f30), `iso-image.nix` force `fsType = "iso9660"`
  # sur `/iso` via `lib.mkImageMediaOverride`. Lorsqu'une clé USB est préparée avec Rufus
  # sous Windows en mode "Image ISO", Rufus crée une table MBR/GPT et formate la partition
  # en FAT32 (`vfat`), exFAT ou NTFS : `mount -t iso9660` échoue alors systématiquement avec
  # "Failed to mount /sysroot/iso" et "Dependency failed for Copy ISO contents to RAM".
  # De plus, si la clé avait précédemment été flashée en mode DD/Etcher, un ancien superbloc
  # ISO9660 résiduel à l'offset 0x8000 du disque entier (`/dev/sdX`) peut entrer en conflit
  # avec la nouvelle partition FAT32 (`/dev/sdX1`) lors de la détection udev par label.
  fileSystems = lib.mkImageMediaOverride {
    "/iso" = lib.mkImageMediaOverride {
      fsType = lib.mkForce "auto";
    };
  };

  boot.initrd.availableKernelModules = [
    "vfat" "fat" "nls_cp437" "nls_iso8859-1" "exfat" "ntfs3"
    "iso9660" "squashfs" "loop" "overlay"
    "usb_storage" "sd_mod" "sr_mod"
    "xhci_pci" "ehci_pci" "ohci_pci" "uhci_hcd"
    "nvme" "ahci" "mmc_block" "rtsx_pci_sdmmc" "rtsx_usb_sdmmc"
  ];

  boot.initrd.systemd.contents."/etc/filesystems".text = ''
    vfat
    iso9660
    exfat
    ntfs3
  '';

  boot.initrd.systemd.services.find-noos-iso = {
    description = "Detect and configure Noos NAS installer medium (FAT32 / exFAT / NTFS / ISO9660)";
    wantedBy = [ "initrd.target" ];
    before = [
      "sysroot-iso.mount"
      "copytoram.service"
      "initrd-switch-root.target"
    ];
    after = [
      "systemd-udevd.service"
      "systemd-modules-load.service"
      "sysroot.mount"
    ];
    unitConfig = {
      DefaultDependencies = false;
      RequiresMountsFor = "/sysroot";
    };
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    path = [
      pkgs.coreutils
      config.boot.initrd.systemd.package
      config.boot.initrd.systemd.package.util-linux
      config.boot.initrd.systemd.package.kmod
    ];
    script = ''
      set +e
      for mod in vfat fat nls_cp437 nls_iso8859-1 exfat ntfs3 iso9660 squashfs loop usb_storage sd_mod sr_mod; do
        modprobe "$mod" 2>/dev/null || true
      done

      mkdir -p /tmp-iso-probe
      found_dev=""
      found_fstype=""

      for attempt in $(seq 1 15); do
        udevadm settle --timeout=3 2>/dev/null || true

        # Tester les partitions (/dev/sdX1, /dev/nvme0n1p1...) EN PREMIER avant les disques
        # entiers (/dev/sdX, /dev/sr0) pour éviter tout faux positif sur un ancien superbloc
        # ISO9660 résiduel à l'offset 32768 d'une clé reformatée en FAT32 par Rufus.
        candidates=""
        for sysdev in /sys/class/block/*; do
          [ -e "$sysdev" ] || continue
          devname=$(basename "$sysdev")
          case "$devname" in
            loop*|ram*|dm-*|md*|zram*) continue ;;
          esac
          if [ -f "$sysdev/partition" ]; then
            candidates="$candidates /dev/$devname"
          fi
        done
        for sysdev in /sys/class/block/*; do
          [ -e "$sysdev" ] || continue
          devname=$(basename "$sysdev")
          case "$devname" in
            loop*|ram*|dm-*|md*|zram*) continue ;;
          esac
          if [ ! -f "$sysdev/partition" ]; then
            candidates="$candidates /dev/$devname"
          fi
        done

        for dev in $candidates; do
          [ -b "$dev" ] || continue
          for fstype in vfat iso9660 exfat ntfs3 auto; do
            if mount -t "$fstype" -o ro "$dev" /tmp-iso-probe 2>/dev/null; then
              if [ -f /tmp-iso-probe/nix-store.squashfs ]; then
                found_dev="$dev"
                if [ "$fstype" = "auto" ]; then
                  found_fstype=$(findmnt -n -o FSTYPE --target /tmp-iso-probe 2>/dev/null || echo "auto")
                else
                  found_fstype="$fstype"
                fi
                umount /tmp-iso-probe 2>/dev/null || true
                break 3
              fi
              umount /tmp-iso-probe 2>/dev/null || true
            fi
          done
        done

        sleep 1
      done
      rmdir /tmp-iso-probe 2>/dev/null || true

      if [ -n "$found_dev" ]; then
        dev_base=$(basename "$found_dev")
        echo "[noos-iso] Support d'installation Noos detecte sur $found_dev (type: $found_fstype)"

        # 1. Forcer le lien udev /dev/disk/by-label/NOOS_NAS vers la bonne partition
        #    et retirer le label d'un éventuel disque parent contenant un ancien header ISO9660
        mkdir -p /run/udev/rules.d /dev/disk/by-label
        cat > /run/udev/rules.d/61-noos-iso-label.rules <<EOF
ACTION!="remove", SUBSYSTEM=="block", KERNEL!="$dev_base", ENV{ID_FS_LABEL_ENC}=="NOOS_NAS", SYMLINK=""
ACTION!="remove", SUBSYSTEM=="block", KERNEL=="$dev_base", ENV{ID_FS_USAGE}="filesystem", ENV{ID_FS_LABEL}="NOOS_NAS", ENV{ID_FS_LABEL_ENC}="NOOS_NAS", SYMLINK+="disk/by-label/NOOS_NAS", TAG+="systemd"
EOF
        ln -sf "../../$dev_base" /dev/disk/by-label/NOOS_NAS
        udevadm control --reload 2>/dev/null || true
        udevadm trigger --action=change --subsystem-match=block 2>/dev/null || true
        udevadm settle --timeout=5 2>/dev/null || true
        ln -sf "../../$dev_base" /dev/disk/by-label/NOOS_NAS

        # 2. Surcharger dynamiquement sysroot-iso.mount avec le périphérique et le fstype exacts
        mkdir -p /run/systemd/system/sysroot-iso.mount.d
        cat > /run/systemd/system/sysroot-iso.mount.d/override.conf <<EOF
[Mount]
What=$found_dev
Type=$found_fstype
EOF
        systemctl daemon-reload 2>/dev/null || true

        # 3. Monter directement /sysroot/iso pour satisfaire immédiatement sysroot-iso.mount
        mkdir -p /sysroot/iso
        if ! mountpoint -q /sysroot/iso 2>/dev/null; then
          mount -t "$found_fstype" -o ro "$found_dev" /sysroot/iso 2>/dev/null || true
        fi
      else
        echo "[noos-iso] Attention: aucun périphérique contenant /nix-store.squashfs n'a été trouvé"
      fi
      exit 0
    '';
  };

  # Empêcher l'extinction / mise en veille de l'écran console (DPMS & consoleblank)
  boot.kernelParams = [ "consoleblank=0" ];

  # Pilotes réseau Realtek 2.5GbE (RTL8125), Intel (igc, e1000e) et firmwares
  boot.kernelModules = [ "r8169" "igc" "e1000e" ];
  hardware.enableRedistributableFirmware = true;

  # Réseau : DHCP automatique sur TOUTES les interfaces via dhcpcd
  networking.hostName = "noos-installer";
  networking.networkmanager.enable = lib.mkForce false;
  networking.useDHCP = lib.mkForce true;
  networking.dhcpcd.enable = true;

  # Suppression des bannières encombrantes par défaut de NixOS
  services.getty.helpLine = lib.mkForce "";
  services.getty.greetingLine = lib.mkForce "";

  # Autoriser root sans mot de passe en console pour dépannage éventuel
  users.users.root.initialHashedPassword = "";

  # Timezone & Locale
  time.timeZone = "Europe/Paris";
  i18n.defaultLocale = "fr_FR.UTF-8";
  console.keyMap = "fr";

  # SSH pour accès de secours à distance
  services.openssh = {
    enable = true;
    settings.PermitRootLogin = "yes";
  };

  # Activer les flakes dans l'environnement d'installation
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
}
