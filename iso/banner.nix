{ config, pkgs, lib, ... }:

{
  systemd.services.steveos-banner = {
    description = "STEvE_OS NAS Edition - Bannière Console d'Installation";
    after = [ "network-online.target" "steveos-web-installer.service" ];
    wants = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      StandardOutput = "tty";
      TTYPath = "/dev/tty1";
      TTYReset = false;
      TTYVHangup = false;
      RemainAfterExit = true;
    };
    script = ''
      IP=$(${pkgs.hostname}/bin/hostname -I 2>/dev/null | ${pkgs.gawk}/bin/awk '{print $1}')
      if [ -z "$IP" ]; then
        IP="127.0.0.1"
      fi

      cat << 'BANNER_EOF' > /dev/tty1

  [1;35m╔══════════════════════════════════════════════════════════════════════════════╗[0m
  [1;35m║[0m               [1;37m🚀 STEvE_OS NAS Edition — Installateur Réseau[0m                  [1;35m║[0m
  [1;35m╚══════════════════════════════════════════════════════════════════════════════╝[0m

    [1;32m●[0m Le système est démarré et prêt pour l'installation.
    [1;36m●[0m L'installateur Web est accessible sur votre réseau local :

BANNER_EOF

      echo -e "        \033[1;33m👉  http://$IP:8080\033[0m\n" > /dev/tty1

      cat << 'BANNER_EOF2' > /dev/tty1
    [1;37mPour installer STEvE_OS NAS :[0m
    Ouvrez un navigateur sur un autre ordinateur (PC, Mac, smartphone)
    connecté à votre box ou réseau local, puis saisissez l'adresse ci-dessus.

  [1;35m──────────────────────────────────────────────────────────────────────────────[0m
    [0;90mConsole locale de secours (root sans mot de passe). Port SSH actif sur 22.[0m

BANNER_EOF2
    '';
  };
}
