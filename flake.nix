{
  description = "Elysian Realm — reproducible devShells and separated dependency build roots";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    crane.url = "github:ipetkov/crane";

    # Optional M6 Stars upstream toolchain pins (never forked in-repo;
    # default devShells, packages, and CI remain on platform defaults).
    elymalloc = {
      url = "github:tyler274/ElyMalloc/cce11466569646e4e1f7afe9b44c6c80dd613b9f";
      flake = false;
    };
    elyld = {
      url = "github:tyler274/ElyLD/1b1e263ff40d5eea10966f6d2651f50ce43831ab";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, rust-overlay, crane, elymalloc, elyld }:
    let
      m6Pins = {
        elymalloc = {
          repo = "tyler274/ElyMalloc";
          branch = "main3";
          rev = "cce11466569646e4e1f7afe9b44c6c80dd613b9f";
          src = elymalloc;
        };
        elyld = {
          repo = "tyler274/ElyLD";
          branch = "main";
          rev = "1b1e263ff40d5eea10966f6d2651f50ce43831ab";
          src = elyld;
        };
      };

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

          linuxGuiLibs = pkgs.lib.optionals pkgs.stdenv.isLinux (with pkgs; [
            libxkbcommon
            wayland
            xorg.libX11
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
            vulkan-loader
          ]);

          linuxGuiEnv = pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath linuxGuiLibs;
          };

          mkShellWithProfile = profile: extraPackages: extraEnv:
            pkgs.mkShell ({
              name = "elysian-realm-${profile}";
              packages = basePackages ++ extraPackages;
              ELYSIAN_DEV_SHELL = "1";
              ELYSIAN_SHELL_PROFILE = profile;
            } // extraEnv);
        in
        {
          default = mkShellWithProfile "default" linuxGuiLibs linuxGuiEnv;
          agent = mkShellWithProfile "agent" [ ] { };
          # Optional M6 Stars evaluation shell exposing the pinned upstream
          # ElyMalloc and ElyLD source trees without altering default builds.
          m6 = mkShellWithProfile "m6"
            (pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.bubblewrap ])
            {
              ELYSIAN_ELYMALLOC_REV = m6Pins.elymalloc.rev;
              ELYSIAN_ELYMALLOC_SRC = "${elymalloc}";
              ELYSIAN_ELYLD_REV = m6Pins.elyld.rev;
              ELYSIAN_ELYLD_SRC = "${elyld}";
            };
        };
    in
    {
      lib = {
        inherit m6Pins;
      };
      packages = forAllSystems mkPackages;
      devShells = forAllSystems mkDevShells;
    };
}
