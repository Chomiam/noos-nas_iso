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

  # Compatibilité Rufus (mode ISO FAT32/NTFS/exFAT + mode DD ISO9660) & contrôleurs USB au stage-1 (initrd)
  # Sans vfat + nls_cp437 + nls_iso8859_1 dans l'initrd, une clé flashée par Rufus en mode "Image ISO"
  # (qui reformate la clé en FAT32/exFAT/NTFS avec le label NOOS_NAS) échoue avec "Failed to mount /sysroot/iso".
  boot.initrd.supportedFilesystems = [ "vfat" "exfat" "ntfs" "iso9660" ];
  boot.initrd.availableKernelModules = [
    "vfat" "fat" "nls_cp437" "nls_iso8859_1" "exfat" "ntfs3"
    "iso9660" "squashfs" "loop" "overlay"
    "uas" "usb_storage" "sd_mod" "sr_mod"
    "xhci_pci" "xhci_hcd" "ehci_pci" "ehci_hcd" "ohci_pci" "uhci_hcd"
    "nvme" "ahci" "mmc_block" "rtsx_pci_sdmmc" "rtsx_usb_sdmmc"
  ];
  boot.initrd.kernelModules = [
    "vfat" "fat" "nls_cp437" "nls_iso8859_1"
    "iso9660" "squashfs" "loop"
    "usb_storage" "uas" "sd_mod" "sr_mod"
  ];

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
