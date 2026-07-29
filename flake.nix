{
  description = "Reproducible Gesenius OCR and Unicode corpus toolchain";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.11";
    flake-parts.url = "github:hercules-ci/flake-parts";
    pyproject-nix = {
      url = "github:pyproject-nix/pyproject.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    uv2nix = {
      url = "github:pyproject-nix/uv2nix";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    pyproject-build-systems = {
      url = "github:pyproject-nix/build-system-pkgs";
      inputs.pyproject-nix.follows = "pyproject-nix";
      inputs.uv2nix.follows = "uv2nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      perSystem =
        {
          pkgs,
          lib,
          system,
          ...
        }:
        let
          workspace = inputs.uv2nix.lib.workspace.loadWorkspace {
            workspaceRoot = ./ocr;
          };
          python = pkgs.python313;
          pythonSet =
            (pkgs.callPackage inputs.pyproject-nix.build.packages { inherit python; }).overrideScope
              (
                lib.composeManyExtensions [
                  inputs.pyproject-build-systems.overlays.default
                  (workspace.mkPyprojectOverlay {
                    sourcePreference = "wheel";
                  })
                  (final: prev: {
                    # The CPU torchvision wheel links libtorch from the sibling
                    # torch wheel. Declare that native relationship explicitly
                    # so autoPatchelf can resolve it in the Nix build.
                    torchvision = prev.torchvision.overrideAttrs (old: {
                      buildInputs = (old.buildInputs or [ ]) ++ [ final.torch ];
                      preFixup = (old.preFixup or "") + ''
                        addAutoPatchelfSearchPath ${final.torch}/${python.sitePackages}/torch/lib
                      '';
                    });
                  })
                ]
              );
          ocrEnv = pythonSet.mkVirtualEnv "gesenius-ocr-env" workspace.deps.default;
          rustSource = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
              ./fixtures
              ./pilot.toml
              ./pipeline.toml
              ./schema
              ./sources.toml
              ./rustfmt.toml
            ];
          };
          tesseractWithLanguages = pkgs.tesseract5.override {
            enableLanguages = [
              "eng"
              "heb"
              "ara"
              "syr"
              "grc"
              "lat"
            ];
          };
          nativeTools = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
            poppler-utils
            imagemagick
            sqlite
            libxml2
            jing
            curl
            uv
            tesseractWithLanguages
            noto-fonts
            noto-fonts-lgc-plus
          ];
        in
        {
          packages = {
            default = pkgs.rustPlatform.buildRustPackage {
              pname = "gesenius";
              version = "0.1.0";
              src = rustSource;
              cargoLock.lockFile = ./Cargo.lock;
              doCheck = true;
            };
            ocr-environment = ocrEnv;
          };

          devShells.default = pkgs.mkShell {
            packages = nativeTools ++ [ ocrEnv ];
            RUST_BACKTRACE = "1";
            SOURCE_DATE_EPOCH = "0";
            shellHook = ''
              echo "Gesenius OCR shell: Rust, Tesseract, Kraken, Poppler, ImageMagick, SQLite, and XML validators"
            '';
          };

          checks = {
            inherit (inputs.self.packages.${system}) default;
            formatting =
              pkgs.runCommand "gesenius-formatting"
                {
                  nativeBuildInputs = [
                    pkgs.rustfmt
                    pkgs.cargo
                  ];
                  src = rustSource;
                }
                ''
                  cp -r "$src" source
                  chmod -R u+w source
                  cd source
                  cargo fmt --all --check
                  touch "$out"
                '';
          };

          formatter = pkgs.nixfmt-rfc-style;
        };
    };
}
