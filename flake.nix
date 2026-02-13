{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    deploy-rs = {
      url = "github:serokell/deploy-rs";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    any-sync-bundle-src = {
      url = "github:grishy/any-sync-bundle/v1.3.0-2026-01-31";
      flake = false;
    };

    valkey-bloom-src = {
      url = "github:valkey-io/valkey-bloom/1.0.0";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, deploy-rs, any-sync-bundle-src, valkey-bloom-src, ... }:
    let
      forAllSystems = f: nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-darwin" ] (system: f {
        inherit system;
        pkgs = import nixpkgs { inherit system; };
      });

      linuxPkgs = import nixpkgs { system = "x86_64-linux"; };
    in
    {
      packages = forAllSystems ({ system, pkgs }: {
        gluebox = pkgs.rustPlatform.buildRustPackage {
          pname = "gluebox";
          version = "0.1.0";
          src = nixpkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config cmake ];
          buildInputs = with pkgs; [ openssl sqlite ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.apple-sdk_15 ];
          meta = {
            description = "Glue layer syncing Linear, Anytype, Matrix, and Documenso";
            mainProgram = "gluebox";
          };
        };

        any-sync-bundle = (pkgs.buildGoModule.override { go = pkgs.go_1_25; }) {
          pname = "any-sync-bundle";
          version = "1.3.0-2026-01-31";
          src = any-sync-bundle-src;
          vendorHash = "sha256-IUAticFP900vVNRlnU/fzScg1/AIXjux0XuXMYFq0gQ=";
          env.CGO_ENABLED = 0;
          ldflags = [
            "-w" "-s"
            "-X github.com/grishy/any-sync-bundle/cmd.version=v1.3.0-2026-01-31"
            "-X github.com/grishy/any-sync-bundle/cmd.commit=${any-sync-bundle-src.rev or "unknown"}"
          ];
          doCheck = false;
          meta = {
            description = "All-in-one self-hosted Anytype server";
            mainProgram = "any-sync-bundle";
          };
        };

        default = self.packages.${system}.gluebox;
      } // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
        valkey-bloom = pkgs.rustPlatform.buildRustPackage {
          pname = "valkey-bloom";
          version = "1.0.0";
          src = valkey-bloom-src;
          cargoLock.lockFile = ./nix/valkey-bloom-Cargo.lock;
          nativeBuildInputs = with pkgs; [ clang ];
          LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          postPatch = ''
            ln -sf ${./nix/valkey-bloom-Cargo.lock} Cargo.lock
          '';
          postInstall = ''
            mkdir -p $out/lib
            find target/release -name "libvalkey_bloom.so" -exec cp {} $out/lib/ \;
          '';
          doCheck = false;
          meta.description = "Bloom filter module for Valkey";
        };
      });

      nixosConfigurations.gluebox-prod = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        specialArgs = { inherit self; };
        modules = [ ./hosts/gluebox-prod ];
      };

      deploy.nodes.gluebox-prod = {
        hostname = "gluebox-prod";
        profiles.system = {
          user = "root";
          sshUser = "root";
          path = deploy-rs.lib.x86_64-linux.activate.nixos self.nixosConfigurations.gluebox-prod;
        };
      };

      checks = builtins.mapAttrs (system: deployLib: deployLib.deployChecks self.deploy) deploy-rs.lib;

      devShells = forAllSystems ({ system, pkgs }: {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc cargo clippy rustfmt rust-analyzer pkg-config openssl sqlite
          ];
        };
      });
    };
}
