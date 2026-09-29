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
in
{
  environment.systemPackages = [
    steveos-web-installer
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
      ExecStart = "${steveos-web-installer}/bin/steveos-web-installer";
      Restart = "always";
      RestartSec = 3;
      StandardOutput = "journal";
      StandardError = "journal";
    };
  };
}
