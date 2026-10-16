---
name: steveos-knowledge
description: >-
  Base de connaissances, retours d'expérience (REX), bonnes pratiques et pièges critiques
  à éviter sur l'écosystème STEvE_OS (steveos-nas, steveos-nas-dashboard, steve_nas_eggs).
  Activer ce skill pour toute tâche d'architecture, de débogage, de build, de mise à jour,
  de conteneurisation Docker/Eggs ou d'intégration NixOS.
---

# 🧠 Base de Connaissances & Retours d'Expérience (REX) — Écosystème STEvE_OS

Ce guide regroupe les apprentissages essentiels, l'architecture des dépôts, les bonnes pratiques de développement et la liste des erreurs critiques à ne jamais reproduire sur les projets liés à **STEvE_OS**.

---

## 🏛️ 1. Architecture des Composants STEvE_OS

| Dépôt / Projet | Rôle & Technologies | Emplacement local |
| :--- | :--- | :--- |
| **`steveos-nas`** (`steve_os-nix`) | Configuration déclarative NixOS du NAS (Modules, Flake, services système, virtualisation, réseau, stockage ZFS/Btrfs). | `/home/chomiam/Projects/steveos-nas` |
| **`steveos-nas-dashboard`** | Tableau de bord Web réactif & API haute performance (Backend Rust Axum, Frontend Vanilla JS/CSS Catppuccin Mocha). | `/home/chomiam/Projects/steveos-nas-dashboard` |
| **`steveos_nas_store`** | Catalogue et templates Docker Compose officiels pour l'App Store du NAS (Nextcloud, Jellyfin, AdGuard Home, Vaultwarden, etc.). | `/home/chomiam/Projects/steveos_nas_store` (GitHub: `Chomiam/steveos_nas_store`) |
| **`steve_nas_eggs`** | Catalogue d'Eggs de serveurs de jeux conteneurisés Docker (Minecraft, Palworld, Valheim, Terraria, CS2, Enshrouded, etc.). | Dépôt GitHub officiel `Chomiam/steve_nas_eggs` |

### Principes d'intégration :
- `steveos-nas` intègre `steveos-nas-dashboard` via une entrée Flake (`flake.nix` / `flake.lock`).
- `steveos-nas-dashboard` interroge dynamiquement le dépôt distant `steveos_nas_store` pour l'installation en 1 clic des applications Docker.
- Les binaires du Dashboard sont construits par **GitHub Actions** et hébergés sur le cache binaire Cachix officiel `steveos` (`https://steveos.cachix.org`).
- Le Dashboard communique en local avec Docker, systemd, ZFS, WireGuard et NixOS via des commandes CLI isolées et des sockets IPC.

---

## ⚠️ 2. Pièges Critiques & Erreurs à Ne Jamais Reproduire (Anti-Patterns)

### A. Réseau & API
1. **Pas de `git ls-remote` synchrone sur de gros dépôts (`nixpkgs`) :**
   - *Erreur passée* : Exécuter `git ls-remote https://github.com/nixos/nixpkgs` lors de chaque vérification de mise à jour.
   - *Impact* : Blocage du thread HTTP serveur pendant 20 à 30 secondes, interface web figée sur `Vérification de l'état du système...` avec des tirets `--`.
   - *Règle* : `nixpkgs` est géré et verrouillé par `flake.lock` dans `steve_os-nix`. Ne jamais sonder `nixpkgs` à distance via `git ls-remote`.
2. **Double comptage des composants internes :**
   - *Erreur passée* : Détecter une mise à jour du Dashboard dans sa télémétrie dédiée ET le compter une seconde fois comme paquet système tiers dans `package_updates_count`.
   - *Règle* : Exclure systématiquement les composants internes suivis (`steveos-nas-dashboard`) du balayage générique des paquets.

### B. Frontend JavaScript & Résilience UI
3. **Préférer `Promise.allSettled` à `Promise.all` pour les rafraîchissements globaux :**
   - *Erreur passée* : Utiliser `Promise.all` dans `refreshAll()`.
   - *Impact* : Si une seule requête échouait (ex: réseau indisponible ou module tiers désactivé), l'ensemble du rafraîchissement du Dashboard était avorté.
   - *Règle* : Toujours utiliser `Promise.allSettled` pour les écrans d'accueil ou de synthèse.
4. **Vérification d'existence des sessions et objets dynamiques :**
   - *Erreur passée* : Accéder directement à `currentUser` ou à `data.containers.length` sans garde.
   - *Règle* :
     ```javascript
     const user = (currentUserSession && currentUserSession.username) || "chomiam";
     const containers = Array.isArray(data.containers) ? data.containers : [];
     ```
5. **Pré-remplissage des compteurs de sous-onglets :**
   - *Erreur passée* : Ne charger les données d'un sous-onglet (ex: `Générations Système ( -- )`) que lors du clic sur l'onglet, laissant `--` affiché par défaut.
   - *Règle* : Fournir les compteurs légers dès la réponse de synthèse globale du backend (`status.system_generations_count`) ou précharger les requêtes en tâche de fond.
6. **Mise en page des tableaux denses et badges d'origine :**
   - *Erreur passée* : Badges sans `white-space: nowrap` qui se coupent verticalement dans les cellules étroites (ex: `🔒 Système` sur une ligne et `[NixOS]` en dessous) et mélange désordonné des démons système et des règles personnalisées de l'utilisateur.
   - *Règle* :
     - Toujours verrouiller les badges compacts avec `white-space: nowrap; display: inline-flex; align-items: center; gap: 5px;`.
     - Dans les tableaux où cohabitent des dizaines d'entrées système immuables et des règles utilisateur, structurer l'affichage en groupes distincts avec accordéon réductible (`localStorage`) pour les éléments système.
7. **Cycle de vie et multi-états des conteneurs applicatifs / Serveurs de jeu :**
   - *Erreur passée* : Détection binaire simplifiée (`Running == true -> online`, sinon `offline`), masquant les crashs applicatifs et les phases critiques de boot.
   - *Règle* :
     - Différencier l'arrêt volontaire (`ExitCode == 0` -> `stopped`) du crash/erreur (`ExitCode != 0`, `OOMKilled`, ou `dead` -> `error`).
     - Détecter la phase de démarrage (`starting`) : transition via actions utilisateur (start/restart), probe TCP non bloquant sur le port de jeu, et délai de warm-up de boot.
     - Adapter les boutons d'actions selon l'état réel (bouton "Voir Crash Log" sur erreur, "Console Boot" pendant le démarrage).
8. **Performance de rendu & Pattern SWR (Stale-While-Revalidate) :**
   - *Erreur passée* : Attendre la réponse réseau d'une API lourde (ex: `list_game_servers` qui interroge Docker) avant de rendre l'UI, laissant l'utilisateur devant un écran vide pendant 2 à 3 secondes.
   - *Règle* :
     - Hydrater immédiatement l'UI (0ms) depuis `localStorage` au chargement de l'onglet ou de la page.
     - Déclencher la requête API en arrière-plan et rafraîchir le DOM de manière transparente à l'arrivée des données fraîches.
     - Côté Backend Rust : grouper les inspections Docker en une seule commande batch (`docker inspect c1 c2 ...`), court-circuiter dès le début si la liste est vide, et mettre en cache mémoire (TTL) les catalogues ou fichiers statiques souvent relus sur disque.
9. **Feedback visuel & progression des actions asynchrones (Boutons d'inspection / mise à jour) :**
   - *Erreur passée* : Boutons d'action réseau (ex: "Vérifier maintenant") plats et statiques qui se contentent d'un texte figé "Recherche...", ne donnant aucun sentiment de travail actif ni de progression.
   - *Règle* :
     - Intégrer un balayage lumineux shimmer et une icône rotative fluide (`@keyframes spinIconSmooth`).
     - Intégrer une jauge de progression micro-fine sur la bordure du bouton (incréments d'étapes : 30% -> 65% -> 85% -> 100%).
     - Afficher des libellés contextuels séquentiels ("Interrogation GitHub...", "Analyse des paquets...", "Finalisation...").
     - Offrir un état de succès transitoire bien visible (bordure verte, checkmark "✨ Système synchronisé !") avec impulsion lumineuse sur l'horodatage.

10. **Persistance bidirectionnelle de l'état d'interface (Routage URL Hash & LocalStorage) :**
    - *Erreur passée* : Sauvegarder l'onglet actif uniquement en `sessionStorage` et conserver une URL statique `/`.
    - *Impact* : Tout rafraîchissement (F5, saisie barre d'adresse, navigation privée) ramenait systématiquement l'utilisateur sur la Vue d'ensemble (`tab-overview`), et la vue d'accueil clignotait brièvement à chaque rechargement avant que l'authentification asynchrone ne s'achève.
    - *Règle* :
      - Toujours synchroniser l'onglet et le sous-onglet actif avec `window.location.hash` (`#storage`, `#files`, `#containers/store`, `#network/samba`, etc.) et avec `localStorage`.
      - Supporter les boutons Précédent/Suivant de l'historique du navigateur via `window.addEventListener("hashchange", ...)`.
      - Appliquer un pré-rendu anti-flicker dès l'analyse DOM (`DOMContentLoaded` et attribut `data-initial-tab` dans l'en-tête HTML) pour masquer instantanément l'accueil et activer le bon onglet avant même l'appel réseau `/api/auth/me`.
      - Sauvegarder et restaurer le dossier courant de l'explorateur de fichiers (`steveos_files_path`) dans `localStorage`.

11. **Superposition et plan d'affichage des notifications Toast (`z-index` & `pointer-events`) :**
    - *Erreur passée* : Définir `#toast-container` avec `z-index: 999` tandis que les fenêtres modales et backdrops ont un `z-index: 2000`.
    - *Impact* : Lorsqu'une action déclenchait une alerte ou un toast depuis une fenêtre modale (ex: confirmation d'ouverture d'un port pare-feu en 1 clic ou notification de déploiement), le toast apparaissait masqué derrière l'overlay sombre de la modale.
    - *Règle* :
      - Élever impérativement `#toast-container` et tous les toasts flottants au premier plan absolu (`z-index: 100050 !important;`).
      - Définir `pointer-events: none;` sur `#toast-container` et `pointer-events: auto;` sur les cartes `.toast` individuelles afin d'éviter qu'une zone invisible vide ne bloque les clics sur l'interface sous-jacente.

12. **Bulle flottante de progression des mises à jour & Auto-dismiss (10s) :**
    - *Erreur passée* : Afficher un toast flottant persistant lors d'une tâche asynchrone (ex: mise à jour système NixOS) sans jamais programmer son masquage automatique lors de la complétion (`stage === "completed"`), et sans horodatage de fin dans le backend.
    - *Impact* : La bulle restait affichée indéfiniment en bas d'écran (avec un ancien libellé de compilation ou un statut achevé), obligeant l'utilisateur à chercher et cliquer manuellement sur la croix de fermeture `✕`. De plus, tout rechargement de page réaffichait la bulle indéfiniment tant qu'elle n'avait pas été expressément fermée.
    - *Règle* :
      - Enregistrer un horodatage UNIX `completed_timestamp` côté backend dès le passage à l'état `completed`.
      - Côté frontend, dès détection de l'état `completed`, basculer immédiatement le titre en confirmation claire (*"Mise à jour terminée avec succès !"*), passer la barre à 100% avec gradient vert, et armer un timer d'extinction automatique (`scheduleUpdateToastDismiss(10000)`).
      - Animer la sortie avec une classe CSS de fade-out et translation (`.toast-fading-out`) avant le masquage définitif (`display: none`).
      - Lors du rechargement de page (`checkInitialUpdateProgress`), calculer le delta de temps écoulé : masquer et purger immédiatement si plus de 10s se sont écoulées, ou planifier le masquage pour le temps restant.

13. **Hydratation instantanée SWR et sondes réseau non-bloquantes (Onglet DNS & Télémetrie) :**
    - *Erreur passée* : Déclencher des pings ICMP bloquants (`Command::new("ping")`) ou des connexions `TcpStream::connect_timeout` synchrones séquentielles dans les handlers d'API pour mesurer la latence des résolveurs DNS. Côté frontend, ne disposer d'aucun catalogue de repli local (UI vide tant que l'API n'a pas répondu) et parser `await res.json()` sans vérifier `res.ok`.
    - *Conséquences* : Écran blanc / vide sur l'onglet Réseau > DNS si l'API retourne HTTP 404 (ancien binaire en cours) ou freeze complet de la requête HTTP pendant 10 secondes si certains résolveurs ou le port 53 ne répondent pas.
    - *Règle* :
      1. Côté Frontend : toujours implémenter le pattern SWR (Stale-While-Revalidate). Rendre immédiatement (0ms) un catalogue par défaut (`DEFAULT_DNS_PROVIDERS`) dans le DOM avant tout appel réseau.
      2. Toujours valider `if (!res.ok) return;` avant de parser le flux JSON (`SyntaxError: Unexpected end of JSON input`).
      3. Côté Backend Rust : lancer toutes les mesures de latence en parallèle avec `tokio::spawn` et des timeouts très stricts (`tokio::time::timeout(Duration::from_millis(350), ...)`). Zéro appel bloquant à `ping` ou sous-processus synchrone.

14. **Intégrité de l'arborescence DOM des sous-onglets (`.network-subpane`) :**
    - *Erreur passée* : Omettre la fermeture `</div>` d'un sous-onglet (ex: `subtab-sftp`), ce qui imbriquait accidentellement le nouveau sous-onglet suivant (`subtab-dns`) à l'intérieur de l'ancien.
    - *Conséquences* : Lorsque le premier sous-onglet devient inactif (`display: none`), tous les sous-onglets enfants imbriqués sont masqués en cascade. Le clic sur l'onglet semble inopérant et laisse un écran complètement blanc.
    - *Règle* :
      1. Tous les conteneurs de sous-onglets (`.network-subpane`, `.containers-subpane`) doivent impérativement être des enfants directs de premier niveau du panneau d'onglet parent (`.tab-pane`).
      2. Toujours valider l'arbre DOM et les ancêtres des nouveaux conteneurs avec un parser DOM automatisé pour garantir l'absence d'imbrication involontaire.

15. **Ergonomie des modales de volume & Logique de montage des grappes RAID :**
    - *Règle métier* : Ne jamais proposer le bouton d'action directe "Monter sans formater" sur une grappe RAID ou une partition brute n'ayant aucun système de fichiers valide détecté (`r.filesystem` manquant, vide ou 'non formaté'). L'interface doit guider clairement l'utilisateur vers le formatage initial via un badge d'avertissement.
    - *Règle d'interface* : Privilégier une disposition panoramique horizontale (largeur 900-950px, grille 2 colonnes équilibrées) pour les formulaires riches de gestion de stockage. Les drapeaux de montage (flags `nofail`, `noatime`, `defaults`, `compress=zstd`) doivent être disposés sur des lignes distinctes (`.mount-flag-row`) avec description inline, badge de criticité et switch toggle, plutôt que comprimés dans des colonnes étroites nécessitant un défilement vertical excessif.

### C. Gestion des Médias & Authentification
6. **Streaming média (Lecteurs Audio, Vidéo, Visionneuse) :**
   - *Erreur passée* : Les balises natives `<audio>`, `<video>` et `<img>` n'envoient pas les en-têtes HTTP `Authorization: Bearer ...`.
   - *Règle* :
     - Supporter l'authentification par paramètre d'URL `?token=...` et la synchronisation de cookie de session `steveos_token`.
     - Désactiver le `Cache-Control: no-store` pour les flux multimédias avec en-têtes `Accept-Ranges: bytes` afin de permettre le scrubbing (sauts de lecture dans la vidéo/audio).

12. **Lecteurs Média Flottants (Picture-in-Picture) et Fenêtres Déplaçables :**
    - *Erreur passée* : Appliquer `!important` sur les propriétés CSS `top`, `left`, `right`, `bottom` de la modale en mode vignette (`.mini-player-mode .file-modal-window`), et définir une transition sur `all`.
    - *Impact* : Les styles inline calculés dynamiquement en JavaScript lors du glisser-déposer (`win.style.left = ...`) étaient purement ignorés par le moteur CSS en raison du `!important` de la feuille de style. Le lecteur semblait figé et impossible à déplacer. De plus, `transition: all` entraînait une latence et des saccades lors du suivi du curseur.
    - *Règle* :
      1. Déclarer la position par défaut dans le CSS *sans* `!important` (`top: 122px; right: 24px; left: auto; bottom: auto;`) et injecter les coordonnées en JS avec `win.style.setProperty("left", ..., "important")` et `win.style.setProperty("right", "auto", "important")` lors du drag.
      2. Restreindre la transition CSS aux effets de halo et bordure (`transition: box-shadow 0.25s ease, border-color 0.25s ease !important;`) sans animer les positions `top`/`left`.
      3. Aligner par défaut la position verticale sur le conteneur principal adjacent (`top: 122px` pour coïncider avec `.files-main-pane`) pour une esthétique rigoureuse.
      4. Gérer simultanément les événements souris (`mousedown`, `mousemove`, `mouseup`) et tactiles (`touchstart`, `touchmove`, `touchend`) avec `document.body.style.userSelect = "none"` pendant la translation.

### D. Déploiement & Sécurité NixOS
7. **Pas de compilation lourde locale (Règle n°1 d'AGENTS.md) :**
   - La machine de dev et le NAS ne doivent jamais compiler Rust en local. Toujours attendre la fin du build Cachix sur GitHub Actions avant de bumper `flake.lock`.
8. **Déploiement réservé à l'utilisateur (Règle n°5 d'AGENTS.md) :**
   - L'agent ne doit jamais exécuter `nixos-rebuild switch` ou `git pull` directement sur le NAS distant via SSH. C'est l'utilisateur qui déclenche la mise à jour depuis le Dashboard.
9. **Paquets déclaratifs optionnels codés en dur dans des listes statiques :**
   - *Erreur passée* : Déclarer une option `cfg.services.<pkg>.enable` (ex: `goverlay`), mais laisser `<pkg>` en dur dans une liste statique `users.users.<user>.packages` ou `environment.systemPackages` d'un module connexe (ex: `modules/gaming/default.nix`). Résultat : même si l'utilisateur désactive l'option via `vars.nix` (`goverlay = false`), le paquet et toutes ses dépendances lourdes ou cassées (ex: `lazarus-qt6`) continuent d'être injectés dans la dérivation et compilés lors d'un `nh os switch` ou `nixos-rebuild`.
   - *Règle* : Tout paquet associé à une option configurable doit être conditionné systématiquement avec `lib.optional cfg.<option>.enable <paquet>` dans TOUS les modules sans exception.
10. **Nom d'utilisateur et chemins personnels codés en dur dans une distribution partagée :**
    - *Erreurs passées* :
      1. Hardcoder un utilisateur par défaut (`chomiam`) ou des chemins `/home/chomiam` dans le backend Rust ou les options de service sans transmettre dynamiquement `vars.user.username` au dashboard (`runuser -u chomiam` échoue).
      2. Omettre `homeDirectory` lors de la génération de `vars.nix` par l'installateur web ou coder en dur `homeDirectory = "/home/chomiam"` dans `vars-defaults.nix` / `options.nix`. Résultat : NixOS créait le répertoire `/home/chomiam` pour le compte d'un tiers, et le script d'activation scannant `/home/*` croyait détecter un compte orphelin `chomiam`, le recréant automatiquement dans `/etc/passwd` avec les privilèges `wheel` (admin).
    - *Règle* :
      1. Toujours propager `user = vars.user.username` depuis `hosts/nas/configuration.nix` vers les services applicatifs.
      2. Dans l'installateur web (`install.rs`), toujours injecter explicitement `fullName` et `homeDirectory = "/home/${username}"` dans `vars.nix`.
      3. Dans `options.nix` et `configuration.nix`, dériver dynamiquement `homeDirectory = "/home/${username}"` si non spécifié, sans jamais hardcoder de chemin personnel en fallback.
      4. Dans le script d'activation `steveosUsersSync` (`modules/core/users.nix`), exclure impérativement `[ "$h" != "${u.homeDirectory}" ]` lors du balayage de `/home/*` pour éviter toute création parasite.
11. **Isolation stricte de la page de connexion & Déconnexion atomique (Dashboard) :**
    - *Erreur passée* : Déclarer l'écran de connexion sous forme d'un simple overlay semi-transparent (`backdrop-filter: blur(...)`) au-dessus de la structure complète du dashboard (`<header>`, `<nav>`, `<main>`), tout en masquant le modal par défaut (`style="display:none;"`). Lors de la déconnexion (`logoutUser`), ne faire que supprimer les jetons et réafficher le modal sans vider le DOM, sans couper le polling et sans recharger la page.
    - *Conséquences* : Le dashboard complet, ses graphiques, ses disques et son arborescence restent visibles et chargés en arrière-plan sous le modal de connexion. L'utilisateur a l'impression que la déconnexion n'a pas eu lieu tant qu'il n'actualise pas manuellement la page (`F5`).
    - *Règle* :
      1. Encapsuler impérativement toute l'interface active du NAS dans un conteneur `#app-shell`.
      2. Conditionner l'affichage via des classes CSS strictes sur `<body>` : `body.not-authenticated .app-shell { display: none !important; }` et `body.authenticated .app-shell { display: block; }`.
      3. Rendre l'écran de connexion 100% opaque (`background: #11111b;` sans transparence ni dépendance au flou d'arrière-plan).
      4. Lors du `logoutUser()` : masquer instantanément `#app-shell`, révoquer la session sur l'API, purger cookies et stockage, nettoyer les paramètres d'URL (`?token=...`), et exécuter impérativement `window.location.replace("/")` pour purger la mémoire, les intervals et le DOM.
12. **Anti-pattern `iframe src=""` et roue de chargement permanente de l'onglet :**
    - *Erreur passée* : Déclarer un `<iframe src="">` ou des `<img src="">` vides dans le DOM initial pour un composant masqué (ex: visualiseur de document PDF ou miniature YouTube).
    - *Conséquence* : Selon la RFC 3986, un URI relatif vide sur un iframe pointe vers l'URI de base elle-même (`/`). Le navigateur tente donc de charger la page complète en boucle récursive à l'intérieur de l'iframe, ce qui maintient la roue de chargement de l'onglet du navigateur (`loading spinner`) en rotation continue sans jamais s'arrêter.
    - *Règle* :
      1. Utiliser impérativement `src="about:blank"` pour tout `<iframe>` non initialisé.
      2. Fournir un fichier natif `/favicon.ico` à la racine pour éviter que le navigateur ne tourne en boucle sur une erreur 404.
      3. Toujours déclarer `preload="none"` sur les balises `<video>` et `<audio>` tant qu'aucun média n'est chargé, et ne jamais y placer `autoplay` dans le HTML initial.
13. **Désynchronisation de `flake.lock` entre le Dashboard et la configuration de l'OS (`noos-nas`) :**
    - *Erreur passée* : Développer une nouvelle version du Dashboard (`noos-nas-dashboard`), attendre le succès du workflow GitHub Actions Cachix, mais omettre de propager le hash mis à jour dans `flake.lock` de la configuration de l'OS (`noos-nas`).
    - *Impact* : Sur le NAS de l'utilisateur, le processus de mise à jour s'exécute sans erreur (`Code 0`) car `nixos-rebuild switch` applique fidèlement la configuration déclarative existante, mais le dashboard redémarre sur l'ancienne version car `flake.lock` était resté figé sur l'ancien commit.
    - *Règles & Triple Verrou* :
      1. **Script de release automatisé** : Toujours utiliser [`scripts/release.sh`](file:///home/chomiam/Projects/steveos-nas-dashboard/scripts/release.sh) qui enchaîne automatiquement la compilation Cachix et la mise à jour de `flake.lock` dans `../steveos-nas` suivi de son push sur `main`.
      2. **Résilience proactive du Dashboard** : Dans `src/updates.rs`, exécuter systématiquement `nix flake lock --update-input noos-nas-dashboard` avant `nixos-rebuild switch` dès qu'une mise à niveau du dashboard est signalée.
      3. **Télémétrie explicite** : Présenter dans l'interface la version active, la version verrouillée par l'OS et la dernière version disponible sur GitHub.

---

### E. Surveillance Matérielle, Ventilation & Sécurité Thermique
13. **Zéro composant matériel en dur (CPU / Carte Mère / GPU) :**
    - *Erreur passée* : Définir des valeurs de secours fixes (Dual Xeon E5-2650 v4, Huananzhi X99, Intel Arc A380) dans le backend Rust ou l'UI.
    - *Impact* : Tout utilisateur installant STEvE_OS sur une machine AMD Ryzen ou Intel Core voyait une fausse carte mère et un faux GPU affichés.
    - *Règle* : Détecter dynamiquement les composants via Linux sysfs (`/sys/class/dmi/id/`, `/sys/bus/pci/devices/*`, `/proc/cpuinfo`, `/sys/class/drm/`).
14. **Protection Failsafe Thermique Inviolable (80°C = 100% PWM) :**
    - *Règle fondamentale* : Dans tout démon d'asservissement des ventilateurs, forcer inconditionnellement la vitesse à 100% (255/255) si n'importe quelle sonde franchit 80°C, afin d'écarter tout risque de destruction matérielle par mauvaise courbe utilisateur.
15. **Persistance des Contrôleurs `hwmon` entre Redémarrages :**
    - *Règle* : Les index `/sys/class/hwmon/hwmonX` changent selon l'ordre d'initialisation des pilotes par le noyau Linux. Toujours associer les ventilateurs et sondes par `chip_name` (ex: `nct6775`, `k10temp`, `coretemp`) et index de canal (`fan1`, `temp1`) plutôt que par le numéro volatil de hwmon.
16. **Gestion des Ventilateurs GPU et Contrôleurs Autonomes (Intel Arc / i915 / VBIOS) :**
    - *Erreur passée* : Détecter un ventilateur GPU uniquement via la présence de `fanX_input` (tachymètre RPM) et supposer qu'il est réglable par PWM logiciel via `pwmX`.
    - *Impact* : Les ventilateurs des cartes graphiques dédiées Intel Arc (pilotes `i915` et `xe`) ne fournissent aucun fichier sysfs `pwmX` car leur régulation thermique est asservie directement en boucle fermée par le microcode matériel (VBIOS). Dans l'interface, les boutons de profils (Silencieux, Équilibré, Freeze) et le test de vitesse restaient cliquables mais n'avaient aucun effet mécanique.
    - *Règle & Pattern* :
      1. Tester systématiquement si `pwm_file.exists()` ET si le fichier est accessible en écriture (`fs::OpenOptions::new().write(true).open(&pwm_file).is_ok()`).
      2. Identifier les cartes GPU (`i915`, `xe`) et qualifier l'état `is_autonomous_firmware = true`.
      3. Dans l'UI, afficher un encart d'information Catppuccin explicatif (`🛡️ Régulation Autonome VBIOS`) et désactiver l'écrasement manuel ou le test PWM avec une mention pédagogique, tout en continuant à afficher le tachymètre RPM en temps réel.

### F. Gestion des Périphériques de Bloc, LVM, RAID & Erreurs Système
17. **Activation préalable indispensable des volumes logiques LVM (`vgchange -ay`) & Vérification de nœud :**
    - *Erreur passée* : Tenter de sonder (`blkid`) ou de formater (`mkfs.btrfs`) un volume logique LVM ou une grappe RAID (ex: `/dev/vg1/storage`) sans vérifier au préalable si le nœud de fichier bloc existe dans `/dev`.
    - *Impact* : Après un redémarrage, une importation de pool ou si le volume group n'a pas été activé par systemd/udev, `/dev/vg1/storage` est absent. `blkid` échoue en retournant un code d'erreur et une sortie vide. L'ancien code en déduisait à tort `!has_fs` et lançait un formatage forcé `mkfs.btrfs -f /dev/vg1/storage`, provoquant une cascade d'erreurs critiques (`ERROR: mount check: cannot open ...: No such file or directory`, `ERROR: zoned: unable to stat ...`).
    - *Règle* :
      1. Extraire le nom du Volume Group (`/dev/<vg>/<lv>` ou `/dev/mapper/<vg>-<lv>`) et exécuter impérativement `vgchange -ay <vg>` suivi de `udevadm settle` pour activer les volumes et forcer la création des liens symboliques et des nœuds `/dev`.
      2. Vérifier physiquement l'existence du nœud (`Path::new(&dev).exists()`).
      3. Si le fichier spécial de bloc n'existe toujours pas, interrompre immédiatement avec une erreur claire et descriptive, sans JAMAIS présumer qu'il s'agit d'un disque vierge à formater.
      4. Double confirmation du système de fichiers : si `blkid` ne renvoie rien, sonder avec `lsblk -no FSTYPE` avant d'autoriser tout formatage.

18. **Restitution des erreurs CLI/système et persistance des alertes (Toasts d'erreur 10s+) :**
    - *Erreur passée* : Afficher les erreurs système brutes (multi-lignes de mkfs, mount, udev, btrfs) dans une notification Toast standard de 3,5 secondes, sans formatage, sans bouton de copie et sans temps de lecture suffisant.
    - *Impact* : L'utilisateur ne dispose pas du temps nécessaire pour lire le message qui disparaît immédiatement, et le texte tronqué empêche tout diagnostic ou transmission au support.
    - *Règle* :
      1. Les erreurs système et de montage doivent s'afficher dans un composant dédié (`#system-error-toast` ou toast d'erreur persistant) affiché pendant **au moins 10 à 12 secondes**.
      2. Fournir une barre de décompte visuelle (`countdown progress bar`) avec mise en pause au survol de la souris (`mouseenter` / `mouseleave`).
      3. Séparer le diagnostic en un résumé humain court et une zone de terminal monospace rétro-éclairée (`<pre><code>`) pour les logs techniques détaillés.
      4. Intégrer un bouton de copie rapide en 1 clic (`📋 Copier le diagnostic`) et une croix de fermeture manuelle.

19. **Déploiement Docker, Conflit d'écoute sur le port 53 (DNS) & Modale d'Erreur Centrée :**
    - *Erreur passée* : Déverser l'intégralité du log stdout/stderr d'une erreur de déploiement Docker (souvent 50 lignes de couches d'images et d'erreurs OCI) dans le petit toast flottant en bas à droite de l'écran, tout en bloquant sur un conflit de port 53 (`failed to bind host port 0.0.0.0:53/tcp: address already in use`).
    - *Impact* : Le panneau latéral droit était submergé par un bloc rouge étriqué impossible à lire ou à copier confortablement, et l'utilisateur ne comprenait pas pourquoi un conteneur DNS échouait alors qu'il n'avait rien configuré d'autre manuellement.
    - *Règle* :
      1. Les erreurs de déploiement Docker Compose volumineuses doivent ouvrir automatiquement une modale centrée haute priorité (`#modal-docker-deploy-error` avec `z-index: 100070 !important`), dotée d'une zone de texte `<textarea>` monospace plein format, d'un bouton de copie en 1 clic et d'un bandeau d'analyse contextuelle.
      2. Le toast flottant en bas à droite ne doit afficher qu'un résumé concis d'une seule ligne et un bouton `🔍 Voir le rapport d'erreur` pour rouvrir la modale.
      3. Pour le port 53 (AdGuard Home, Pi-hole) : sous Linux, Docker tente de lier `0.0.0.0:53`, ce qui échoue si `systemd-resolved` écoute sur `127.0.0.53:53` ou si `dnsmasq` écoute sur `192.168.122.1:53`. La solution pérenne sous NixOS est de désactiver le stub listener local (`services.resolved.extraConfig = "DNSStubListener=no\n";`) tout en maintenant des résolveurs amonts dans `networking.nameservers` afin que le NAS conserve son accès Internet en toutes circonstances.

20. **Publication impérative des Docker Compose sur `steveos_nas_store`, format standard et protocoles réseaux :**
    - *Erreurs passées* :
      1. Modifier localement ou générer un `docker-compose.yml` sans le committer ni le pousser sur le dépôt GitHub officiel [`Chomiam/steveos_nas_store`](https://github.com/Chomiam/steveos_nas_store). Le dashboard téléchargeant les manifests et compose directement depuis GitHub (`raw.githubusercontent.com`), toute omission laissait les NAS déployer des versions obsolètes, erronées ou cassées.
      2. Mappages incomplets de protocoles : pour les services DNS (AdGuard Home, Pi-hole), mapper uniquement `53:53` (TCP seul sous Docker). Or 99% du trafic DNS standard s'exécute en UDP, provoquant un échec total de résolution DNS des clients.
      3. Omission des ports d'administration Web : mapper uniquement le port applicatif (53) sans mapper le port d'initialisation et d'interface web (`3000:3000`), entraînant un chargement infini dans le navigateur (`http://IP:3000`).
      4. Déclarer des volumes inadaptés (ex: `/data` au lieu de `/opt/adguardhome/work` et `/opt/adguardhome/conf`).
    - *Règles obligatoires* :
      1. **Dépôt centralisé** : Toute modification sur une application Docker du store (compose, manifest, icône) DOIT être commitée et poussée immédiatement sur `https://github.com/Chomiam/steveos_nas_store`.
      2. **Format standard avec commentaires d'en-tête décoratifs** : Chaque `compose.yaml` doit obligatoirement respecter l'en-tête officiel STEvE_OS avec métadonnées (`Application`, `Port hôte`, `Données hôte`, `Mode`) et bloc de transition NixOS en pied de page.
      3. **Ports & Protocoles stricts** : Spécifier explicitement `/tcp` et `/udp` dès qu'un service écoute sur les deux (notamment port 53). Aligner `default_port` dans `manifest.json` et `store.json` sur le port réel de l'interface Web (ex: 3000 pour AdGuard Home).

21. **Gestion des Conteneurs Applicatifs vs Serveurs de Jeux & Suppression avec confirmation :**
    - *Erreurs passées* :
      1. Affichage en grille désordonné : une grille de cartes volumineuses rendait la lecture difficile dès qu'un conteneur avait un nom ou une image un peu longue, masquait les boutons d'actions et manquait de compacité.
      2. Pollution de l'onglet Docker Compose par les conteneurs de serveurs de jeux (`steveos-game*`) : ces conteneurs sont déjà gérés avec un cycle de vie complet dans la section Serveurs de Jeux / Eggs. Leur présence dans l'onglet applicatif créait de la confusion.
      3. Absence d'un bouton de suppression de conteneur : l'utilisateur était obligé d'utiliser la CLI pour supprimer un conteneur arrêté ou défectueux.
      4. Suppression brutale sans confirmation ni option pour purger l'image : risque d'effacement accidentel ou accumulation d'images orphelines volumineuses sur le disque hôte.
    - *Règles & Patterns* :
      1. **Affichage en Lignes (`.docker-container-row`)** : Vue horizontale compacte, bordure latérale d'état (vert/orange), nom tronqué avec tooltip `max-width`, image et ports bien séparés, et boutons d'actions iconiques alignés sur la droite.
      2. **Filtrage contextuel étanche** : Toujours filtrer `!name.startsWith("steveos-game")` dans `loadDockerContainers()` pour isoler les conteneurs applicatifs de la boutique des conteneurs de jeux.
      3. **Suppression sécurisée avec modale (#modal-delete-docker)** : Confirmation obligatoire avec rappel de la préservation des volumes sur l'hôte et case à cocher pour purger l'image Docker (`delete_image=true`). Le backend exécute un `docker compose down` si un fichier compose existe ou un `docker stop` + `docker rm` en fallback, suivi de `docker rmi` si demandé.

22. **Libération Totale du Port 53 pour AdGuard Home / Pi-hole (systemd-resolved, libvirt & sudo runtime) :**
    - *Erreurs passées* :
      1. Conditionner `services.resolved.settings.Resolve.DNSStubListener = "no"` sans positionner `services.resolved.enable = true` dans NixOS. Par défaut sous Nixpkgs, `services.resolved.enable = false`, ce qui fait que NixOS ignore complètement `settings` et ne génère aucun fichier `/etc/systemd/resolved.conf`. Le démon `systemd-resolved` hérité au niveau système démarre donc avec sa configuration par défaut et continue d'écouter sur `127.0.0.53:53` et `127.0.0.54:53`.
      2. Le réseau NAT par défaut de libvirt (`virbr0`) démarre automatiquement `dnsmasq` sur `192.168.122.1:53`. Même s'il écoute sur l'adresse du bridge virtuel, sous le noyau Linux cela provoque un conflit `EADDRINUSE` lorsque Docker tente de lier `0.0.0.0:53`.
      3. Le backend Rust tentait d'écrire dans `/etc/systemd/` et d'appeler `systemctl` sans `sudo` ; les appels échouaient silencieusement faute de privilèges root.
    - *Règles & Patterns éprouvés* :
      1. **NixOS** : Toujours coupler `services.resolved.enable = true;` avec `services.resolved.settings.Resolve.DNSStubListener = "no";` pour garantir la génération déclarative de `resolved.conf`.
      2. **Libvirt** : Toujours inclure `<dns enable='no'/>` dans la définition du réseau `virbr0` afin que `dnsmasq` ne fournisse que le DHCP sans écouter sur le port 53.
      3. **Dashboard Web** : Exécuter la libération à chaud via `sudo systemctl stop/restart systemd-resolved` (bénéficiant de `security.sudo.extraRules` sans mot de passe) et injecter proactivement la libération du port 53 avant le `docker compose up` des applications DNS.

23. **Activation & Résolution Robuste des Périphériques LVM/RAID (`/dev/<vg>/<lv>`, `/dev/mapper/`) et Modules Noyau Hôte :**
    - *Erreurs passées* :
      1. Dans `resolve_and_activate_block_device`, lorsqu'un chemin complet de volume logique LVM (`/dev/vg1/storage`) était passé, le Cas A (`vgs vg1/storage`) échouait (car `vgs` n'attend qu'un nom de VG). Le Cas B prenait le relais mais ne chargeait pas les modules noyau RAID (`dm-raid`, `raid456`, etc.). Sans ces modules noyau, `lvchange -ay` échoue silencieusement sous Linux car le kernel ne peut pas instancier la cible device-mapper RAID5.
      2. Si le volume logique `storage` n'avait pas encore été alloué dans le groupe `vg1`, le Cas B ne procédait à aucune création (`lvcreate`) lors d'une requête de formatage forcé (`force_format == true`), contrairement au Cas A.
      3. Dépendance aveugle envers udev : `lvchange` active le device-mapper, mais les liens symboliques `/dev/<vg>/<lv>` et les fichiers de périphériques peuvent tarder à être créés par udev. Sans `vgmknodes`, le test `Path::new("/dev/vg1/storage").exists()` retournait `false` immédiatement.
      4. Dans la configuration déclarative NixOS (`steveos-nas`), les modules noyau RAID (`dm-mod`, `dm-raid`, `raid0`, `raid1`, `raid456`, `raid10`) n'étaient pas déclarés dans `boot.kernelModules`, empêchant la reconnaissance native des grappes au démarrage système.
    - *Règles & Patterns éprouvés* :
      1. **Modules noyau déclaratifs & dynamiques** : Toujours inscrire les modules RAID dans `boot.kernelModules` dans `modules/storage/default.nix`, et charger proactivement `dm-mod`, `dm-raid`, `raid0`, `raid1`, `raid456`, `raid10` via `modprobe` dans le backend Rust avant toute activation LVM.
      2. **Parsing & Résolution unifiée VG/LV** : Qu'on reçoive `/dev/<vg>/<lv>`, `<vg>/<lv>`, `/dev/mapper/<vg>-<lv>`, ou `/dev/<vg>`, décomposer en `(vg_candidate, lv_candidate)`. Si le VG existe (`vgs <vg>`), activer avec `vgchange -ay -K --activationmode degraded <vg>`.
      3. **Auto-allocation à la demande** : Si un LV spécifique est ciblé mais n'existe pas encore dans le VG (`lvs <vg>/<lv>` absent) et que `force_format` est demandé, exécuter automatiquement `lvcreate --type <raid_type> -l 100%FREE -n <lv> <vg>`.
      4. **Génération synchrone des nœuds spéciaux** : Toujours exécuter `vgmknodes` et `udevadm settle --timeout=3` après l'activation. Tester à la fois `/dev/<vg>/<lv>`, `/dev/mapper/<vg>-<lv>` (avec substitution des tirets LVM `--`), et vérifier via `dmsetup info` en repli.

24. **Gestion du Pare-feu NixOS et Conteneurs Docker Multi-ports (DOCKER-USER, ports web vs DNS 53) :**
    - *Erreurs passées* :
      1. Par défaut sous NixOS, `networking.firewall.enable = true` applique une politique `DROP` sur le trafic non autorisé. Lorsque des conteneurs Docker publient des ports sur l'hôte, les paquets entrants provenant du LAN arrivant sur l'interface physique traversent la chaîne `FORWARD`. Faute d'inclusion de la règle `DOCKER-USER -j ACCEPT` et de `docker0` dans `trustedInterfaces`, le pare-feu NixOS effectue un DROP silencieux, ce qui provoque un **timeout TCP / chargement infini** dans le navigateur des clients LAN (ex: `http://<IP_NAS>:3000`).
      2. Dans l'App Store (`docker_store.rs`), `customize_compose_yaml` remplaçait aveuglément la première ligne de port du template par le port configuré. Pour AdGuard Home, la première ligne étant `53:53/tcp`, celle-ci était réécrite en `3000:53/tcp`, redirigeant le port HTTP 3000 vers le démon DNS !
      3. Dans `services.rs`, `extract_web_port` sélectionnait le premier port exposé (53), générant un lien `http://<IP_NAS>:53` bloqué par les navigateurs (`ERR_UNSAFE_PORT`).
      4. **Conflit de port DHCP client (Port 68/udp)** : Inclure `68:68/udp` dans le Compose d'AdGuard Home provoque un crash `failed to bind host port 0.0.0.0:68/udp: address already in use` car `dhcpcd` (ou le client DHCP de l'hôte) utilise déjà ce port. Lorsqu'un port échoue à se lier, Docker abandonne la configuration réseau du conteneur (`Networks: {}` et `Ports: {}`), laissant le conteneur isolé sur `127.0.0.1` sans aucun port hôte publié.
    - *Règles & Patterns éprouvés* :
      1. **NixOS Firewall & Docker** : Toujours inclure `networking.firewall.trustedInterfaces = [ "docker0" ];` et `networking.firewall.extraCommands = '' iptables -I DOCKER-USER -j ACCEPT 2>/dev/null || true '';` dans `modules/services/containers.nix`. Cela permet au trafic légitime des conteneurs explicitement publiés par Docker d'être routé sans blocage tout en conservant la protection intégrale des ports du système hôte (`INPUT`).
      2. **Résolution ciblée des ports d'interface Web** : Dans `extract_web_port`, toujours filtrer la liste des ports non-web / unsafe (53, 67, 68, 853, 22...) et prioriser les ports Web canoniques (3000, 80, 8080, 443, etc.).
      3. **Substitution intelligente dans les Compose multi-ports** : Ne jamais écraser un port DNS (53) ou auxiliaire lors de la personnalisation du port Web d'une application de boutique.
      4. **Ne jamais exposer le port client DHCP 68** : Les conteneurs DNS/DHCP n'ont besoin que du port serveur 67/udp si le DHCP est activé, jamais du port client 68 qui est réservé à l'OS hôte.

25. **Formatage de Volume RAID : Élimination des Verrous d'Interface Silencieux (Bouton disabled sans retour) et Conflits de Signatures Superblocs (wipefs -a et -t <fstype>) :**
    - *Erreurs passées* :
      1. **Verrouillage UI passif par attribut `disabled`** : Dans la modale de formatage (`mount-volume-modal`), le bouton de soumission « Formater et monter » était purement et simplement désactivé par l'attribut HTML `disabled = true` tant que la case de confirmation obligatoire n'était pas cochée. Lorsqu'un utilisateur clique sur un `<button disabled>`, le navigateur n'émet aucun événement `click` et l'interface reste muette (« il ne se passe absolument rien »). L'utilisateur croit alors à un bogue de clic ou à un freeze de l'interface.
      2. **Conflit de signatures superbloc résiduelles lors du montage** : Lorsqu'un volume ou des disques physiques ont été précédemment formatés sous un autre système de fichiers (ex: XFS), `mkfs.btrfs` initialise le système Btrfs mais des métadonnées résiduelles peuvent subsister. Si la commande `mount` est appelée sans le drapeau explicite `-t <fstype>`, `libmount` peut sonder et détecter l'ancienne signature (ex: XFS) et tenter d'appliquer des options incompatibles (`compress=zstd`), provoquant l'erreur système critique : `mount: /mnt/storage: échec de fsconfig() :xfs: Unknown parameter 'compress'`.
    - *Règles & Patterns éprouvés* :
      1. **Interactivité & Guidage UI au Clic** : Ne jamais laisser un bouton d'action principal silencieusement bloqué par `disabled`. Le bouton doit demeurer cliquable : si la case de confirmation obligatoire n'est pas cochée au moment du clic, afficher un toast explicatif (`showToast`), déclencher une animation de secousse visuelle (`shake-alert`), faire défiler la vue (`scrollIntoView`) jusqu'à la case et placer le focus dessus.
      2. **Purge Préventive des Signatures avec `wipefs -a`** : Avant tout formatage forcé d'un volume logique ou périphérique bloc (`mkfs.btrfs`, `mkfs.ext4`, etc.), exécuter systématiquement `wipefs -a &final_block_device` suivi de `udevadm settle --timeout=2` pour détruire toute signature ou superbloc antérieur susceptible de tromper `blkid` ou `libmount`.
      3. **Spécification Explicite du Type de Système de Fichiers (`-t <detected_fs>`)** : Toujours passer l'argument explicite `-t <fs_type>` (ex: `-t btrfs`) à la commande `mount` au lieu de laisser le noyau ou la libc deviner le pilote.

26. **Architecture des Partages Réseau Dynamiques Samba & Remplacement de `extraConfig` dans Nixpkgs 24.11+ / 26.05 :**
    - *Piège Nixpkgs & Erreurs passées* :
      1. Tenter d'utiliser `services.samba.extraConfig` dans un module NixOS moderne. Dans Nixpkgs 24.11+ et instable (26.05), l'option `extraConfig` a été dépréciée puis définitivement supprimée. Son utilisation lève une erreur fatale d'évaluation (`The option services.samba.extraConfig does not exist`).
      2. Redémarrer brutalement Samba (`systemctl restart samba-smbd`) lors de chaque création ou modification de partage. Cela coupait les transferts de fichiers en cours et déconnectait les sessions actives des clients Windows, macOS ou Linux.
    - *Règles & Architecture pérenne STEvE_OS* :
      1. **Inclusion Déclarative Propre** : Déclarer dans le module NixOS `modules/services/samba.nix` :
         ```nix
         services.samba.settings.global."include" = "/var/lib/steveos/samba_shares.conf";
         ```
         et assurer la présence persistante des fichiers via `systemd.tmpfiles.rules`.
      2. **Génération Dynamique & Persistance** : Le backend Rust (`src/samba.rs`) gère les partages déclaratifs au format JSON (`/var/lib/steveos/samba_shares.json`) et compile instantanément la configuration Samba au format smb.conf (`/var/lib/steveos/samba_shares.conf`).
      3. **Rechargement à Chaud sans Déconnexion (Zero Downtime)** : Toujours recharger Samba via `systemctl reload-or-restart samba-smbd` et `smbcontrol smbd reload-config` pour appliquer les nouveaux partages et permissions sans interrompre les sessions TCP des clients connectés.
      4. **Extensions VFS Clés** :
         - Apple macOS Finder & Time Machine : `vfs objects = fruit streams_xattr`, `fruit:time machine = yes`.
         - Corbeille Réseau Automatique : `vfs objects = recycle`, `recycle:repository = .recycle`, `recycle:keeptree = yes`.
         - Clichés Instantanés Windows VSS : `vfs objects = shadow_copy2`.
         - Chiffrement Matériel SMB3 : `smb encrypt = mandatory` ou `desired` (AES-128/256-GCM).
      5. **Séparation Déclarative des Utilisateurs** : La gestion et la création des utilisateurs/mots de passe relèvent exclusivement du module et de l'onglet `Utilisateurs & Groupes` (`#tab-users`). L'onglet Samba ne doit pas réinventer la création de comptes, mais se concentrer sur l'attribution fine des droits d'accès (`valid_users`, `write_list`, `read_list`).

27. **Architecture sFTP Haute Sécurité, Confinement Chroot Jail et Filtrage des Utilisateurs SSH :**
    - *Pièges & Bonnes pratiques sFTP / SSH* :
      1. **Sous-système internal-sftp vs sftp-server binaire** : Toujours utiliser `internal-sftp` pour OpenSSH. Contrairement à `sftp-server` (qui requiert la présence des bibliothèques dynamiques `/lib`, `/bin/sh` et des fichiers de périphériques dans la prison chroot), `internal-sftp` s'exécute directement dans le contexte du processus sshd sans nécessiter aucune arborescence FHS dans le dossier chrooté.
      2. **Filtrage des Comptes Autorisés SSH** : Ne jamais exposer tous les utilisateurs du système au protocole sFTP. Les comptes créés avec le shell `/run/current-system/sw/bin/nologin` (utilisateurs de partages Samba seuls) doivent être formellement exclus de la liste de sélection et de l'accès SSH.
      3. **Inclusion Déclarative OpenSSH** : Utiliser `services.openssh.extraConfig = "Include /var/lib/steveos/sftp_shares.conf\\n";` pour injecter dynamiquement les blocs `Match User` et `ChrootDirectory` sans modifier impurement la configuration globale de NixOS.
      4. **Mode Lecture Seule sFTP** : Dans OpenSSH, l'interdiction d'écriture et d'upload sFTP s'applique de manière native via le drapeau `-R` : `ForceCommand internal-sftp -R -u 002`.
      5. **Rechargement à Chaud sans Déconnexion (SIGHUP)** : Toujours recharger le démon OpenSSH via `systemctl reload-or-restart sshd` (signal HUP). Les sessions de transfert sFTP en cours ne sont pas interrompues lors de l'ajout ou de la modification d'un partage.



28. **Montage Réseau Distant (sshfs / CIFS), Découverte LAN et Points de Montage Épinglés :**
    - *Pièges & Bonnes pratiques explorateur de fichiers* :
      1. **FUSE et user_allow_other** : Pour que les montages FUSE (sshfs) soient accessibles par les utilisateurs non-root et le service web STEvE_OS, toujours activer déclarativement `programs.fuse.userAllowOther = true;` dans NixOS et passer l option `-o allow_other,idmap=user`.
      2. **Sécurité des Mots de Passe en CLI** : Ne jamais passer de mots de passe en clair dans les arguments de ligne de commande sshfs (visibles dans `/proc` via `ps`). Toujours privilégier `password_stdin` via flux stdin sécurisé ou les clés SSH privées.
      3. **Découverte Passive et Non-Bloquante** : La détection des partages LAN doit combiner la table ARP du noyau (`/proc/net/arp`), mDNS/Avahi et des sondes TCP asynchrones sur ports 445 (SMB) et 22 (sFTP) avec des timeouts très courts (<= 250ms) pour ne jamais bloquer l interface.
      4. **VFS Transparente pour l Explorateur** : En montant les partages sous `/mnt/remote/<proto>/<id>`, l explorateur de fichiers réutilise 100% de ses fonctionnalités natives (streaming, renommage, compression, upload/download, miniatures) sans avoir besoin d écrire un client sFTP/SMB virtuel.

29. **Codes d'arrêt de conteneurs Docker (SIGINT 130 / SIGTERM 143 vs crash) & Délai de grâce pour serveurs de jeu :**
    - *Erreurs passées* :
      1. Catégoriser tout code d'arrêt non nul (`exit_code != 0`) d'un conteneur arrêté comme un crash critique (`s.status = "error"`).
      2. Lorsqu'un utilisateur arrêtait proprement son serveur Minecraft, Palworld ou Valheim, le conteneur s'arrêtait avec le code 130 ou 143. Le dashboard affichait une pastille rouge alarmante « Arrêt anormal (Code 130) » ou « Arrêté par signal SIGTERM (143) » et un bouton « Crash Log », faisant croire à une défaillance brutale.
      3. Exécuter `docker stop <container>` avec le timeout par défaut (10 secondes), insuffisant pour les serveurs de jeu nécessitant de sauvegarder de volumineux chunks sur disque avant l'extinction, ce qui entraînait parfois un arrêt forcé par `SIGKILL` (code 137).
    - *Règles & Patterns éprouvés* :
      1. **Codes de signal conventionnels Linux/POSIX (128 + N)** :
         - Interruption utilisateur / signal d'arrêt propre : `SIGINT` (signal 2) -> Code **130** (`128 + 2`).
         - Signal de terminaison gracieux standard Docker : `SIGTERM` (signal 15) -> Code **143** (`128 + 15`).
         - Sortie nominale : Code **0**.
         - Tous ces codes (0, 130, 143) traduisent un arrêt normal et ordonné du processus, et doivent impérativement être qualifiés de `s.status = "stopped"` avec une description rassurante (« Arrêté proprement »).
      2. **Véritables codes de crash ou d'anomalie** :
         - Code 137 (`128 + 9`, SIGKILL) : conteneur tué de force par timeout ou saturé en RAM (`OOMKilled`).
         - Code 139 (`128 + 11`, SIGSEGV) : violation d'accès mémoire / crash binaire natif.
         - Code 1 : erreur applicative non gérée (script d'entrypoint échoué, dépendance manquante).
         - Seuls ces codes justifient l'état `s.status = "error"` et le bouton d'inspection des logs de crash.
      3. **Délai de grâce généreux (`-t 30`)** : Toujours allouer au moins 25 à 30 secondes (`docker stop -t 30 <container>` et `docker restart -t 30 <container>`) aux commandes d'arrêt et redémarrage de serveurs de jeu afin de laisser le temps aux moteurs de jeu d'achever la synchronisation sur disque sans subir un SIGKILL prématuré.

30. **Cycle de vie et suppression robuste des comptes utilisateurs Linux/Samba (Verrous userdel, sessions résiduelles & compte principal) :**
    - *Erreurs passées* :
      1. Utiliser `userdel` sans l'option `-f` (force). Dès qu'un utilisateur possède un processus actif, une session SSH ouverte, ou une tranche systemd (`user-<uid>.slice`), `userdel` échoue avec l'erreur `userdel: user <user> is currently used by process <PID>` (code 8).
      2. Traiter le code de retour 6 (`user does not exist`) comme une défaillance critique et bloquer la purge de `users-registry.json`. Si le compte était déjà absent de `/etc/passwd` ou purement virtuel dans le registre, la modale de suppression échouait en boucle sans jamais retirer l'utilisateur.
      3. Utiliser l'argument `-r` de `userdel` qui échoue de manière bloquante (code 12) si des fichiers appartenant à un tiers (fichiers root créés par des conteneurs, montages) se trouvent dans `/home/<user>`.
      4. Omettre l'état de chargement (`disabled`, spinner) sur le bouton "Confirmer la Suppression" du modal, ne fournissant aucun retour visuel pendant les opérations système.
    - *Règles & Patterns éprouvés* :
      1. **Purge préalable des processus & sessions** :
         - Toujours exécuter `loginctl terminate-user <username>` et `pkill -9 -u <username>` suivi d'un bref délai d'attente (150ms) pour libérer les verrous sur `/etc/passwd` et les descripteurs de fichiers avant d'appeler `userdel`.
      2. **Forçage `userdel -f` et tolérance de non-existence (Code 6)** :
         - Utiliser `userdel -f <username>` pour forcer la suppression même si des résidus de session persistent.
         - Considérer les codes 0 (succès) et 6 (`user does not exist`) comme une validation de sortie : dans les deux cas, le compte est garanti absent de `/etc/passwd`.
      3. **Suppression indépendante des répertoires hôtes** :
         - Supprimer `/home/<username>` et `/mnt/storage/shares/<username>` via des commandes `rm -rf` et `std::fs::remove_dir_all` dédiées plutôt que de faire dépendre le succès global de l'option `-r` de `userdel`.
      4. **Nettoyage Samba complet** :
         - Exécuter à la fois `smbpasswd -x <username>` et `pdbedit -x -u <username>`.
      5. **Immunité du compte administrateur principal** :
         - Protéger le compte principal déclaré dans `vars.nix` (`STEVEOS_USER`) contre toute tentative de suppression avec le même statut immuable que `root` (`🔒 Immuable` dans l'interface et blocage 400 dans l'API).
      6. **Gestion d'état UI au clic** :
         - Désactiver le bouton de confirmation, afficher `⏳ Suppression en cours...`, encoder l'URL (`encodeURIComponent`), et parser la réponse avec gestion de repli en cas de code HTTP d'erreur.

---





31. **Scoping Strict des Verrous Asynchrones Tokio (`RwLock`) dans les Middlewares Axum & Prévention des Interblocages Post-Reboot :**
    - *Erreurs passées* :
      1. Maintenir un verrou d'écriture (`RwLockWriteGuard`) ou de lecture (`RwLockReadGuard`) actif pendant l'évaluation de `next.run(req).await` dans un middleware Axum (`auth_middleware`).
      2. Lors d'un redémarrage du système, le cache de sessions en mémoire vive est vide mais les sessions persistent sur disque (`/var/lib/steveos/sessions.json`). Lorsque le navigateur se reconnecte avec des dizaines de requêtes simultanées, la première requête recharge la session depuis le disque et prend un verrou d'écriture `sessions_lock.write().await`. En exécutant `return next.run(req).await;` au sein du scope de ce verrou, le verrou exclusif reste verrouillé pendant toute la durée du traitement de la requête HTTP (qui peut durer 1 à 2 secondes pour les sondes matérielles).
      3. Toutes les requêtes concurrentes s'empilent en attente de lecture (`sessions_lock.read().await`).
      4. Si le handler de la première requête (ou une tâche fille) requiert les détails de session via `get_session_from_headers`, il sollicite un verrou de lecture sur la même tâche. Comme le verrou Tokio `RwLock` n'est pas réentrant, cela engendre un interblocage immédiat (deadlock) et permanent, gelant la totalité des 48 threads Tokio de l'application dans `futex(FUTEX_WAIT_PRIVATE, ...)`.
    - *Règles & Patterns éprouvés* :
      1. **Isolation stricte par blocs `{ ... }` (Drop immédiat)** : Tout verrou `RwLockReadGuard` ou `RwLockWriteGuard` doit impérativement être acquis, utilisé et relâché au sein d'un sous-bloc explicite `{ ... }` avant tout passage au middleware suivant (`next.run(req).await`) ou toute opération I/O asynchrone longue.
      2. **Pattern de validation sans maintien de garde** :
         ```rust
         let is_valid = {
             let sessions = sessions_lock.read().await;
             sessions.get(&token).map(|s| s.expires_at > now).unwrap_or(false)
         }; // La garde `sessions` est détruite et le verrou libéré ici !
         if is_valid {
             return next.run(req).await;
         }
         ```
      3. **Même règle pour la restauration depuis le disque** :
         ```rust
         if let Some(session) = disk_session {
             if session.expires_at > now {
                 {
                     let mut sessions = sessions_lock.write().await;
                     sessions.insert(token, session);
                 } // Verrou d'écriture libéré immédiatement en RAM
                 return next.run(req).await;
             }
         }
         ```

32. **Parsing Robuste des Métriques `/proc/mdstat` (Barre de Progression ASCII vs Symboles et Lignes Multiples) :**
    - *Erreurs passées* :
      1. Utiliser `line.split('=')` pour extraire le pourcentage de resynchronisation ou reconstruction RAID (`resync = 88.8%`). Or la ligne de `/proc/mdstat` contient la barre ASCII `[=========>...]` qui contient une succession de caractères `=`. `split('=')` fragmentait la barre de progression et `nth(1)` renvoyait une chaîne vide, forçant le fallback à `0.0%`.
      2. Extraire le nom de la grappe sur la ligne immédiatement précédente (`lines[idx - 1]`). Or le noyau insère entre la déclaration du périphérique (`md127 : active ...`) et la progression (`[===>...]`) une ligne intermédiaire contenant le nombre de blocs et le superbloc (`7813772288 blocks super 1.2 ...`). Le premier token extrait était donc le nombre de blocs au lieu de `md127`.
      3. Utiliser des entités HTML littérales (`&bull;`) dans des assignations `textContent` en JavaScript au lieu du caractère Unicode réel (`•`).
    - *Règles & Patterns éprouvés* :
      1. **Isolation par délimiteur `%`** : Toujours cibler le token précédant le délimiteur `%` : `line.split('%').next().and_then(|s| s.split_whitespace().last())`.
      2. **Scan rétrograde des périphériques `md*`** : Parcourir les lignes antérieures en boucle inversée (`lines[..idx].iter().rev()`) jusqu'à rencontrer un entête `md* :` valide.
      3. **Unicode natif dans le DOM** : Dans `textContent`, employer directement le caractère `•` plutôt que l'entité HTML `&bull;`.

33. **Isolation Stricte des Routes WebSocket VNC & Rejet des Filtres Permissifs `path.ends_with(...)` :**
    - *Erreur passée* : Déclarer `path.ends_with("/vnc")` dans les exemptions d'authentification de `auth_middleware` pour faire fonctionner la console VNC des machines virtuelles.
    - *Impact* : Toute personne sur le LAN pouvait se connecter directement au websocket VNC (`/api/vms/:name/vnc`) de n'importe quelle VM sans authentification. De plus, `ends_with` pouvait être exploité sur d'autres endpoints.
    - *Règle* :
      1. Remplacer les filtres `ends_with` par des comparaisons strictes d'URL (`path == "/api/auth/login"`, `path == "/api/auth/status"`).
      2. Protéger impérativement les connexions WebSocket VNC par token de session transmis lors de l'établissement du handshake (`?token=...` ou cookie de session).

34. **Contrôle d'Accès Basé sur les Rôles (RBAC) & Exigence Systématique `session.is_admin` :**
    - *Erreur passée* : Valider uniquement la validité temporelle du token dans `auth_middleware` sans vérifier `session.is_admin` sur les endpoints destructifs ou sensibles (terminal, disques, formats, arrêt système, modification de fichiers, pare-feu).
    - *Impact* : Tout compte utilisateur secondaire créé pour un simple partage Samba ou sFTP pouvait appeler l'API de terminal (`/api/terminal/exec`) ou de formatage de disques et agir comme root.
    - *Règle* :
      1. Toute route permettant l'exécution de commandes, la modification de fichiers hors de son home, la gestion du stockage, la reconfiguration réseau/pare-feu ou les actions d'alimentation système DOIT impérativement exiger que `session.is_admin == true`.
      2. Renvoyer immédiatement un statut HTTP `403 Forbidden` (`{"success": false, "error": "Accès refusé. Privilèges administrateur requis."}`) si l'utilisateur n'est pas administrateur.

35. **Bannissement Absolu des Interpolations Shell (`bash -c "echo '...'"` & `sh -c`) :**
    - *Erreur passée* : Utiliser `Command::new("sudo").args(["bash", "-c", &format!("echo '{}' > /etc/resolv.conf", resolv_content)])` dans `src/dns.rs` ou `sh -c "echo '{}'"` dans `src/games.rs`.
    - *Impact* : Une entrée utilisateur contenant une simple apostrophe ou des métacaractères shell (`'`, `;`, `$()`) provoque une exécution de commande arbitraire avec les privilèges root.
    - *Règle* :
      1. Ne JAMAIS construire de commande shell via formatage de chaîne ou interpolation d'arguments non assainis.
      2. Toujours utiliser les APIs natives Rust de manipulation de fichiers (`std::fs::write`, `std::fs::OpenOptions`).
      3. Valider strictement les entrées réseau (IPs avec `IpAddr::from_str`, ports dans `1..=65535`).

36. **Incompatibilité Structurelle entre Droits de Stockage (`chmod 2775`) et Chroot sFTP OpenSSH :**
    - *Erreur passée* : Déclarer `ChrootDirectory /storage/data` dans OpenSSH pour un partage sFTP, alors que le dossier a été configuré par le module de stockage avec `chown <user>:storage` et `chmod 2775`.
    - *Impact* : Le démon `sshd` refuse immédiatement toute connexion sFTP avec l'erreur système critique `fatal: bad ownership or modes for chroot directory`. OpenSSH exige que TOUS les dossiers de l'arborescence menant au chroot appartiennent strictement à `root:root` et soient interdits d'écriture aux groupes (`chmod 755` max).
    - *Règle* :
      1. Ne jamais chrooter directement sur un point de montage multi-utilisateurs modifiable par le groupe `storage`.
      2. Pour un chroot sFTP sécurisé, créer une arborescence dédiée `/mnt/storage/sftp_jails/<user>` appartenant à `root:root` (`0755`), et monter ou lier le dossier de travail inscriptible dans un sous-répertoire `/mnt/storage/sftp_jails/<user>/data` appartenant à `<user>:storage`.

37. **Intégrité des Déclarations JS Globales, Déduplication et Piège du Hoisting (`escapeHtml`) :**
    - *Erreur passée* : Déclarer `function escapeHtml(str)` une première fois de manière sécurisée (avec garde `if (!str) return ""`), puis la redéclarer plus loin dans le fichier sans cette garde.
    - *Impact* : En JavaScript, le mécanisme de hoisting fait écraser la première définition par la seconde dans tout le script. Tout appel à `escapeHtml(null)` ou `escapeHtml(undefined)` provoque alors un plantage immédiat `TypeError: Cannot read properties of undefined (reading 'replace')`.
    - *Règle* :
      1. Toujours tester l'absence de fonctions dupliquées via un linter ou un script d'audit AST avant toute release.
      2. Déclarer systématiquement les variables avec `let` ou `const` pour éviter toute pollution du scope global `window`.

---

## 🛠️ 3. Patterns Recommandés & Recettes Éprouvées

### Comptage ultra-rapide des générations NixOS (0ms) :
Pour connaître instantanément le nombre de générations système sans lancer de commande lourde `nixos-rebuild list-generations` :
```rust
pub fn get_system_generations_count() -> u32 {
    if let Ok(entries) = fs::read_dir("/nix/var/nix/profiles") {
        entries.flatten().filter(|e| {
            let fname = e.file_name().to_string_lossy().to_string();
            fname.starts_with("system-") && fname.ends_with("-link")
        }).count() as u32
    } else {
        0
    }
}
```

### Intégration des Eggs & Bannières de Jeux :
- Toujours associer les serveurs de jeux à leur Egg d'origine via `egg_id`.
- Réutiliser dynamiquement `banner_url` (dégradé panoramique) et `icon_url` (logo PNG transparent avec ombre portée) :
  ```javascript
  const bannerStyle = bannerUrl
    ? `background: linear-gradient(180deg, rgba(17, 17, 27, 0.40) 0%, rgba(17, 17, 27, 0.92) 100%), url('${bannerUrl}') center/cover no-repeat;`
    : ``;
  ```
- Toujours prévoir un fallback sécurisé vers l'emoji natif si l'image distante échoue à charger (`onerror="this.style.display='none'; this.nextElementSibling.style.display='inline-block';"`).

---

### Régulation Thermique Linéaire par Morceaux (Courbes Ventilateurs) :
Pour convertir une température mesurée en consigne PWM fluide à partir d'une liste ordonnée de points d'inflexion `(temp_c, pwm_percent)` :
```rust
pub fn interpolate_pwm(temp: f32, curve: &[CurvePoint]) -> u8 {
    if temp >= 80.0 { return 100; } // Sécurité Failsafe inconditionnelle
    if curve.is_empty() { return 50; }
    if temp <= curve.first().unwrap().temp_c { return curve.first().unwrap().pwm_percent; }
    if temp >= curve.last().unwrap().temp_c { return curve.last().unwrap().pwm_percent; }

    for window in curve.windows(2) {
        let (p1, p2) = (&window[0], &window[1]);
        if temp >= p1.temp_c && temp <= p2.temp_c {
            let ratio = (temp - p1.temp_c) / (p2.temp_c - p1.temp_c).max(0.1);
            let pwm = p1.pwm_percent as f32 + ratio * (p2.pwm_percent as f32 - p1.pwm_percent as f32);
            return pwm.round().clamp(0.0, 100.0) as u8;
        }
    }
    100
}
```

---

### Affichage en ligne & inventaire matériel épuré (Dashboard) :
- Privilégier une disposition horizontale pleine largeur (`.hw-row-item` avec pastilles flex-wrap) plutôt qu'une grille de cartes imposantes qui fragmentent la lecture sur la page d'accueil.
- Décorer chaque ligne d'une bordure gauche thématique aux couleurs Catppuccin Mocha (CPU: mauve, Carte Mère: blue, RAM: yellow, GPU: peach, Réseau: teal, Stockage: sapphire).
- **Filtrage réseau strict** : Toujours filtrer les interfaces virtuelles bruyantes (`docker0`, `veth*`, `virbr*`, `br-*`, `wg*`, `tun*`, `tap*`) via la détection `/sys/class/net/<iface>/device` côté backend (`is_physical`) et un filtre de repli côté frontend, afin de ne valoriser sur la page d'accueil que les adaptateurs physiques réels (`eno1`, `enp*`, `eth*`) avec leur débit négocié, IPv4 locale et adresse MAC.

---

### Routage par Hash d'URL & Persistance Multi-Vues (Dashboard) :
- Ne jamais reposer uniquement sur `sessionStorage` pour l'état d'onglets : `sessionStorage` est volatile et se réinitialise lors des saisies d'URL dans la barre d'adresse, l'ouverture de nouvelles fenêtres ou les restrictions d'isolation de contexte.
- Utiliser un couplage dynamique entre `window.location.hash` (`#storage`, `#files`, `#containers`, `#network/samba`, etc.) et `localStorage.setItem("steveos_active_tab", tabId)`.
- Écouter l'événement `window.addEventListener("hashchange", ...)` pour garantir la cohérence des boutons Précédent / Suivant du navigateur.
- **Anti-flicker de la vue d'accueil** : Détecter et appliquer la vue cible dès le `DOMContentLoaded` (et via un attribut de pré-rendu `data-initial-tab` dans l'en-tête HTML) afin d'éviter tout clignotement ou affichage bref de la vue d'accueil (`tab-overview`) pendant l'attente de la vérification de session asynchrone (`/api/auth/me`).
- Conserver également le chemin actif de l'explorateur de fichiers (`steveos_files_path`) pour maintenir l'utilisateur dans son dossier courant après rafraîchissement.

---

### Transition visuelle & Rechargement fluide post-mise à jour (Dashboard) :
- Lors de la finalisation d'une mise à jour système (`stage === "completed"` ou 100%), déclencher un overlay plein écran avec floutage d'arrière-plan (`backdrop-filter: blur(20px)` et classe `body.app-updating-reload #app-shell` avec `filter: blur(14px) brightness(0.7)`).
- Présenter une carte centrée Catppuccin Mocha animée avec icône rayonnante ✨, halo radial et micro-jauge de progression de synchronisation (2.2s).
- **Persistance du contexte** : Mémoriser l'onglet actif dans `sessionStorage.setItem("steveos_active_tab", activeTab)` avant rechargement pour replacer automatiquement l'utilisateur sur son écran d'origine (qu'il soit resté sur l'onglet Mises à jour ou sur un autre onglet).
- Recharger la page via `window.location.replace()` avec un paramètre de timestamp pour purger le cache et appliquer immédiatement les nouveaux assets Web (HTML/JS/CSS) et le nouveau binaire.

---

### Architecture App Store STEvE_OS (`steveos_nas_store` & Docker Compose v2) :
- **Pourquoi Docker Compose asynchrone plutôt que la recompilation déclarative NixOS pour 600+ applications** :
  - Déployer des applications tierces via `virtualisation.oci-containers` forcerait un `nixos-rebuild switch` (plusieurs minutes d'évaluation, compilation et création de générations système) à chaque clic d'installation ou modification de variable.
  - L'orchestrateur Docker Compose v2 asynchrone permet un déploiement instantané (< 2 secondes) sous `/home/<user>/docker/<app_id>/compose.yaml` avec persistance unifiée dans `./data/...`.
- **Pipeline d'ingestion & Audit qualité rigoureux (`steveos_nas_store`)** :
  - Ingestion automatisée depuis les collections amont (Portainer types 1 et 3) avec résolution des stacks Git distantes.
  - Récriture impérative des chemins hôtes arbitraires (`/portainer/...`, `/srv/docker/...`) vers des chemins relatifs fiables et propres `./data/...`.
  - Élimination des applications obsolètes ou abandonnées :
    - Test de conformité Docker Compose Specification via `docker compose config -q`.
    - Détection des dépôts 404 et des images Docker Hub non maintenues (dernière mise à jour antérieure à 3 ans sans mise à jour).
    - Filtrage des projets dépréciés ou archivés (`deprecated`, `unmaintained`, `archived`, `discontinued`, `eol`).
- **Piège critique du typage des ports dans les schémas de données (Serde Rust)** :
  - *Erreur passée* : Typer `default_port` en `u16` dans les structures de catalogue Rust.
  - *Impact* : Des templates tiers amont contenaient des erreurs de saisie (ex: `98000`, `82303`). Serde échouait silencieusement à désérialiser tout le catalogue `store.json`, provoquant un repli sur un catalogue vide (0 applications).
  - *Règle* : Toujours utiliser `u32` pour les ports dans les structures Rust Serde afin de tolérer toute valeur brute, et normaliser impérativement les ports dans la plage TCP valide (1-65535) lors de l'ingestion et de l'audit.
- **Alerte Bouclier Pare-feu 1-Clic (`🛡️`) dans la modale d'installation** :
  - NixOS bloquant tout port non déclaré par défaut, l'installation d'une application Docker nécessite l'ouverture de son port hôte pour un accès LAN.
  - Intégrer une carte d'alerte pare-feu interactive dans la fenêtre d'installation :
    - Détection en direct via `/api/firewall`.
    - Si ouvert : pastille verte rassurante (`🛡️ Port X (TCP) Ouvert`).
    - Si fermé : avertissement pédagogique jaune avec bouton d'action immédiate `🔓 Ouvrir le port X en 1 clic`.
    - L'action appelle `POST /api/firewall/rules` (`category: "Conteneurs"`), actualise le pare-feu et bascule l'alerte au vert sans interrompre l'installation.
- **Fluidité de rendu du Store (Slicing 48 applications)** :
  - Ne jamais insérer plus de 50 à 100 cartes complexes simultanément dans le DOM.
  - Appliquer un découpage par tranches de 48 items avec un bouton dynamique "Afficher plus d'applications (+48)", tout en appliquant les recherches, tris et filtres par catégories sur l'intégralité du catalogue (640+ applications) en mémoire.

---

### Toast Flottant de Déploiement Docker en Arrière-Plan (Dashboard) :
- Ne jamais laisser l'utilisateur bloqué dans une modale figée pendant la création de volumes, le pulling d'images et le démarrage d'une stack Compose.
- **Cycle de vie du toast flottant** :
  1. Dès le clic sur "Déployer", fermer immédiatement la modale de configuration et faire glisser un toast élégant en bas à droite (`.docker-deploy-floating-toast`, `z-index: 100050`).
  2. Animer une micro-jauge de progression séquentielle (20% -> 45% -> 75% -> 100%) synchronisée avec les étapes d'orchestration (volumes, variables d'environnement, compose up, vérification de l'état).
  3. À la complétion réussie (code HTTP 200) :
     - Passer la pastille en vert (`status-success`) avec indicateur lumineux `✓ Déploiement terminé`.
     - Générer un bouton d'accès direct `🚀 Ouvrir l'application` pointant directement vers l'URL locale `http://<ip>:<port>`.
     - Ajouter un bouton d'action secondaire `📦 Voir les conteneurs` pour naviguer vers l'onglet conteneurs.
     - Programmer une fermeture automatique après 12 secondes avec repli manuel par bouton croix `✕`.
  4. En cas d'échec : passer la jauge en rouge (`status-error`), afficher le message d'erreur retourné par le daemon Docker et maintenir le toast ouvert pour consultation.

---

### Noyau Linux LTS & Stabilité Système pour NAS (NixOS) :
- **Suivi permanent du dernier noyau stable LTS NixOS (`pkgs.linuxPackages`)** :
  - Dans l'écosystème NixOS, `pkgs.linuxPackages` représente le noyau stable par défaut sélectionné et validé par les mainteneurs NixOS (actuellement la branche Linux 6.18.x) pour laquelle l'ensemble de la distribution (OpenZFS 2.4.x, pilotes Nvidia, virtualisation KVM/VFIO, Docker) est officiellement compilée et garantie dans le cache binaire.
  - Le noyau amont de pointe (`pkgs.linuxPackages_latest`) est quant à lui sur les versions 7.x expérimentales, non recommandées pour un serveur de stockage.
- **Architecture adoptée dans STEvE_OS** :
  - Déclaration de l'option `steveos.boot.kernel` dans `modules/options.nix` (valeur par défaut : `"lts"`).
  - Module dédié `modules/core/kernel.nix` assignant `boot.kernelPackages = pkgs.linuxPackages;` par défaut via `mkDefault` lorsque `kernel = "lts"`.
  - Cela garantit que le NAS bénéficie automatiquement du **dernier noyau stable LTS en permanence** au fil des mises à jour Nixpkgs sans intervention manuelle.
  - Possibilité d'épinglage fixe sur des versions antérieures (`"6_12"`, `"6_6"`) ou vers le noyau amont (`"latest"`).
  - Contrôle utilisateur centralisé via `vars.nix` (`kernel = "lts"`), documenté dans `vars-defaults.nix`.

---

### Easter Egg Rétro & Animation Pixel-Art (Dashboard) :
- **Déclenchement & Fenêtre glissante** :
  - Détection de 5 clics consécutifs sur le logo de marque (`#header-logo-wrap`) avec une temporisation glissante de 2.5 secondes (`clearTimeout`).
  - Animation de micro-rebond (`logoClickBounce`) sur le logo à chaque clic pour donner un retour tactile immédiat.
- **Rendu Pixel-Art Vectoriel Optimisé (SVG RLE)** :
  - Créer des sprites de 28x28 pixels nets via `shape-rendering="crispEdges"` et `image-rendering="pixelated"`.
  - Fusionner les pixels horizontaux contigus de même couleur (`width="N"`) en encodage RLE pour diviser le poids du DOM par 5 tout en garantissant un rendu 100% vectoriel sans artefacts de flou.
  - Cycle de course à 4 frames synchronisées (`[data-frame="0..3"]`) avec foulées alternées, canne levée, sac en balancier et nuage de poussière.
- **Synthèse Sonore Web Audio API (Zero-Dependency)** :
  - Utiliser l'API native `AudioContext` (activée automatiquement grâce au geste utilisateur des 5 clics) pour synthétiser en direct des pas de course légers.
  - Alterner la fréquence (245Hz / 290Hz) et le filtrage passe-bande entre le pied gauche et le pied droit pour un effet sonore authentique et rythmé.
  - Jouer un tintement d'arrivée doux à la fin du sprint (5.8s) avant de nettoyer le conteneur du DOM.
- **Positionnement de la Bulle de Dialogue (Anti-clipping)** :
  - Lorsque la piste est située sur la bordure inférieure du header (`bottom: -2px`), positionner la bulle de dialogue **en-dessous** du personnage (`top: 58px; transform: translateX(-50%)`) avec une flèche pointant vers le haut, afin qu'elle flotte harmonieusement au-dessus des onglets de navigation sans jamais être tronquée par le haut de la fenêtre du navigateur.

---

### Montage Durable Déclaratif `/mnt` (`mounts.json` + `mounts.nix`) & Gestion des Flags :
- **Architecture de double effet (Runtime immédiat & Déclaratif NixOS)** :
  - Pour concilier accès immédiat sans redémarrage et persistance durable après reboot, le Dashboard applique :
    1. Montage direct via `mount -o <runtime_flags> <device> <target>` avec attribution automatique des droits `chown -R <user>:storage <target>` et `chmod 2775 <target>` (setgid).
    2. Écriture atomique dans `mounts.json` synchronisée sur `/var/lib/steveos/mounts.json`, le répertoire de config (`$STEVEOS_CONFIG_DIR/mounts.json`) et le clone de développement.
    3. Lecture déclarative native par le module NixOS `modules/storage/mounts.nix` (`builtins.fromJSON (builtins.readFile ../../mounts.json)`) qui génère dynamiquement `fileSystems."<mountPoint>"` et les règles `systemd.tmpfiles.rules`.
- **Piège critique des Flags de montage (`nofail` vs Kernel runtime)** :
  - *Piège* : Passer aveuglément l'option `nofail` dans la commande runtime `mount -o defaults,noatime,nofail /dev/... /mnt/...` peut provoquer une erreur `mount: wrong fs type, bad option, bad superblock` selon le système de fichiers ou la version du noyau, car `nofail` est un drapeau géré exclusivement par `/etc/fstab` et `systemd`.
  - *Règle* : Toujours assainir les options envoyées à la commande `mount` runtime en filtrant les options spécifiques à fstab/systemd (`nofail`, `x-systemd.*`, `auto`, `noauto`), tout en conservant `nofail` dans `mounts.json` pour NixOS.
  - **Importance vitale de `nofail` sur un NAS** : Évite absolument tout basculement bloquant du NAS en mode rescue emergency au démarrage si un disque externe USB, un disque de données secondaire ou une baie est débranché ou temporairement indisponible.
  - **Drapeau `noatime`** : Recommandé par défaut sur tous les montages NAS pour éliminer les écritures inutiles d'horodatage de lecture, réduisant drastiquement l'usure mécanique/flash et augmentant les débits.
  - **Drapeau `compress=zstd`** : Activé automatiquement pour les volumes Btrfs pour économiser 20% à 35% d'espace disque.

---

### Refonte Ergonomique de l'Onglet Stockage (Dashboard) :
- **Affichage en Lignes pleine largeur (`.storage-disk-row`)** :
  - Remplacer les cartes verticales volumineuses par des lignes épurées et lisibles contenant l'identifiant physique (`Slot M.2 NVMe`, `Baie 1 (SATA)`), le modèle, le numéro de série, la capacité, la télémétrie S.M.A.R.T. et thermique, ainsi que l'état d'alimentation.
- **Cadre dédié pour les Grappes RAID (`.raid-cluster-frame`)** :
  - Encadrer visuellement chaque grappe RAID avec une bordure Catppuccin Mocha douce et une section dédiée aux **disques physiques membres de la grappe** (`.raid-members-section`), reliés par des connecteurs d'arbres hiérarchiques (`├──`, `└──`).
### Résolution et Activation Automatique des Périphériques LVM/RAID (`src/storage.rs`) :
- Toujours encapsuler la détection des disques et le montage dans une fonction de résolution préalable :
```rust
fn resolve_and_activate_block_device(clean_dev: &str, req: &MountRequest) -> Result<String, String> {
    // 1. Détection des volumes LVM (/dev/<vg>/<lv> ou /dev/mapper/<vg>-<lv>)
    // 2. Activation automatique via `vgchange -ay <vg>` et synchronisation udev `udevadm settle`
    // 3. Résolution symlink et chemin canonique
    // 4. Validation physique d'existence : std::path::Path::new(&resolved).exists()
    // 5. En cas de non-existence physique, interrompre immédiatement SANS tenter de formater
}
```
- **Bannière d'erreur ergonomique (Toast interactif 10s+)** :
  - `#system-error-toast` flottant au premier plan absolu (`z-index: 100060 !important;`) avec accentuation rouge crimson et glassmorphism Catppuccin Mocha.
  - Temporisation minimale de 10 à 12 secondes avec mise en pause au survol de la souris (`mouseenter` / `mouseleave`).
  - Terminal de logs rétro-éclairé rétractable (`<pre><code>`) et bouton de copie en 1 clic (`📋 Copier`).

---

### Préservation des Données Existantes & Différenciation Montage vs Formatage :
- **Risque critique d'écrasement de données importées** :
  - *Problème* : Lorsqu'un utilisateur branche un volume RAID, un disque ou une partition provenant d'une autre machine (Debian, Synology, TrueNAS, unRAID, etc.), le système de fichiers n'est pas encore monté mais contient déjà toutes ses données. Un comportement automatisé qui tente de formater (`mkfs`) en l'absence de point de montage détruirait instantanément les données existantes.
  - *Règle architecturale adoptée* :
    1. **Détection proactive du système de fichiers réel** : Interroger systématiquement `blkid -o value -s TYPE <dev>` et `lsblk -no FSTYPE <dev>` pour afficher le filesystem réel (ex: `BTRFS`, `EXT4`, `XFS`, `NTFS`, `VFAT`) même lorsque le volume est démonté.
    2. **Séparation étanche dans l'API (`force_format: Option<bool>`)** : Le backend refuse catégoriquement d'exécuter `mkfs` sauf si le drapeau `force_format == Some(true)` est explicitement transmis. Si aucun système de fichiers n'est détecté et que `force_format != true`, le backend interrompt l'opération avec un message pédagogique clair invitant à utiliser l'option de formatage si le disque est neuf.
    3. **Ergonomie du Dashboard** : Deux boutons distincts sur chaque volume :
       - `📁 Monter sans formater` : Modalité douce avec bandeau vert "Préservation intégrale des données garantie", sélecteur de système de fichiers masqué et montage immédiat.
       - `⚠️ Formater le volume` : Modale d'alerte rouge avec avertissement destructif explicite, choix du filesystem (`mkfs.btrfs`, `mkfs.ext4`, `mkfs.xfs`), saisie du label et case à cocher de confirmation obligatoire.

---

### Gestion du Partitionnement Dynamique & Espaces Libres (`sfdisk` + `parted`) :
- **Détection des Blocs d'Espace Non Alloué** :
  - Calculer la différence entre `size_bytes` du disque physique et la somme des tailles des partitions allouées. Si le reliquat dépasse 50 Mo, synthétiser dynamiquement une partition virtuelle `is_free_space: true`.
- **Barre Visuelle Proportionnelle (`.disk-partition-bar`)** :
  - Segmenter visuellement chaque disque à 100% de sa largeur avec un ratio exact `(part_size / disk_size) * 100%`.
  - Appliquer des styles Catppuccin distinctifs : bleu pour le système NixOS, vert pour les volumes montés, mauve pour les partitions non montées, et hachures translucides pour l'espace libre non alloué.
- **Ajout de Partition sans collision de secteurs (`sfdisk --append`)** :
  - Utiliser `echo ",<size>MiB" | sfdisk --append <disk>` (ou `parted -s -a optimal <disk> mkpart primary ...` en fallback). Cela permet d'allouer automatiquement le premier bloc libre disponible sans devoir calculer manuellement les secteurs de début (`start`).
  - Suppression sécurisée via `sfdisk --delete <disk> <num>` précédée d'un démontage propre (`umount -f`) et nettoyage des signatures (`wipefs -a`).
  - Verrouillage absolu interdisant la suppression ou l'altération des partitions contenant `/`, `/boot` ou `/nix`.

---

### Périphériques Amovibles, Automounting & Éjection Sécurisée (USB, SSD Externes, Lecteurs Optiques) :
- **Intégration NixOS Udisks2 & Devmon** :
  - Déclaration de `services.udisks2.enable = true;` et `services.devmon.enable = true;` dans `modules/storage/default.nix` complété par `parted`, `eject`, `dosfstools`, `exfatprogs`, `ntfs3g`, `udisks2`.
  - `udisks2` seul fournit l'API D-Bus sans monter automatiquement les médias à l'insertion. L'activation de `services.devmon.enable = true;` assure le montage automatique transparent sous `/media/`.
  - Conforme à la Règle n°2 de discernement : outils CLI purs sans dépendance `nix-ld`.
- **Scan consolidé (`scan_removable_devices`)** :
  - Filtrage `lsblk -b --json` sur `tran == "usb"`, `rm == true`, `hotplug == true` et les lecteurs optiques (`type == "rom"` ou nom `sr*`).
- **Modale de Montage Dédiée & Permissions Non-POSIX (exFAT, FAT32, NTFS)** :
  - Pour les périphériques démontés ou dont le montage automatique a échoué, modale `#modal-mount-removable` intuitive avec suggestion automatique de `/media/<LABEL>` ou `/media/<NOM>`.
  - **Piège critique des systèmes de fichiers non-POSIX** : sur FAT32/exFAT/NTFS, `chown` échoue silencieusement. Le backend Rust résout l'UID de l'utilisateur principal et le GID du groupe `storage`, injectant automatiquement `uid=...,gid=...,dmask=0002,fmask=0113` pour garantir un accès en lecture et écriture sans blocage root.
  - Option de persistance désactivée par défaut pour éviter de polluer `mounts.json` avec des clés USB temporaires.
- **Accès Fichiers 1-Clic (`openFilesAtPath`)** :
  - Navigation instantanée vers `#files` avec mémorisation du chemin actif pour parcourir les dossiers du média amovible.
- **Éjection physique et logicielle propre** :
  - Synchronisation des buffers (`sync`), démontage de toutes les partitions associées (`umount -f`).
  - Pour les lecteurs optiques : ouverture mécanique du tiroir via `eject <dev>`.
  - Pour les périphériques USB : coupure de l'alimentation électrique du port via `udisksctl power-off -b <dev>` (avec fallback `eject`).
  - Déclenchement d'un toast flottant `#safe-removal-toast` avec compte à rebours de 10 secondes : *« Votre périphérique [nom] peut être déconnecté en toute sécurité »*.

---

### Gestion Déclarative du DNS, Libération du Port 53 & Fallbacks Anti-Coupure (`dns.json` + `dns.nix`) :
- **Architecture de Passerelle Déclarative (`dns.json` -> `modules/core/dns.nix`)** :
  - Le Dashboard Web applique le changement DNS à chaud (runtime `/etc/resolv.conf`) et persiste la sélection dans `dns.json`.
  - Le module NixOS `modules/core/dns.nix` lit `dns.json` et configure déclarativement `networking.nameservers`.
- **Résolution définitive du conflit sur le port 53 (AdGuard Home / Pi-hole)** :
  - *Piège historique* : `systemd-resolved` écoute par défaut sur `127.0.0.53:53`, ce qui empêche Docker de lier `0.0.0.0:53` lors du déploiement d'AdGuard Home ou Pi-hole (`address already in use`).
  - *Syntaxe moderne Nixpkgs (24.11+)* : Ne plus utiliser `services.resolved.extraConfig` (déprécié), utiliser impérativement :
    ```nix
    services.resolved.settings.Resolve.DNSStubListener = "no";
    ```
- **Règle absolue du Fallback DNS Anti-Coupure** :
  - En cas d'utilisation d'un DNS personnalisé local (conteneur AdGuard ou Pi-hole sur `127.0.0.1` ou IP LAN), TOUJOURS adjoindre un ou deux serveurs de secours distants stables (Cloudflare `1.1.1.1` et Quad9 `9.9.9.9`).
  - Cela évite toute perte de connectivité Internet sur le NAS (mises à jour, git, nixpkgs) en cas d'arrêt, de redémarrage ou de crash du conteneur DNS.
- **Mesure de Latence Ping en Direct** :
  - Probe TCP rapide sur le port 53 avec fallback ICMP ping pour afficher la latence réelle de chaque résolveur (Cloudflare, Quad9, AdGuard DNS, Google, Mullvad) dans l'interface Catppuccin Mocha.

---

### Standardisation des Templates Docker Compose (`steveos_nas_store`) :

- **Format officiel obligatoire des fichiers `compose.yaml`** :
  Tout template Docker Compose publié sur [`Chomiam/steveos_nas_store`](https://github.com/Chomiam/steveos_nas_store) doit obligatoirement intégrer le cartouche de métadonnées officiel en en-tête et le bloc de correspondance NixOS en pied de page :
  ```yaml
  # =========================================================================
  # 🐳 STEvE_OS NAS Edition — Configuration Docker Compose
  # Application  : <Nom de l'application> (<app_id>)
  # Port hôte    : <port_principal_web>
  # Données hôte : /home/{user}/docker/<app_id>
  # Mode         : Production STEvE_OS Store
  # =========================================================================

  version: "3.8"

  services:
    <app_id>:
      image: <image_officielle>:<tag>
      container_name: <app_id>
      restart: unless-stopped
      ports:
        - "<port_web>:<port_web>/tcp"
        # Spécifier /tcp et /udp si nécessaire (ex: DNS sur port 53)
      environment:
        - TZ=Europe/Paris
        - PUID=1000
        - PGID=100
      volumes:
        - /home/{user}/docker/<app_id>/<dossier>:<chemin_conteneur>

  # =========================================================================
  # ❄️ Équivalent Déclaration NixOS (/etc/nixos/docker/<app_id>.nix)
  # =========================================================================
  ```
- **Définition cohérente du port par défaut (`default_port`)** :
  - Dans `manifest.json` et `store.json`, `default_port` doit toujours correspondre au port de l'interface utilisateur accessible dans le navigateur web (ex: `3000` pour AdGuard Home, et NON le port DNS `53`).
---

### Moteur de Tâches Asynchrones Persistantes, Dissolution & Assemblage RAID avec Reprise sur Reboot :
- **Problématique & Besoin métier** :
  - La dissolution (« casser ») ou la création (« assembler ») d'une grappe RAID sont des opérations longues et critiques (démontage, wipefs, formatage, synchronisation mdadm/LVM).
  - Ces opérations ne doivent JAMAIS dépendre d'une connexion HTTP synchrone (timeout navigateur, coupure réseau, fermeture de session).
  - En cas de coupure électrique, redémarrage du NAS ou mise à jour système NixOS en plein milieu d'une opération, l'action doit **reprendre automatiquement là où elle s'est arrêtée** sans abandonner le stockage dans un état inconsistant.
- **Architecture de Persistance & Machine à États Finis (`StorageJob`)** :
  - **Fichier d'état sur disque** : `/var/lib/steveos/storage_jobs.json` (persistant à travers les boots et les mises à jour NixOS).
  - **Modèle `StorageJob`** : stocke `id`, `job_type` (`destroy_raid` ou `create_raid`), `status` (`running`, `resumed`, `completed`, `failed`), `current_step`, `total_steps`, `progress_percent`, `step_name`, `step_detail`, disques cibles et paramètres.
  - **Machine à états pour la Dissolution (`destroy_raid`)** :
    1. *Étape 1 (15%)* : Vérification sécurité (`is_system_device`) & Démontage forcé propre (`umount -f`).
    2. *Étape 2 (30%)* : Nettoyage déclaratif des points de montage (`mounts.json`).
    3. *Étape 3 (55%)* : Suppression logique (`lvremove`/`vgremove` LVM2 ou `mdadm --stop`).
    4. *Étape 4 (75%)* : Nettoyage métadonnées physiques (`pvremove -y -ff` ou `mdadm --zero-superblock --force`).
    5. *Étape 5 (90%)* : Effacement bas niveau des signatures (`wipefs -a -f`).
    6. *Étape 6 (100%)* : Synchronisation du système de blocs (`vgmknodes`, `udevadm settle`).
  - **Machine à états pour l'Assemblage (`create_raid`)** :
    1. *Étape 1 (15%)* : Nettoyage préliminaire des disques (`umount`, `wipefs`, `zero-superblock`).
    2. *Étape 2 (40%)* : Chargement modules noyau (`modprobe raid*`) et création grappe (`mdadm --create --run`).
    3. *Étape 3 (65%)* : Attente stabilisation et formatage filesystem (`mkfs.ext4`, `mkfs.btrfs` ou `mkfs.xfs`).
    4. *Étape 4 (85%)* : Montage (`mkdir -p`, `mount`) et configuration des droits (`chown`, `chmod 2775`).
    5. *Étape 5 (100%)* : Inscription déclarative (`mounts.json`) et synchronisation udev.
- **Reprise Idempotente au Démarrage (`init_storage_tasks_tracker`)** :
  - Appelé au lancement de `main.rs` : si une tâche `running` ou `resumed` est trouvée dans `storage_jobs.json`, elle est marquée `resumed` et un worker de reprise est détaché immédiatement en tâche de fond.
  - Chaque étape est testée de manière idempotente (ne plante pas si une ressource a déjà été supprimée ou formatée avant la coupure).
- **Double Feedback Visuel Temps Réel (Catppuccin Mocha)** :
  - **Toast flottant persistant en bas d'écran (`#storage-floating-toast`)** : visible sur tous les onglets du NAS, avec jauge animée en pourcentage, icône active (`🧨` pulsant ou `🛠️` rotatif), phase courante et bouton direct d'accès vers Stockage.
  - **Panneau dédié dans l'onglet Stockage (`#storage-raid-job-panel`)** : stepper visuel des étapes franchies (✔) et en cours (⏳), détails des disques cibles et statut de reprise.
  - **Sondage dynamique & Détection au boot** : appel automatique de `checkActiveStorageJobOnLoad()` au chargement du Dashboard pour afficher instantanément la progression sans manipulation utilisateur.

---

### Assistant de Création RAID Pro & Visualisation Graphique des Baies :
- **Affichage des Disques en Lignes (Row-Style) vs Grilles** :
  - Bannir les grilles étriquées pour la sélection des périphériques de stockage : chaque disque doit disposer de sa propre carte horizontale (`.raid-disk-row-card`) avec toggle complet au clic, bus matériel (`SATA 6Gb/s`, `NVMe PCIe`), modèle constructeur, capacité exacte et état de disponibilité.
- **Rendu Visuel Dynamique des Baies et Blocs de Parité** :
  - Représentation sous forme de rack de serveur NAS (`.raid-disk-rack-visual`) : chaque disque physique sélectionné simule un tiroir caddy avec LED d'état et un empilement vertical de 4 blocs logiques.
  - Découpage visuel précis selon le niveau de RAID :
    - *RAID 0* : 100% Blocs de données (`Data A1..A4`), 0% parité.
    - *RAID 1* : Disque 1 en Données, Disques 2..N en Miroir exact (`Mirror M1..M4`).
    - *RAID 5* : Blocs distribués en diagonale/damier (3x Données + 1x Parité `P1..P4` alternée par disque).
    - *RAID 6* : Double parité distribuée (`Data`, `Parité P`, `Parité Q`).
    - *RAID 10* : Paires en miroir agrégées (`Pair 1: D1+M1`, `Pair 2: D2+M2`).
    - *JBOD/Linéaire* : Blocs continus sans striping.
  - **Emplacements Fantômes (Ghost Slots)** : Si le nombre de disques cochés est inférieur au minimum requis par l'algorithme (ex: 2 disques pour RAID 5 qui en demande 3), afficher des baies fantômes en pointillés animés (`+ Disque Requis`) pour guider visuellement l'utilisateur.
- **Sélecteur Comparatif de Systèmes de Fichiers (Btrfs, XFS, Ext4)** :
  - Présentation sous forme de cartes interactives (`.fs-type-card`) détaillant les points forts et contraintes de chaque moteur :
    - *Btrfs* : Recommandé STEvE_OS (Snapshots instantanés CoW, auto-guérison bitrot, compression zstd) / Plus exigeant en RAM.
    - *XFS* : Recommandé pour gros débits et fichiers volumineux (multimédia 4K, ISO, scalabilité multithreadée) / Pas de shrink possible.
    - *Ext4* : Standard historique Linux (légèreté absolue, fiabilité universelle, fsck rapide) / Pas de CoW ni de snapshots natifs.

---

### Studio de Création RAID Widescreen (1360px) & Matrice Stylisée des Niveaux de Redondance :
- **Architecture de Disposition 2 Colonnes (Split Studio)** :
  - *Problème des modales étroites 1 colonne (900px)* : Empiler séquentiellement les formulaires, le choix du niveau RAID, la liste des disques, le schéma graphique et les systèmes de fichiers force l'utilisateur à un défilement vertical fastidieux, reléguant le schéma visuel sous la ligne de flottaison.
  - *Solution Widescreen (1360px)* :
    - `.raid-creator-pro-window` avec largeur panoramique (`1360px !important`, `height: 88vh`, `max-height: 90vh`).
    - Grille CSS 2 colonnes (`.raid-studio-grid` : `1.15fr 0.95fr`) :
      - **Colonne gauche** (`.raid-studio-col-left`) : Paramètres d'accès (nom, point de montage), matrice stylisée des niveaux de RAID, sélection des disques physiques en lignes compactes et sélecteur de système de fichiers (Btrfs, XFS, Ext4).
      - **Colonne droite (Cockpit visuel temps réel)** (`.raid-studio-col-right`) : Carte sticky contenant le schéma dynamique de la baie (`.raid-disk-rack-visual`), la jauge capacitaire bicolore (Données utiles vs Parité), les métriques modulaires (Nette, Parité, Brute) et la fiche de synthèse technique instantanée.
    - Avantage UX : Toute modification à gauche (clic sur un niveau RAID, coche d'un disque, saisie de nom) actualise instantanément le cockpit de droite sous les yeux de l'utilisateur sans aucun scroll.
- **Matrice Stylisée des Niveaux de RAID (Élimination des blocs ternes)** :
  - Abandonner les cartes grises monolithiques encombrées de longs paragraphes.
  - Structurer chaque profil en micro-carte interactive (`.raid-level-card`) dans une grille 3x2 :
    - Accents colorés néon Catppuccin Mocha spécifiques :
      - *RAID 5* : Vert Émeraude (`--green`), badge `N - 1`, mention `🛡️ 1 panne tolérée`.
      - *RAID 1* : Bleu Saphir (`--blue`), badge `50%`, mention `🛡️ Tolère 1 sur 2`.
      - *RAID 6* : Pêche Ambré (`--peach`), badge `N - 2`, mention `🛡️🛡️ 2 pannes simultanées`.
      - *RAID 10* : Mauve Néon (`--mauve`), badge `50%`, mention `⚡ 1 panne par paire`.
      - *RAID 0* : Rouge Cramoisi (`--red`), badge `100%`, mention `⚠️ 0 tolérance aux pannes`.
      - *JBOD* : Titane / Subtext (`--subtext0`), badge `100%`, mention `📦 Pas de striping`.
    - Indicateur radio personnalisé à point lumineux radial (`.raid-radio-glow`) qui s'allume au clic dans la couleur signature du profil avec halo lumineux et bordure active.
    - Synchronisation automatique avec la jauge, la baie des disques et les messages d'aide contextuelle.

---

## 🔄 4. Protocole d'Actualisation Continue de ce Fichier


À chaque fois qu'un bogue est résolu, qu'un écueil est identifié ou qu'une nouvelle architecture est introduite :
1. Ajouter l'anomalie et la solution dans la section **2. Pièges Critiques**.
2. Documenter la recette validée dans la section **3. Patterns Recommandés**.
3. Conserver un historique clair pour éviter toute régression sur les versions futures.
