{ pkgs, lib }:

pkgs.stdenvNoCC.mkDerivation {
  pname = "noos-catppuccin-grub-theme";
  version = "1.0.0";

  src = pkgs.catppuccin-grub;

  dontBuild = true;

  installPhase = ''
    mkdir -p $out/icons
    cp -r $src/* $out/
    chmod -R u+w $out

    # Remplacer le logo, le fond d'écran et ajouter les icônes Noos
    cp ${./assets/logo.png} $out/logo.png
    cp ${./assets/background.png} $out/background.png
    cp ${./assets/installer.png} $out/icons/installer.png
    cp ${./assets/steveos.png} $out/icons/steveos.png
    cp ${./assets/noos.png} $out/icons/noos.png

    # Remplacer theme.txt avec les positions optimisées pour le logo Noos
    cp ${./theme.txt} $out/theme.txt
  '';
}
