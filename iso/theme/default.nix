{ pkgs, lib }:

pkgs.stdenvNoCC.mkDerivation {
  pname = "steveos-catppuccin-grub-theme";
  version = "1.0.0";

  src = pkgs.catppuccin-grub;

  dontBuild = true;

  installPhase = ''
    mkdir -p $out/icons
    cp -r $src/* $out/
    chmod -R u+w $out

    # Remplacer le logo, le fond d'écran et ajouter les icônes STEvE_OS
    cp ${./assets/logo.png} $out/logo.png
    cp ${./assets/background.png} $out/background.png
    cp ${./assets/installer.png} $out/icons/installer.png
    cp ${./assets/steveos.png} $out/icons/steveos.png

    # Remplacer theme.txt avec les positions optimisées pour le logo STEvE_OS
    cp ${./theme.txt} $out/theme.txt
  '';
}
