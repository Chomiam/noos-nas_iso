let selectedDisk = null;
let selectedFs = "btrfs";
let isUserValid = false;
let currentIp = "127.0.0.1";
let sseSource = null;
let pollTimer = null;
let countdownTimer = null;
let isUpdating = false;

document.addEventListener("DOMContentLoaded", () => {
  fetchNetworkInfo();
  checkUpdates(false);
  loadDisks();
  checkCurrentStatus();
});

async function checkUpdates(manual = false) {
  const modal = document.getElementById("update-modal");
  const spinner = document.getElementById("update-spinner");
  const iconDone = document.getElementById("update-icon-done");
  const iconWarn = document.getElementById("update-icon-warn");
  const title = document.getElementById("update-title");
  const desc = document.getElementById("update-desc");
  const commitsBox = document.getElementById("update-commits-box");
  const btnApply = document.getElementById("update-actions");
  const statusText = document.getElementById("update-status-text");

  if (manual && modal) {
    modal.style.display = "flex";
    modal.style.opacity = "1";
    spinner.style.display = "block";
    iconDone.style.display = "none";
    iconWarn.style.display = "none";
    commitsBox.style.display = "none";
    btnApply.style.display = "none";
    title.textContent = "Vérification des mises à jour...";
    desc.textContent = "Interrogation du dépôt GitHub Chomiam/steveos-nas_iso...";
  }

  try {
    const res = await fetch("/api/update/check");
    if (!res.ok) throw new Error("Erreur HTTP lors de la vérification");
    const data = await res.json();

    if (data.update_available) {
      if (modal) {
        modal.style.display = "flex";
        modal.style.opacity = "1";
        spinner.style.display = "none";
        iconWarn.style.display = "block";
        title.textContent = "Mise à jour disponible !";
        desc.textContent = "Une nouvelle version de l'installateur a été détectée sur GitHub. Mise à jour et redémarrage automatique en cours...";
        
        document.getElementById("commit-current").textContent = data.current_short;
        document.getElementById("commit-remote").textContent = data.remote_short;
        commitsBox.style.display = "flex";
        btnApply.style.display = "none";
      }

      if (statusText) statusText.textContent = `MàJ : ${data.current_short} ➔ ${data.remote_short}`;

      // Lancement automatique de la mise à jour
      setTimeout(() => {
        applyUpdate();
      }, 1000);

    } else {
      if (statusText) statusText.textContent = `À jour (${data.current_short})`;

      if (modal && modal.style.display !== "none") {
        spinner.style.display = "none";
        iconDone.style.display = "block";
        title.textContent = "Installateur à jour";
        desc.textContent = data.message || "Vous disposez de la dernière version du dépôt steveos-nas_iso.";
        
        setTimeout(() => {
          modal.style.opacity = "0";
          setTimeout(() => { modal.style.display = "none"; }, 300);
        }, 1200);
      }
    }
  } catch (err) {
    console.warn("Update check failed:", err);
    if (statusText) statusText.textContent = "MàJ : Hors-ligne";

    if (modal && modal.style.display !== "none") {
      spinner.style.display = "none";
      iconWarn.style.display = "block";
      title.textContent = "Vérification hors-ligne";
      desc.textContent = "Impossible de contacter GitHub pour vérifier les mises à jour. Poursuite de l'installation avec la version embarquée.";
      
      setTimeout(() => {
        modal.style.opacity = "0";
        setTimeout(() => { modal.style.display = "none"; }, 300);
      }, 1500);
    }
  }
}

async function applyUpdate() {
  if (isUpdating) return;
  isUpdating = true;

  const modal = document.getElementById("update-modal");
  const spinner = document.getElementById("update-spinner");
  const iconDone = document.getElementById("update-icon-done");
  const iconWarn = document.getElementById("update-icon-warn");
  const title = document.getElementById("update-title");
  const desc = document.getElementById("update-desc");
  const btnApply = document.getElementById("update-actions");

  if (modal) {
    modal.style.display = "flex";
    modal.style.opacity = "1";
    spinner.style.display = "block";
    iconDone.style.display = "none";
    iconWarn.style.display = "none";
    btnApply.style.display = "none";
    title.textContent = "Mise à jour en cours...";
    desc.textContent = "Téléchargement, compilation de la nouvelle version et redémarrage de l'installateur...";
  }

  try {
    const res = await fetch("/api/update/apply", { method: "POST" });
    const data = await res.json();
    if (!res.ok || !data.ok) {
      throw new Error(data.message || "Échec de l'application de la mise à jour");
    }

    desc.textContent = "Redémarrage du service en cours... Rechargement automatique de la page...";
    
    // Attendre 3 secondes puis poller le serveur jusqu'à ce qu'il réponde
    setTimeout(() => {
      const reloadInterval = setInterval(async () => {
        try {
          const check = await fetch("/api/network", { cache: "no-store" });
          if (check.ok) {
            clearInterval(reloadInterval);
            window.location.reload();
          }
        } catch {
          // Serveur en cours de redémarrage
        }
      }, 1500);
    }, 3000);

  } catch (err) {
    isUpdating = false;
    alert("Erreur de mise à jour : " + err.message);
    if (modal) modal.style.display = "none";
  }
}


async function fetchNetworkInfo() {
  try {
    const res = await fetch("/api/network");
    if (res.ok) {
      const data = await res.json();
      currentIp = data.ip;
      document.getElementById("header-ip-badge").textContent = `IP : http://${currentIp}:8080`;
      const urlPreview = document.getElementById("final-dashboard-url");
      if (urlPreview) {
        urlPreview.textContent = `http://${currentIp}:9339`;
      }
    }
  } catch (err) {
    console.error("Failed to load network info", err);
  }
}

async function loadDisks() {
  const container = document.getElementById("disks-container");
  container.innerHTML = `
    <div class="loading-state">
      <div class="spinner"></div>
      <span>Détection des disques disponibles...</span>
    </div>
  `;

  try {
    const res = await fetch("/api/disks");
    if (!res.ok) throw new Error("Erreur de récupération des disques");
    const disks = await res.json();

    if (!disks || disks.length === 0) {
      container.innerHTML = `
        <div class="info-callout" style="border-color: var(--yellow);">
          <div class="info-icon">⚠️</div>
          <div class="info-content">
            <strong>Aucun disque compatible détecté :</strong><br>
            Vérifiez les branchements de vos disques SATA ou NVMe et cliquez sur Rafraîchir.
          </div>
        </div>
      `;
      return;
    }

    container.innerHTML = "";
    selectedDisk = null;

    disks.forEach((d, idx) => {
      const card = document.createElement("div");
      card.className = `disk-card ${d.is_install_media ? "disabled" : ""}`;
      card.id = `disk-${d.name}`;

      const icon = d.transport.toLowerCase().includes("nvme") ? "⚡" : (d.is_rotational ? "💽" : "💾");

      card.innerHTML = `
        <div class="disk-card-left">
          <div class="disk-icon">${icon}</div>
          <div class="disk-details">
            <div class="disk-name-row">
              <span class="disk-path">${d.path}</span>
              ${d.is_install_media ? '<span class="badge badge-live">Clé USB / Live ISO (Protégé)</span>' : ''}
            </div>
            <div class="disk-model">${d.model || "Disque standard"}</div>
            <div class="disk-badges">
              <span class="badge badge-bus">${d.transport}</span>
              <span class="badge badge-type">${d.is_rotational ? "HDD" : "SSD"}</span>
              ${d.partitions_count > 0 ? `<span class="badge badge-type">${d.partitions_count} partition(s)</span>` : ''}
            </div>
          </div>
        </div>
        <div class="disk-card-right">
          <div class="disk-size">${d.size_human}</div>
          <input type="radio" name="disk-selection" class="disk-radio" ${d.is_install_media ? "disabled" : ""} value="${d.path}">
        </div>
      `;

      if (!d.is_install_media) {
        card.onclick = () => selectDisk(d, card);
        // Sélectionner par défaut le premier disque non-live
        if (!selectedDisk) {
          selectDisk(d, card);
        }
      }

      container.appendChild(card);
    });

  } catch (err) {
    container.innerHTML = `
      <div class="info-callout" style="border-color: var(--red);">
        <div class="info-icon">❌</div>
        <div class="info-content">Erreur lors de la détection des disques : ${err.message}</div>
      </div>
    `;
  }
}

function selectDisk(disk, element) {
  selectedDisk = disk;
  document.querySelectorAll(".disk-card").forEach(c => c.classList.remove("selected"));
  element.classList.add("selected");
  const radio = element.querySelector(".disk-radio");
  if (radio) radio.checked = true;
}

function selectFilesystem(fs, element) {
  selectedFs = fs;
  document.querySelectorAll(".fs-card").forEach(c => c.classList.remove("selected"));
  element.classList.add("selected");
  const radio = element.querySelector('input[type="radio"]');
  if (radio) radio.checked = true;
}

async function checkUsernameLive() {
  const input = document.getElementById("username");
  const icon = document.getElementById("user-val-icon");
  const hint = document.getElementById("user-hint");
  const val = input.value.trim();

  if (!val) {
    input.classList.remove("is-valid", "is-invalid");
    icon.textContent = "";
    hint.className = "input-hint";
    hint.textContent = "Minuscules (a-z), chiffres (0-9), tiret (-) et souligné (_). Pas d'espaces ni d'accents.";
    isUserValid = false;
    return;
  }

  // Client regex check
  const posixRegex = /^[a-z_][a-z0-9_-]{0,31}$/;
  if (!posixRegex.test(val)) {
    input.classList.add("is-invalid");
    input.classList.remove("is-valid");
    icon.textContent = "❌";
    hint.className = "input-hint error";

    if (/[A-Z]/.test(val)) {
      hint.textContent = "Les majuscules sont interdites dans le nom d'utilisateur.";
    } else if (/\s/.test(val)) {
      hint.textContent = "Les espaces sont strictement interdits.";
    } else if (/^[0-9-]/.test(val)) {
      hint.textContent = "Le nom d'utilisateur doit commencer par une lettre minuscule ou un souligné (_).";
    } else {
      hint.textContent = "Caractère interdit détecté. Seules lettres minuscules, chiffres, - et _ sont admis.";
    }
    isUserValid = false;
    return;
  }

  // Server check for reserved names
  try {
    const res = await fetch("/api/validate-user", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: val })
    });
    const data = await res.json();
    if (data.valid) {
      input.classList.remove("is-invalid");
      input.classList.add("is-valid");
      icon.textContent = "✅";
      hint.className = "input-hint";
      hint.textContent = "Nom d'utilisateur valide et conforme.";
      isUserValid = true;
    } else {
      input.classList.add("is-invalid");
      input.classList.remove("is-valid");
      icon.textContent = "❌";
      hint.className = "input-hint error";
      hint.textContent = data.error || "Nom d'utilisateur invalide.";
      isUserValid = false;
    }
  } catch {
    isUserValid = true;
  }
}

function checkPasswordMatch() {
  const p1 = document.getElementById("password").value;
  const p2 = document.getElementById("password-confirm").value;
  const hint = document.getElementById("pwd-hint");

  if (!p2) {
    hint.textContent = "";
    return true;
  }

  if (p1 !== p2) {
    hint.className = "input-hint error";
    hint.textContent = "Les mots de passe ne correspondent pas.";
    return false;
  } else if (p1.length < 4) {
    hint.className = "input-hint error";
    hint.textContent = "Le mot de passe doit comporter au moins 4 caractères.";
    return false;
  } else {
    hint.className = "input-hint";
    hint.textContent = "Les mots de passe correspondent.";
    return true;
  }
}

function togglePasswordVisibility(inputId, btn) {
  const input = document.getElementById(inputId);
  if (input.type === "password") {
    input.type = "text";
    btn.textContent = "🙈";
  } else {
    input.type = "password";
    btn.textContent = "👁️";
  }
}

async function confirmAndStartInstall() {
  const username = document.getElementById("username").value.trim();
  const hostname = document.getElementById("hostname").value.trim() || "steveos-nas";
  const password = document.getElementById("password").value;
  const passwordConfirm = document.getElementById("password-confirm").value;

  if (!username || !isUserValid) {
    alert("Veuillez saisir un nom d'utilisateur valide.");
    document.getElementById("username").focus();
    return;
  }

  if (!password || password.length < 4) {
    alert("Veuillez saisir un mot de passe d'au moins 4 caractères.");
    document.getElementById("password").focus();
    return;
  }

  if (password !== passwordConfirm) {
    alert("Les mots de passe ne correspondent pas.");
    document.getElementById("password-confirm").focus();
    return;
  }

  if (!selectedDisk) {
    alert("Veuillez sélectionner un disque cible pour installer le système.");
    return;
  }

  const confirmMsg = `⚠️ ATTENTION : DESTRUCTION DES DONNÉES\n\n` +
    `Le disque suivant sera entièrement formaté pour STEvE_OS NAS :\n` +
    `• Disque : ${selectedDisk.path} (${selectedDisk.model})\n` +
    `• Capacité : ${selectedDisk.size_human}\n` +
    `• Système de fichiers : ${selectedFs.toUpperCase()}\n` +
    `• Utilisateur : ${username}\n\n` +
    `Toutes les partitions existantes sur ce disque seront EFFACÉES.\n` +
    `Êtes-vous certain de vouloir continuer ?`;

  if (!confirm(confirmMsg)) {
    return;
  }

  const btn = document.getElementById("btn-start-install");
  btn.disabled = true;
  btn.textContent = "Démarrage en cours...";

  try {
    const res = await fetch("/api/install", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        username,
        hostname,
        password,
        disk_path: selectedDisk.path,
        filesystem: selectedFs
      })
    });

    const data = await res.json();
    if (!res.ok || !data.ok) {
      alert("Erreur : " + (data.message || "Impossible de démarrer l'installation"));
      btn.disabled = false;
      btn.textContent = "🚀 Démarrer l'Installation de STEvE_OS NAS";
      return;
    }

    // Basculer vers l'écran de progression
    switchToProgressView();
  } catch (err) {
    alert("Erreur de communication : " + err.message);
    btn.disabled = false;
    btn.textContent = "🚀 Démarrer l'Installation de STEvE_OS NAS";
  }
}

function switchToProgressView() {
  document.getElementById("form-card").style.display = "none";
  document.getElementById("progress-card").style.display = "flex";
  connectLogsStream();
  startStatusPolling();
}

function connectLogsStream() {
  if (sseSource) sseSource.close();

  const terminal = document.getElementById("terminal-body");
  sseSource = new EventSource("/api/install/stream");

  sseSource.onmessage = (event) => {
    appendTerminalLine(event.data);
  };

  sseSource.onerror = () => {
    // Reconnexion automatique gérée par EventSource
  };
}

function appendTerminalLine(text) {
  const terminal = document.getElementById("terminal-body");
  const line = document.createElement("div");
  line.className = "term-line";

  if (text.startsWith("[PROGRESS]")) {
    line.className += " term-progress";
    // Mettre à jour la jauge
    const match = text.match(/\[PROGRESS\]\s+(\d+)%\s+-\s+(.+)/);
    if (match) {
      const perc = parseInt(match[1], 10);
      const stepText = match[2];
      updateProgressBar(perc, stepText);
    }
  } else if (text.startsWith("[STDERR]")) {
    line.className += " term-stderr";
  } else if (text.includes("❌") || text.includes("ÉCHEC") || text.includes("Erreur")) {
    line.className += " term-error";
  } else if (text.includes("🎉") || text.includes("succès") || text.includes("Félicitations")) {
    line.className += " term-success";
  } else if (text.startsWith("🚀") || text.startsWith("---")) {
    line.className += " term-system";
  }

  line.textContent = text;
  terminal.appendChild(line);
  terminal.scrollTop = terminal.scrollHeight;
}

function updateProgressBar(percentage, stepDesc) {
  document.getElementById("progress-bar-fill").style.width = `${percentage}%`;
  document.getElementById("progress-perc-text").textContent = `${percentage}%`;
  if (stepDesc) {
    document.getElementById("current-step-desc").textContent = stepDesc;
  }
}

function startStatusPolling() {
  if (pollTimer) clearInterval(pollTimer);
  pollTimer = setInterval(async () => {
    try {
      const res = await fetch("/api/status");
      if (res.ok) {
        const data = await res.json();
        updateProgressBar(data.progress, data.step);

        if (data.status === "success") {
          clearInterval(pollTimer);
          showSuccessScreen();
        } else if (data.status === "error") {
          clearInterval(pollTimer);
          document.getElementById("terminal-status-badge").textContent = "ERREUR";
          document.getElementById("terminal-status-badge").style.borderColor = "var(--red)";
          document.getElementById("terminal-status-badge").style.color = "var(--red)";
        }
      }
    } catch (err) {
      console.error("Polling status error", err);
    }
  }, 2000);
}

async function checkCurrentStatus() {
  try {
    const res = await fetch("/api/status");
    if (res.ok) {
      const data = await res.json();
      if (data.status === "running") {
        switchToProgressView();
      } else if (data.status === "success") {
        showSuccessScreen();
      }
    }
  } catch {}
}

function showSuccessScreen() {
  document.getElementById("progress-card").style.display = "none";
  document.getElementById("success-card").style.display = "flex";

  const urlPreview = document.getElementById("final-dashboard-url");
  if (urlPreview) {
    urlPreview.textContent = `http://${currentIp}:9339`;
  }

  // Lancer le compte à rebours de 15 secondes
  let seconds = 15;
  const cdElement = document.getElementById("reboot-countdown");
  if (countdownTimer) clearInterval(countdownTimer);

  countdownTimer = setInterval(() => {
    seconds -= 1;
    if (cdElement) cdElement.textContent = seconds;
    if (seconds <= 0) {
      clearInterval(countdownTimer);
      rebootNow();
    }
  }, 1000);
}

async function rebootNow() {
  if (countdownTimer) clearInterval(countdownTimer);
  const cdElement = document.getElementById("reboot-countdown");
  if (cdElement) cdElement.textContent = "Redémarrage...";

  try {
    await fetch("/api/reboot", { method: "POST" });
  } catch {}

  alert("Le NAS redémarre. Dès le redémarrage terminé, ouvrez http://" + currentIp + ":9339 dans votre navigateur.");
}
