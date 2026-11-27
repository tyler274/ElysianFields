{
  description = "Elysian Realm — reproducible devShells and separated dependency build roots";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, crane }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      forAllSystems = f:
        nixpkgs.lib.genAttrs supportedSystems (system: f system);

      mkPkgs = system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

      mkToolchain = pkgs:
        pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      # Separated dependency build roots:
      # Third-party crates are compiled once per target (`deps-native`, `deps-wasm`)
      # via `crane.buildDepsOnly`. Workspace source edits invalidate only the
      # leaf workspace derivation instead of recompiling Bevy or other dependencies.
      mkPackages = system:
        let
          pkgs = mkPkgs system;
          rustToolchain = mkToolchain pkgs;
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
          src = craneLib.cleanCargoSource ./.;

          nativeCommon = {
            inherit src;
            strictDeps = true;
            pname = "elysian-realm";
            version = "0.1.0";
          };

          wasmCommon = nativeCommon // {
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            doCheck = false;
          };

          deps-native = craneLib.buildDepsOnly nativeCommon;
          deps-wasm = craneLib.buildDepsOnly wasmCommon;

          elysian-smoke = craneLib.buildPackage (nativeCommon // {
            cargoArtifacts = deps-native;
            pname = "elysian-smoke";
            cargoExtraArgs = "-p elysian-smoke";
          });
        in
        {
          inherit deps-native deps-wasm elysian-smoke;
          default = elysian-smoke;
        };

      mkDevShells = system:
        let
          pkgs = mkPkgs system;
          rustToolchain = mkToolchain pkgs;

          basePackages = with pkgs; [
            rustToolchain
            clang
            lld
            python3
            mdbook
            lychee
            cargo-deny
            wasm-bindgen-cli
            binaryen
            pkg-config
            git
          ];

          mkShellWithProfile = profile:
            pkgs.mkShell {
              name = "elysian-realm-${profile}";
              packages = basePackages;
              ELYSIAN_DEV_SHELL = "1";
              ELYSIAN_SHELL_PROFILE = profile;
            };
        in
        {
          default = mkShellWithProfile "default";
          agent = mkShellWithProfile "agent";
        };
    in
    {
      packages = forAllSystems mkPackages;
      devShells = forAllSystems mkDevShells;
    };
}
