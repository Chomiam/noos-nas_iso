{ config, pkgs, lib, ... }:

let
  bannerScript = pkgs.writeShellScript "steveos-banner-loop" ''
    LAST_IP=""

    get_lan_ip() {
      # 1. Via route par défaut vers l'extérieur (interroge la table de routage du noyau)
      local ip=$(${pkgs.iproute2}/bin/ip -4 route get 1.1.1.1 2>/dev/null | ${pkgs.gawk}/bin/awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}')
      if [ -n "$ip" ] && [[ "$ip" != 127.* ]] && [[ "$ip" != 169.254.* ]]; then
        echo "$ip"
        return
      fi

      # 2. Via première adresse IPv4 avec scope global
      ip=$(${pkgs.iproute2}/bin/ip -4 -o addr show scope global 2>/dev/null | ${pkgs.gawk}/bin/awk '{split($4, a, "/"); print a[1]}' | ${pkgs.gnugrep}/bin/grep -v '^127\.' | ${pkgs.gnugrep}/bin/grep -v '^169\.254\.' | head -n1)
      if [ -n "$ip" ]; then
        echo "$ip"
        return
      fi

      # 3. Via hostname -I en filtrant loopback et lien local
      for candidate in $(${pkgs.hostname}/bin/hostname -I 2>/dev/null); do
        if [[ "$candidate" != 127.* ]] && [[ "$candidate" != 169.254.* ]]; then
          echo "$candidate"
          return
        fi
      done

      echo ""
    }

    # Boucle dynamique de surveillance réseau et d'affichage
    while true; do
      IP=$(get_lan_ip)

      if [ -n "$IP" ]; then
        if [ "$IP" != "$LAST_IP" ]; then
          LAST_IP="$IP"

          # Mise à jour de /etc/issue pour les consoles TTY
          cat << ISSUE_EOF > /etc/issue

  \e[1;35m╔══════════════════════════════════════════════════════════════════════════════╗\e[0m
  \e[1;35m║\e[0m                \e[1;37m🚀 STEvE_OS NAS Edition — Installateur Réseau\e[0m                 \e[1;35m║\e[0m
  \e[1;35m╚══════════════════════════════════════════════════════════════════════════════╝\e[0m

    \e[1;32m●\e[0m Adresse IP réseau locale : \e[1;37m$IP\e[0m
    \e[1;36m●\e[0m Interface web d'installation :

        \e[1;33m👉  http://$IP:8080\e[0m

    Ouvrez ce lien depuis un autre PC connecté au même réseau local.
  \e[1;35m──────────────────────────────────────────────────────────────────────────────\e[0m
    \e[0;90mConsole locale de secours (root sans mot de passe). Port SSH actif sur 22.\e[0m

ISSUE_EOF

          # Affichage immédiat sur /dev/tty1
          echo -e "\n\033[1;32m✔ Adresse IP obtenue : http://$IP:8080\033[0m\n" > /dev/tty1 2>/dev/null || true
          cat /etc/issue > /dev/tty1 2>/dev/null || true
        fi
      else
        if [ "$LAST_IP" != "waiting" ]; then
          LAST_IP="waiting"
          cat << 'WAIT_EOF' > /dev/tty1 2>/dev/null || true

================================================================================
  [!] STEvE_OS NAS Edition : En attente d'une adresse IP réseau (DHCP)...
      - Branchez un câble Ethernet à votre box ou switch.
      - Si votre NAS possède 2 ports Ethernet, essayez le second port.
================================================================================

WAIT_EOF
        fi
      fi

      sleep 2
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
