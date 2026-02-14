{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    haskell-json-fmt.url = "github:lineargraph/haskell-json-fmt";
    haskell-json-fmt.inputs.nixpkgs.follows = "nixpkgs";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    haskell-json-fmt,
    flake-utils,
    nixpkgs,
    treefmt-nix,
    naersk,
    self,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      naersk' = pkgs.callPackage naersk {};
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
        sbgg-matrix = naersk'.buildPackage {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = ./.;
          meta = {
            description = cargoToml.package.description;
          };
        };
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
            Env = [
              "RUST_LOG=info"
              "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
            ];
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
