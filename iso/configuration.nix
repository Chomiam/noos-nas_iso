{ config, pkgs, lib, self ? null, ... }:

{
  imports = [
    ./banner.nix
    ./installer-service.nix
  ];

  # Empreinte du commit ISO pour vérification des mises à jour en ligne
  environment.etc."steveos-iso-commit".text =
    if self != null && self ? rev then self.rev
    else if self != null && self ? dirtyRev then self.dirtyRev
    else "b870c77b9c614da03252827c616159db287167ef";

  # Paramètres de l'image ISO
  image.fileName = lib.mkForce "steveos-nas-installer.iso";
  isoImage.volumeID = lib.mkForce "STEVEOS_NAS";
  isoImage.makeEfiBootable = true;
  isoImage.makeUsbBootable = true;
  boot.zfs.forceImportRoot = false;

  # Réseau
  networking.hostName = "steveos-installer";
  networking.useDHCP = lib.mkDefault true;

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
