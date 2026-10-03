{ config, pkgs, lib, ... }:

let
  noos-web-installer = pkgs.rustPlatform.buildRustPackage {
    pname = "noos-web-installer";
    version = "0.2.1";
    src = ../web-installer;
    cargoLock = {
      lockFile = ../web-installer/Cargo.lock;
    };
  };

  noos-web-installer-wrapper = pkgs.writeShellScriptBin "noos-web-installer-wrapper" ''
    if [ -x "/run/current-web-installer/bin/noos-web-installer" ]; then
      echo "[steveos-installer] Démarrage de la version mise à jour (/run/current-web-installer)..."
      exec /run/current-web-installer/bin/noos-web-installer
    else
      echo "[steveos-installer] Démarrage de la version embarquée dans l'ISO..."
      exec ${noos-web-installer}/bin/noos-web-installer
    fi
  '';
in
{
  environment.systemPackages = [
    noos-web-installer
    noos-web-installer-wrapper
    pkgs.git
    pkgs.btrfs-progs
    pkgs.e2fsprogs
    pkgs.dosfstools
    pkgs.gptfdisk
    pkgs.util-linux
    pkgs.openssl
    pkgs.nixos-install-tools
    pkgs.nettools
    pkgs.hostname
    pkgs.nix
  ];

  networking.firewall.allowedTCPPorts = [ 8080 22 ];

  systemd.services.noos-web-installer = {
    description = "Noos NAS Web Installer Daemon";
    aliases = [ "steveos-web-installer.service" ];
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];
    path = with pkgs; [
      util-linux
      gptfdisk
      dosfstools
      btrfs-progs
      e2fsprogs
      git
      openssl
      nixos-install-tools
      systemd
      coreutils
      bash
      nettools
      hostname
      nix
    ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "${noos-web-installer-wrapper}/bin/noos-web-installer-wrapper";
      Restart = "always";
      RestartSec = 3;
      StandardOutput = "journal";
      StandardError = "journal";
    };
  };
}
