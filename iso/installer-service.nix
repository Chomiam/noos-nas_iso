{ config, pkgs, lib, ... }:

let
  steveos-web-installer = pkgs.rustPlatform.buildRustPackage {
    pname = "steveos-web-installer";
    version = "0.1.0";
    src = ../web-installer;
    cargoLock = {
      lockFile = ../web-installer/Cargo.lock;
    };
  };

  steveos-web-installer-wrapper = pkgs.writeShellScriptBin "steveos-web-installer-wrapper" ''
    if [ -x "/run/current-web-installer/bin/steveos-web-installer" ]; then
      echo "[steveos-installer] Démarrage de la version mise à jour (/run/current-web-installer)..."
      exec /run/current-web-installer/bin/steveos-web-installer
    else
      echo "[steveos-installer] Démarrage de la version embarquée dans l'ISO..."
      exec ${steveos-web-installer}/bin/steveos-web-installer
    fi
  '';
in
{
  environment.systemPackages = [
    steveos-web-installer
    steveos-web-installer-wrapper
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

  systemd.services.steveos-web-installer = {
    description = "STEvE_OS NAS Web Installer Daemon";
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
      ExecStart = "${steveos-web-installer-wrapper}/bin/steveos-web-installer-wrapper";
      Restart = "always";
      RestartSec = 3;
      StandardOutput = "journal";
      StandardError = "journal";
    };
  };
}
