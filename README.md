# 🚀 Noos NAS Edition — Image ISO & Installateur Réseau

Image ISO minimale bootable pour l'installation réseau automatisée de **Noos NAS Edition**.

---

## ✨ Fonctionnalités clés

- **Boot Console (TTY)** : Démarrage direct en mode texte ultra-léger sans environnement de bureau lourd sur la machine cible.
- **Bannière TTY intelligente** : Détecte l'adresse IP locale assignée via DHCP et l'affiche sur `/dev/tty1` avec l'URL de connexion.
- **Installateur Web Réseau (Port 8080)** : Interface moderne Catppuccin Mocha accessible depuis n'importe quel ordinateur ou smartphone connecté au même réseau.
- **Vérification automatique des mises à jour (Auto-Update)** :
  - Dès l'ouverture de l'installateur, le système compare le commit local de l'ISO avec la dernière version sur GitHub (`Chomiam/noos-nas_iso`).
  - Si une mise à jour existe, elle est compilée et le service redémarre instantanément sans intervention manuelle.
- **Sélection et formatage du disque système** :
  - Détection automatique avec modèle, capacité et bus (`NVMe`, `SATA`, etc.).
  - Protection et exclusion automatique de la clé USB d'installation.
  - Choix entre **Btrfs** (avec sous-volumes `@`, `@home`, `@nix`, `@snapshots`) et **Ext4**.
- **Validation stricte de l'administrateur** : Respect rigoureux des standards POSIX (`^[a-z_][a-z0-9_-]{1,31}$`, exclusion des espaces, majuscules et noms réservés).
- **Terminal de logs stylisé en direct (SSE)** : Affichage ligne par ligne du partitionnement, du formatage et de `nixos-install`.
- **Note sur les pools de stockage** : Rappel clair que la création des volumes RAID pour les données se fait post-installation sur le tableau de bord web.
- **Fin d'installation & Transition** : Message chaleureux et compte à rebours avant redémarrage vers le tableau de bord (port `9339`).

---

## 🛠️ Génération de l'ISO

Pour compiler l'image ISO :
```bash
nix build .#iso
```
L'image résultante sera générée dans `result/iso/nixos-minimal-*.iso`.

Un lien pratique `noos-nas-installer.iso` pointe directement vers le dernier fichier généré.

---

## 💾 Gravure sur Clé USB

### Sous Linux (via `dd`) :
```bash
sudo dd if=noos-nas-installer.iso of=/dev/sdX bs=4M status=progress conv=fsync
```
*(Remplacez `/dev/sdX` par le périphérique de votre clé USB)*

### Via un utilitaire graphique :
Vous pouvez utiliser **BalenaEtcher**, **Raspberry Pi Imager** ou **Ventoy**.

---

## 🌐 Utilisation

1. Démarrez le NAS sur la clé USB (en mode UEFI).
2. L'écran de la console (TTY) affichera :
   ```text
   👉 http://<IP_DU_NAS>:8080
   ```
3. Ouvrez cette adresse dans votre navigateur depuis votre ordinateur personnel.
4. Remplissez le formulaire, lancez l'installation et suivez les logs en direct !
