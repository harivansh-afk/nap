# The nap sender for nix-darwin: the nap supervisor and sunshine as
# launchd agents, /etc/nap/config.toml, and DeskPad built from pinned
# source at activation.
#
# Sunshine itself comes from homebrew (nixpkgs's sunshine is linux-only):
#   brew install sunshine
# One-time, per machine, not declarable: sunshine's Screen Recording grant
# (TCC), sunshine web-UI creds (`sunshine --creds`), and moonlight pairing
# on the receiver (run it mid-cast; sunshine only serves while a cast is
# up).
{
  config,
  lib,
  pkgs,
  napPackage,
  deskpadSrc,
  ...
}:
let
  cfg = config.services.nap.sender;
  home = "/Users/${cfg.user}";
  stateDir = "${home}/.local/state/nap";

  originList = lib.concatStringsSep ", " (map toString cfg.origin);

  deskpad = import ./deskpad.nix {
    inherit lib pkgs;
    src = deskpadSrc;
    username = cfg.user;
  };
in
{
  options.services.nap.sender = {
    enable = lib.mkEnableOption "the nap cast sender (supervisor + sunshine agents)";

    package = lib.mkOption {
      type = lib.types.package;
      default = napPackage;
      description = "The nap package providing the nap CLI/supervisor.";
    };

    receiver = lib.mkOption {
      type = lib.types.str;
      example = "http://spark:46270";
      description = "Base URL of napd on the receiver.";
    };

    tokenFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "File holding the bearer token napd expects. Null sends no auth.";
    };

    resolution = lib.mkOption {
      type = lib.types.str;
      default = "3440x1440";
      description = "Virtual display resolution, applied with scaling off.";
    };

    hz = lib.mkOption {
      type = lib.types.ints.positive;
      default = 60;
    };

    origin = lib.mkOption {
      type = lib.types.listOf lib.types.int;
      default = [
        (-573)
        (-1440)
      ];
      description = "Global display-space origin of the virtual display in the arrangement.";
    };

    interface = lib.mkOption {
      type = lib.types.str;
      default = "en0";
      description = "Interface whose address becomes the video target (the video path stays on the LAN).";
    };

    sunshineBin = lib.mkOption {
      type = lib.types.str;
      default = "/opt/homebrew/opt/sunshine/bin/sunshine";
      description = "Path to the homebrew-installed sunshine binary.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = config.system.primaryUser;
      description = "User the cast agents run as.";
    };

    deskpad.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Build and install the patched DeskPad at activation (needs Xcode).";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    environment.etc."nap/config.toml".text = ''
      receiver = "${cfg.receiver}"
      ${lib.optionalString (cfg.tokenFile != null) ''token_file = "${cfg.tokenFile}"''}
      resolution = "${cfg.resolution}"
      hz = ${toString cfg.hz}
      origin = [${originList}]
      interface = "${cfg.interface}"
    '';

    launchd.user.agents = {
      # Never run sunshine any other way: a second instance steals the
      # ports and this one goes deaf. Gated on the supervisor's sunshine-on
      # file: sunshine serves (and its ports listen) only while the cast is
      # up and the display id in its conf is current; on a stale id
      # upstream silently falls back to capturing the real desktop
      # (CGMainDisplayID), which is what this gate fails closed against.
      # Crashes still restart while the gate is present.
      sunshine.serviceConfig = {
        ProgramArguments = [
          "/bin/sh"
          "-c"
          "/bin/wait4path ${cfg.sunshineBin} && exec ${cfg.sunshineBin} ${home}/.config/sunshine/sunshine.conf"
        ];
        RunAtLoad = false;
        KeepAlive = {
          PathState."${stateDir}/sunshine-on" = true;
        };
        StandardOutPath = "${home}/Library/Logs/sunshine.log";
        StandardErrorPath = "${home}/Library/Logs/sunshine.log";
      };

      # The convergence supervisor; PathState = alive exactly while the
      # cast is on, resuming across reboots.
      nap-cast.serviceConfig = {
        ProgramArguments = [
          "/bin/sh"
          "-c"
          "/bin/wait4path /nix/store && exec ${cfg.package}/bin/nap supervise"
        ];
        RunAtLoad = true;
        KeepAlive = {
          PathState."${stateDir}/on" = true;
        };
        ThrottleInterval = 5;
        StandardOutPath = "${home}/Library/Logs/nap.log";
        StandardErrorPath = "${home}/Library/Logs/nap.log";
      };
    };

    system.activationScripts.postActivation.text = lib.mkAfter (
      # One-time migration off `brew services start sunshine`.
      ''
        brewSunshinePlist="${home}/Library/LaunchAgents/homebrew.mxcl.sunshine.plist"
        if [ -f "$brewSunshinePlist" ]; then
          sudo -u ${cfg.user} /bin/launchctl bootout \
            "gui/$(id -u ${cfg.user})/homebrew.mxcl.sunshine" 2>/dev/null || true
          rm -f "$brewSunshinePlist"
          echo "removed brew-services sunshine agent (now nix-owned)"
        fi
      ''
      + lib.optionalString cfg.deskpad.enable ''
        sudo -u ${cfg.user} ${deskpad.buildScript} || echo "warning: DeskPad build failed" >&2
      ''
    );
  };
}
