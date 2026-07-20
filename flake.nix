{
  description = "schema-engine — the encoded-form Schema daemon and thin CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-build = {
      url = "github:LiGoldragon/rust-build";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-build }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        rust = rust-build.lib.${system}.fromPkgs pkgs;
        inherit (rust) craneLib toolchain;
        # The equivalence witness reads a .schema fixture; preserve non-Rust test data
        # (rust.cleanSource strips it), mirroring language-engine-witness.
        src = pkgs.lib.cleanSource ./.;
        commonArguments = { inherit src; strictDeps = true; };
        cargoArtifacts = craneLib.buildDepsOnly commonArguments;
      in
      {
        packages.default = craneLib.buildPackage (commonArguments // { inherit cargoArtifacts; });
        checks = {
          build = craneLib.cargoBuild (commonArguments // { inherit cargoArtifacts; });
          test = craneLib.cargoTest (commonArguments // { inherit cargoArtifacts; });
          doc = craneLib.cargoDoc (commonArguments // {
            inherit cargoArtifacts;
            RUSTDOCFLAGS = "-D warnings";
          });
          fmt = craneLib.cargoFmt { inherit src; };
          clippy = craneLib.cargoClippy (commonArguments // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- -D warnings";
          });
        };
        devShells.default = pkgs.mkShell {
          name = "schema-engine";
          packages = [ pkgs.jujutsu toolchain ];
        };
      });
}
