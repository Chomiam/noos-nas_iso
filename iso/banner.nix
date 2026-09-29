{ config, pkgs, lib, ... }:

let
  bannerScript = pkgs.writeShellScript "steveos-banner-loop" ''
    # Désactiver la mise en veille de la console TTY1
    ${pkgs.util-linux}/bin/setterm -blank 0 -powerdown 0 > /dev/tty1 2>/dev/null || true

    get_lan_ip() {
      # 1. Via route par défaut vers l'extérieur
      local ip=$(${pkgs.iproute2}/bin/ip -4 route get 1.1.1.1 2>/dev/null | ${pkgs.gawk}/bin/awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}')
      if [ -n "$ip" ] && [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] && [[ "$ip" != 127.* ]] && [[ "$ip" != 169.254.* ]]; then
        echo "$ip"
        return
      fi

      # 2. Via première adresse IPv4 avec scope global
      ip=$(${pkgs.iproute2}/bin/ip -4 -o addr show scope global 2>/dev/null | ${pkgs.gawk}/bin/awk '{split($4, a, "/"); print a[1]}' | ${pkgs.gnugrep}/bin/grep -v '^127\.' | ${pkgs.gnugrep}/bin/grep -v '^169\.254\.' | head -n1)
      if [ -n "$ip" ] && [[ "$ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo "$ip"
        return
      fi

      # 3. Via hostname -I en filtrant strictement IPv4 privé (exclure IPv6)
      for candidate in $(${pkgs.hostname}/bin/hostname -I 2>/dev/null); do
        if [[ "$candidate" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] && [[ "$candidate" != 127.* ]] && [[ "$candidate" != 169.254.* ]]; then
          echo "$candidate"
          return
        fi
      done

      echo ""
    }

    ESC=$(printf '\033')
    RESET="''${ESC}[0m"
    BOLD_PURPLE="''${ESC}[1;35m"
    BOLD_CYAN="''${ESC}[1;36m"
    BOLD_GREEN="''${ESC}[1;32m"
    BOLD_YELLOW="''${ESC}[1;33m"
    BOLD_WHITE="''${ESC}[1;37m"
    DIM="''${ESC}[0;90m"

    LAST_IP=""
    LAST_STATE=""

    # Surveillance de l'IP : n'écrit à l'écran qu'en cas de changement d'état ou d'IP
    while true; do
      IP=$(get_lan_ip)

      if [ -n "$IP" ]; then
        if [ "$IP" != "$LAST_IP" ]; then
          LAST_IP="$IP"
          LAST_STATE="ready"

          # Mise à jour de /etc/issue pour les consoles TTY
          cat << ISSUE_EOF > /etc/issue 2>/dev/null || true

''${BOLD_PURPLE}  ╔══════════════════════════════════════════════════════════════════════════════╗''${RESET}
''${BOLD_PURPLE}  ║''${RESET}                ''${BOLD_WHITE}🚀 STEvE_OS NAS Edition — Installateur Réseau''${RESET}                 ''${BOLD_PURPLE}║''${RESET}
''${BOLD_PURPLE}  ╚══════════════════════════════════════════════════════════════════════════════╝''${RESET}

    ''${BOLD_GREEN}●''${RESET} Adresse IP réseau locale : ''${BOLD_WHITE}$IP''${RESET}
    ''${BOLD_CYAN}●''${RESET} Interface web d'installation :

        ''${BOLD_YELLOW}👉  http://$IP:8080''${RESET}

    Ouvrez ce lien depuis un autre PC connecté au même réseau local.
''${BOLD_PURPLE}  ──────────────────────────────────────────────────────────────────────────────''${RESET}
    ''${DIM}Console de secours active sur TTY2 (Alt+F2) ou SSH sur port 22 (root).''${RESET}

ISSUE_EOF

          # Effacer complètement l'écran (2J), le scrollback (3J), placer le curseur en 1,1 (H) et afficher la bannière
          printf "\033[2J\033[3J\033[H" > /dev/tty1 2>/dev/null || true
          cat /etc/issue > /dev/tty1 2>/dev/null || true
        fi
      else
        if [ "$LAST_STATE" != "waiting" ]; then
          LAST_STATE="waiting"
          LAST_IP=""

          # Nettoyage et affichage du statut d'attente réseau
          printf "\033[2J\033[3J\033[H" > /dev/tty1 2>/dev/null || true
          cat << 'WAIT_EOF' > /dev/tty1 2>/dev/null || true

================================================================================
  [!] STEvE_OS NAS Edition : En attente d'une adresse IP réseau (DHCP)...
      - Branchez un câble Ethernet à votre box ou switch.
      - Si votre NAS possède 2 ports Ethernet, essayez le second port.
================================================================================

WAIT_EOF
        fi
      fi

      sleep 4
    done
  '';
in
{
  systemd.services.steveos-banner = {
    description = "STEvE_OS NAS Edition - Bannière Console Dynamique";
    after = [ "network.target" "steveos-web-installer.service" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "${bannerScript}";
      Restart = "always";
      RestartSec = 2;
      StandardOutput = "null";
      StandardError = "journal";
    };
  };
}
