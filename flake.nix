{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.fenix = {
    url = "github:nix-community/fenix";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { fenix, nixpkgs, ... }: {
    devShells = nixpkgs.lib.genAttrs [
      "aarch64-linux"
      "x86_64-linux"
      "aarch64-darwin"
    ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in {
        default = pkgs.mkShell ({
          packages = [
            (fenix.packages.${system}.complete.withComponents [
              "cargo"
              "clippy"
              "rust-src"
              "rustc"
              "rustfmt"
            ])
            pkgs.git
            pkgs.nushell
          ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.wl-clipboard ];

          RUSTFLAGS = "-C target-cpu=native";
        } // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
            pkgs.libxkbcommon
            pkgs.wayland
          ];
        });
      });
  };
}
