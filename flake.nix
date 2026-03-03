{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    crane.url = "github:ipetkov/crane";

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

  outputs = { self, nixpkgs, crane, deploy-rs, any-sync-bundle-src, valkey-bloom-src, ... }:
    let
      forAllSystems = f: nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-darwin" ] (system: f {
        inherit system;
        pkgs = import nixpkgs { inherit system; };
      });

      linuxPkgs = import nixpkgs { system = "x86_64-linux"; };

      glueboxFor = pkgs:
        let
          craneLib = crane.mkLib pkgs;

          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let baseName = baseNameOf path;
              in !(
                (builtins.match ".*\\.jj.*" path != null) ||
                (builtins.match ".*\\.github.*" path != null) ||
                (builtins.match ".*/hosts/.*" path != null) ||
                (builtins.match ".*/nix/.*" path != null) ||
                (builtins.match ".*/target/.*" path != null) ||
                (builtins.match ".*\\.md$" path != null) ||
                baseName == "flake.nix" ||
                baseName == "flake.lock" ||
                baseName == "cliff.toml" ||
                baseName == ".gitignore" ||
                baseName == "result"
              ) || craneLib.filterCargoSources path type;
          };

          commonArgs = {
            inherit src;
            strictDeps = true;
            doCheck = false;
            CARGO_INCREMENTAL = "0";
            nativeBuildInputs = with pkgs; [ pkg-config cmake ];
            buildInputs = with pkgs; [ openssl sqlite ]
              ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [ pkgs.apple-sdk_15 ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        {
          package = craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;
            meta = {
              description = "Glue layer syncing Linear, Anytype, Matrix, and Documenso";
              mainProgram = "gluebox";
            };
          });

          nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
            cargoNextestExtraArgs = "--profile ci";
          });
        };

    in
    {
      packages = forAllSystems ({ system, pkgs }: {
        gluebox = (glueboxFor pkgs).package;

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
          postBuild = ''
            mkdir -p $out/lib
            find . -name "libvalkey_bloom.so" -exec cp {} $out/lib/ \;
          '';
          installPhase = "true";
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

      checks = nixpkgs.lib.recursiveUpdate
        (builtins.mapAttrs (system: deployLib: deployLib.deployChecks self.deploy) deploy-rs.lib)
        (nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-darwin" ] (system:
          let pkgs = import nixpkgs { inherit system; };
          in { nextest = (glueboxFor pkgs).nextest; }
        ));

      devShells = forAllSystems ({ system, pkgs }: {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc cargo clippy rustfmt rust-analyzer pkg-config openssl sqlite
            cargo-nextest
            vultr-cli
            nushell
          ];
          shellHook = ''
            if [[ $- == *i* ]] && command -v nu &> /dev/null; then
              exec nu
            fi
          '';
        };

        ci = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc cargo cargo-nextest pkg-config cmake openssl sqlite
          ];
          env = {
            CARGO_INCREMENTAL = "0";
            CARGO_REGISTRIES_CRATES_IO_PROTOCOL = "sparse";
            CARGO_HTTP_MULTIPLEXING = "true";
            CARGO_NET_RETRY = "5";
          };
        };
      });
    };
}
