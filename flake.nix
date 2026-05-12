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
            at-spi2-core
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
            at-spi2-core
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
              for bin in normawm test_window; do
                if [ -x "$out/bin/$bin" ]; then
                  wrapProgram "$out/bin/$bin" \
                    --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibs}:/run/opengl-driver/lib:/run/opengl-driver-32/lib" \
                    --set LIBGL_DRIVERS_PATH "/run/opengl-driver/lib/dri"
                fi
              done
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
        let
          lib = nixpkgs.lib;
        in
        {
          default = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/normawm";
          };
        } // lib.optionalAttrs (system == "x86_64-linux") {
          vm = {
            type = "app";
            program = "${self.nixosConfigurations.normawm-vm.config.system.build.vm}/bin/run-normawm-vm-vm";
          };
        }
      );

      nixosConfigurations.normawm-vm = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          (
            { pkgs, ... }:
            {
              system.stateVersion = "25.05";

              networking.hostName = "normawm-vm";

              users.users.norma = {
                isNormalUser = true;
                password = "norma";
                extraGroups = [
                  "wheel"
                  "video"
                  "input"
                ];
              };

              services.qemuGuest.enable = true;
              services.xserver.enable = true;
              services.xserver.desktopManager.xfce.enable = true;
              services.xserver.displayManager.lightdm.enable = true;
              services.displayManager.autoLogin = {
                enable = true;
                user = "norma";
              };

              hardware.graphics.enable = true;
              programs.dconf.enable = true;

              environment.systemPackages = [
                self.packages.x86_64-linux.default
                pkgs.mesa-demos
                pkgs.wayland-utils
                pkgs.xfce4-terminal
                pkgs.xterm
              ];

              virtualisation.vmVariant = {
                virtualisation = {
                  cores = 2;
                  memorySize = 3072;
                  diskSize = 8192;
                  qemu.options = [
                    "-device"
                    "virtio-vga"
                    "-display"
                    "gtk,gl=on"
                  ];
                };
              };
            }
          )
        ];
      };

      formatter = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        pkgs.nixfmt-rfc-style
      );
    };
}
