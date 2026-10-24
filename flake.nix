{
  description = "HireFerry server and hfy CLI";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system:
        f {
          pkgs = import nixpkgs { inherit system; };
          inherit system;
        });
    in {
      packages = forAllSystems ({ pkgs, ... }:
        let
          common = {
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.openssl ];
            doCheck = false;
          };
          server = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "hireferry-server";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            cargoBuildFlags = [ "--package" "hireferry" ];
            installPhase = ''
              install -Dm755 target/release/hireferry $out/bin/hireferry
            '';
            meta.mainProgram = "hireferry";
          });
          cli = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "hireferry-cli";
            version = (builtins.fromTOML (builtins.readFile ./crates/hireferry-cli/Cargo.toml)).package.version;
            cargoBuildFlags = [ "--package" "hireferry-cli" "--bin" "hfy" ];
            installPhase = ''
              install -Dm755 target/release/hfy $out/bin/hfy
            '';
            meta.mainProgram = "hfy";
          });
        in {
          inherit server cli;
          default = server;
        });

      apps = forAllSystems ({ system, ... }:
        let
          server = self.packages.${system}.server;
          cli = self.packages.${system}.cli;
        in {
          server = {
            type = "app";
            program = "${server}/bin/hireferry";
          };
          cli = {
            type = "app";
            program = "${cli}/bin/hfy";
          };
          default = self.apps.${system}.server;
        });

      devShells = forAllSystems ({ pkgs, ... }: {
        default = pkgs.mkShell {
          packages = [ pkgs.rustc pkgs.cargo pkgs.pkg-config pkgs.openssl ];
        };
      });
    };
}
