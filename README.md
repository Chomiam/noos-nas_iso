# STEvE_OS NAS Edition — Image ISO & Installateur Réseau

Image ISO minimale bootable pour l'installation réseau de **STEvE_OS NAS Edition**.

## 🚀 Fonctionnalités
- **Boot Console (TTY)** : Démarrage rapide et léger sans interface graphique locale.
- **Bannière TTY intelligente** : Affiche l'adresse IP locale et le lien de l'installateur web.
- **Installateur Web Réseau (Port 8080)** : Interface moderne Catppuccin Mocha accessible depuis n'importe quel ordinateur ou tablette du réseau local.
- **Sélection et formatage du disque** : Support de **Btrfs** (avec sous-volumes optimisés) et **Ext4**.
- **Validation stricte de l'utilisateur** : Contrôle des caractères interdits selon les standards POSIX.
- **Terminal de logs en temps réel** : Suivi visuel du partitionnement, du formatage et de l'installation NixOS via Server-Sent Events.

## 🛠️ Génération de l'ISO
```bash
nix build .#iso
```
L'image ISO sera générée dans `result/iso/`.
