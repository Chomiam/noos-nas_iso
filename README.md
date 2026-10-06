<div align="center">
  <img src="assets/logo.png" alt="Noos NAS Logo" width="500"/>
  <br/><br/>

  # 💿 Noos NAS Web Installer
  ### *L'Installation Dématérialisée et Intelligente de Votre Serveur NAS en 5 Minutes Chrono*

  [![Boot](https://img.shields.io/badge/D%C3%A9marrage-UEFI%20%2F%20BIOS%20Universel-blue?style=for-the-badge&logo=linux&logoColor=white)](https://github.com/Chomiam/noos-nas_iso)
  [![Installer](https://img.shields.io/badge/Interface%20Web-Port%208080%20(Catppuccin)-magenta?style=for-the-badge)](https://github.com/Chomiam/noos-nas_iso)
  [![Auto-Update](https://img.shields.io/badge/Mises%20%C3%A0%20Jour-Auto--Update%20Temps%20R%C3%A9el-brightgreen?style=for-the-badge)](#)
  [![Storage](https://img.shields.io/badge/Syst%C3%A8me%20de%20Fichiers-Btrfs%20%7C%20Ext4-orange?style=for-the-badge)](#)
  [![Safety](https://img.shields.io/badge/S%C3%A9curit%C3%A9-Protection%20Disques%20Donn%C3%A9es-teal?style=for-the-badge)](#)

  <p align="center">
    <strong>Transformez n'importe quel vieil ordinateur ou serveur dédié en un NAS souverain ultra-performant. Zéro commande complexe, zéro stress, 100% guidé depuis votre navigateur.</strong>
  </p>
</div>

---

## 🌟 L'Installation Simplifiée à l'Extrême : Comment Ça Marche ?

Installer un système d'exploitation pour serveur de stockage est souvent réputé intimidant. **Noos NAS Edition fait table rase de la complexité technique** grâce à son image d'installation minimale avec installateur web déporté :

1. **Branchez et Allumez** : Vous n'avez même pas besoin de connecter un écran ou un clavier permanent sur votre NAS. Connectez simplement une clé USB d'installation et un câble Ethernet.
2. **Accédez depuis votre Smartphone ou PC** : L'installateur démarre un serveur web léger et convivial sur le port `8080`. Ouvrez l'adresse dans votre navigateur web préféré (ex: `http://192.168.1.50:8080`).
3. **Mise à Jour Automatique (Zero-Day Update)** : Dès le démarrage, l'installateur vérifie s'il existe une révision plus récente du code sur GitHub et applique les derniers correctifs automatiquement avant même que vous ne cliquiez sur Installer.
4. **Protection Intelligente de Vos Disques** : L'installateur cible exclusivement le disque que vous choisissez pour le système d'exploitation (`/`), tout en protégeant automatiquement votre clé USB d'installation. Les disques réservés au stockage de vos données personnelles restent intacts pour être regroupés en grappes RAID post-installation sur le tableau de bord.
5. **Formatage Btrfs Optimisé par Défaut** : Création déclarative des sous-volumes haute performance (`@`, `@home`, `@nix`, `@snapshots`) garantissant des sauvegardes instantanées et une auto-réparation des fichiers corrompus.

---

## 🚀 Guide Pas-à-Pas en 3 Étapes

### Étape 1 : Télécharger et Graver l'Image ISO

Téléchargez la dernière image officielle `noos-nas-installer.iso` depuis la section [Releases](https://github.com/Chomiam/noos-nas_iso/releases).

Vous pouvez flasher l'image en quelques secondes sur une clé USB (de 4 Go minimum) avec votre utilitaire favori :
- **Graphique (Recommandé)** : [BalenaEtcher](https://etcher.balena.io/), [Rufus](https://rufus.ie/) ou [Ventoy](https://www.ventoy.net/).
- **En ligne de commande (Linux/macOS)** :
  ```bash
  sudo dd if=noos-nas-installer.iso of=/dev/sdX bs=4M status=progress conv=fsync
  ```
  *(Remplacez `/dev/sdX` par le lecteur correspondant à votre clé USB).*

---

### Étape 2 : Démarrer la Machine Cible

- Insérez la clé USB dans votre machine NAS et démarrez-la en sélectionnant la clé dans votre menu de boot UEFI/BIOS.
- L'écran de la machine (ou votre console de gestion) affichera automatiquement une bannière accueillante :
  ```text
  ╔══════════════════════════════════════════════════════════════╗
  ║                🌐 NOOS NAS WEB INSTALLER                     ║
  ║                                                              ║
  ║  👉 Ouvrez votre navigateur sur : http://192.168.1.50:8080   ║
  ╚══════════════════════════════════════════════════════════════╝
  ```

---

### Étape 3 : Suivre l'Assistant Graphique Web

1. Ouvrez l'URL dans votre navigateur web sur votre ordinateur portable ou votre smartphone.
2. Choisissez votre **disque système cible** (NVMe ou SATA SSD).
3. Définissez vos identifiants d'administrateur sécurisés.
4. Cliquez sur **🚀 Démarrer l'Installation**.
5. Suivez le déroulement en direct dans la console stylisée : partitionnement, montage Btrfs, génération matérielle et installation reproductible NixOS.
6. Une fois l'installation terminée, la machine redémarre toute seule et votre [Tableau de Bord Noos NAS](https://github.com/Chomiam/noos-nas-dashboard) est immédiatement prêt à l'emploi sur le **port 9339** !

---

## 🛠️ Compilation Personnalisée de l'ISO (Développeurs)

Pour compiler vous-même l'image ISO bootable via l'environnement déclaratif Nix :

```bash
# Compiler l'image ISO minimale officielle :
nix build .#iso
```

Le fichier ISO bootable résultant sera généré dans `result/iso/` avec un lien pratique `noos-nas-installer.iso`.

---

<div align="center">
  <sub>Fait partie de l'écosystème officiel <a href="https://github.com/Chomiam/noos-nas">Noos NAS Edition</a>.</sub>
</div>
