{
  description = "Noos NAS Edition - Image ISO d'installation réseau";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-26.05";
  };

  outputs = { self, nixpkgs, ... }@inputs:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Configuration du système ISO
      isoConfig = nixpkgs.lib.nixosSystem {
        inherit system;
        specialArgs = { inherit inputs self; };
        modules = [
          "${nixpkgs}/nixos/modules/installer/cd-dvd/installation-cd-minimal.nix"
          ./iso/configuration.nix
        ];
      };
    in
    {
      nixosConfigurations.iso = isoConfig;

      packages.${system} = {
        web-installer = pkgs.rustPlatform.buildRustPackage {
          pname = "noos-web-installer";
          version = "0.1.0";
          src = ./web-installer;
          cargoLock = {
            lockFile = ./web-installer/Cargo.lock;
          };
        };
        iso = isoConfig.config.system.build.isoImage;
        default = self.packages.${system}.iso;
      };

      apps.${system}.web-installer = {
        type = "app";
        program = "${self.packages.${system}.web-installer}/bin/noos-web-installer";
      };
    };
}
