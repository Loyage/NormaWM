{
  description = "NormaWM development environment and package definition";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      fenix,
      ...
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ fenix.overlays.default ];
          };
          lib = pkgs.lib;
          rustToolchain = pkgs.fenix.stable.withComponents [
            "cargo"
            "clippy"
            "rust-analyzer"
            "rust-src"
            "rustc"
            "rustfmt"
          ];
          runtimeLibs = with pkgs; [
            stdenv.cc.cc.lib
            libglvnd
            mesa
            libdrm
            libinput
            seatd
            udev
            wayland
            wayland-protocols
            libxkbcommon
            libx11
            libxcursor
            libxi
            libxrandr
            libxcb
            libxext
          ];
        in
        {
          default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.cargo-watch
              pkgs.clang
              pkgs.just
              pkgs.lld
              pkgs.mdbook
              pkgs.mesa-demos
              pkgs.llvmPackages.libclang
              pkgs.pkg-config
              pkgs.rust-analyzer
              pkgs.wayland-utils
            ] ++ runtimeLibs;

            env.LD_LIBRARY_PATH =
              "${lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib:/run/opengl-driver-32/lib";
            env.LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            env.LIBGL_DRIVERS_PATH = "/run/opengl-driver/lib/dri";
            env.RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

            shellHook = ''
              echo "NormaWM dev shell ready."
              echo "Wayland runtime tools: wayland-info, eglinfo, glxinfo"
              echo "Build with: cargo check"
              echo "Build docs with: mdbook build"
              echo "Run nested compositor with: cargo run"
            '';
          };
        }
      );

      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          lib = pkgs.lib;
          runtimeLibs = with pkgs; [
            libglvnd
            mesa
            libdrm
            libinput
            udev
            seatd
            wayland
            wayland-protocols
            libxkbcommon
            libx11
            libxcursor
            libxi
            libxrandr
            libxcb
            libxext
          ];
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "normawm";
            version = "0.1.0";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [
              pkgs.makeWrapper
              pkgs.pkg-config
            ];

            buildInputs = runtimeLibs;

            postFixup = ''
              wrapProgram $out/bin/normawm \
                --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib:/run/opengl-driver-32/lib" \
                --set LIBGL_DRIVERS_PATH "/run/opengl-driver/lib/dri"
            '';

            meta = with pkgs.lib; {
              description = "NormaWM is a Smithay-based nested Wayland compositor";
              platforms = platforms.linux;
            };
          };
        }
      );

      apps = forAllSystems (
        system:
        {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/normawm";
          };
        }
      );

      formatter = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        pkgs.nixfmt-rfc-style
      );
    };
}
