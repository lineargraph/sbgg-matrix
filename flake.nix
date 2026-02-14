{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    haskell-json-fmt.url = "github:lineargraph/haskell-json-fmt";
    haskell-json-fmt.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    haskell-json-fmt,
    flake-utils,
    nixpkgs,
    treefmt-nix,
    self,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      treefmtEval = treefmt-nix.lib.evalModule pkgs {
        projectRootFile = "flake.nix";
        imports = [
          (haskell-json-fmt.lib.mkTreefmtModule {
            inherit pkgs;
            includes = ["*.json"];
          })
        ];
        programs.alejandra.enable = true;

        programs.rustfmt.enable = true;
        settings.formatter.rustfmt.options = ["--config-path" "${./rustfmt.toml}"];
      };
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
    in {
      formatter = treefmtEval.config.build.wrapper;
      checks.formatting = treefmtEval.config.build.check self;
      packages = rec {
        default = sbgg-matrix;
        sbgg-matrix = pkgs.rustPlatform.buildRustPackage (finalAttrs: {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          meta = {
            description = cargoToml.package.description;
          };
        });
        sbgg-matrix-docker = pkgs.dockerTools.buildImage {
          name = "sbgg-matrix";
          tag = "latest";
          runAsRoot = ''
            #!${pkgs.runtimeShell}
            mkdir -p /data
          '';
          config = {
            Cmd = ["${sbgg-matrix}/bin/sbgg-matrix"];
            WorkingDir = "/data";
          };
        };
      };
      devShells.default = pkgs.mkShell {
        buildInputs = [
          treefmtEval.config.build.wrapper
          pkgs.cargo
          pkgs.rustc
          pkgs.rust-analyzer
          pkgs.clippy
        ];
      };
    });
}
