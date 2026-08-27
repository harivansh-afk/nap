{
  description = "nap: Not AirPlay(tm). Cast a mac onto a linux box's monitor over the LAN.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    # DeskPad virtual display, built from source on the mac: the released
    # binary (and its cask) lacks the ultrawide modes; 3440x1440@60 exists
    # only on main. See nix/sender.nix.
    deskpad-src = {
      url = "github:Stengo/DeskPad";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      deskpad-src,
    }:
    let
      systems = [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        # One derivation, both binaries: `nap` (sender CLI + supervisor;
        # sender-only subcommands are macos-gated at runtime) and `napd`
        # (receiver daemon).
        nap = pkgs.rustPlatform.buildRustPackage {
          pname = "nap";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            description = "Not AirPlay(tm): cast a mac onto a linux box's monitor";
            homepage = "https://git.harivan.sh/harivansh-afk/nap";
            license = pkgs.lib.licenses.mit;
            mainProgram = "nap";
          };
        };
        default = nap;
      });

      nixosModules.receiver =
        { pkgs, ... }@args:
        import ./nix/receiver.nix (
          args
          // {
            napPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.nap;
          }
        );

      darwinModules.sender =
        { pkgs, ... }@args:
        import ./nix/sender.nix (
          args
          // {
            napPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.nap;
            deskpadSrc = deskpad-src;
          }
        );

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
          ];
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
