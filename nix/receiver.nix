# The nap receiver: napd (session lease, kiosk control, published sender
# address) plus the moonlight kiosk itself, cage on tty1.
#
# One-time pairing while a cast is up (the sender only runs sunshine
# mid-cast):
#   sudo -u <kioskUser> env QT_QPA_PLATFORM=offscreen moonlight pair <mac-ip> --pin NNNN
{
  config,
  lib,
  pkgs,
  napPackage,
  ...
}:
let
  cfg = config.services.nap.receiver;

  # A VT blank does not reach DPMS on NVIDIA fbcon (tested live: setterm
  # --blank force as root, dpms stayed On), so the monitor is put to sleep
  # by forcing the DRM connector off, which drops the HDMI link
  # unconditionally. The counterpart is force-connected, never `detect`:
  # after a force-off, nvidia-drm's reprobe reads disconnected with the
  # monitor attached (tested live: cage came up with zero outputs and
  # moonlight streamed into a placeholder screen), so probing can never be
  # trusted to relight. Forcing is sticky in both directions and wlroots
  # cannot undo it; cage-tty1 forces on in ExecStartPre before every cast.
  # Manual recovery if the box is ever dark and unmanaged:
  #   echo on > /sys/class/drm/<connector>/status
  # The write triggers a reprobe and can block indefinitely while DRM state
  # is in flux (observed: ExecStopPost hung for the full 90s stop timeout
  # during a switch-aborted start, SIGKILL, unit failed, and the failed
  # unit failed the whole activation). Both hooks are best-effort with a
  # hard timeout: a missed force costs one lit console, never a wedged stop
  # or a failed switch.
  # tee, not `sh -c 'echo > f'`: the unit PATH has no sh, and timeout needs
  # a child process to bound the potentially-blocking sysfs write.
  connectorStatus = "/sys/class/drm/${cfg.connector}/status";
  monitorOff = pkgs.writeShellApplication {
    name = "nap-monitor-off";
    runtimeInputs = [ pkgs.coreutils ];
    text = ''
      echo off | timeout 10 tee ${connectorStatus} >/dev/null || true
    '';
  };
  monitorOn = pkgs.writeShellApplication {
    name = "nap-monitor-on";
    runtimeInputs = [ pkgs.coreutils ];
    text = ''
      echo on | timeout 10 tee ${connectorStatus} >/dev/null || true
    '';
  };

  kiosk = pkgs.writeShellApplication {
    name = "nap-kiosk";
    runtimeInputs = [
      pkgs.moonlight-qt
      pkgs.coreutils
    ];
    text = ''
      export QT_QPA_PLATFORM=wayland
      export SDL_AUDIODRIVER=dummy
      while true; do
        host=$(cat ${cfg.hostFile} 2>/dev/null || true)
        if [ -n "$host" ]; then
          moonlight stream "$host" ${cfg.stream.app} \
            --resolution ${cfg.stream.resolution} \
            --fps ${toString cfg.stream.fps} \
            --bitrate ${toString cfg.stream.bitrateKbps} \
            --display-mode fullscreen \
            --video-codec ${cfg.stream.codec} \
            --no-vsync \
            --frame-pacing \
            --audio-on-host \
            --quit-after || true
        fi
        sleep 1
      done
    '';
  };
in
{
  options.services.nap.receiver = {
    enable = lib.mkEnableOption "the nap cast receiver (napd + moonlight kiosk)";

    package = lib.mkOption {
      type = lib.types.package;
      default = napPackage;
      description = "The nap package providing napd.";
    };

    bind = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:46270";
      description = ''
        ADDR:PORT napd listens on. Control traffic is tiny; binding a
        tailnet address gives authenticated reachability without opening
        the LAN, while the video stream stays on the LAN regardless.
      '';
    };

    tokenFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "File holding the bearer token senders must present. Null disables auth.";
    };

    leaseSecs = lib.mkOption {
      type = lib.types.ints.positive;
      default = 20;
      description = ''
        Session lease. The sender renews on its steady 5s tick; expiry
        tears the kiosk down and darkens the panel, so a sleeping or
        vanished sender never leaves the kiosk retrying at a lit monitor.
      '';
    };

    connector = lib.mkOption {
      type = lib.types.str;
      default = "card1-HDMI-A-1";
      example = "card0-DP-1";
      description = "DRM connector of the target monitor, as named under /sys/class/drm.";
    };

    hostFile = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/nap/host";
      description = "Where napd publishes the sender's video address for the kiosk.";
    };

    kioskUser = lib.mkOption {
      type = lib.types.str;
      default = "moonlight";
      description = "Dedicated user the kiosk runs as; its home holds the moonlight pairing state.";
    };

    stream = {
      resolution = lib.mkOption {
        type = lib.types.str;
        default = "3440x1440";
      };
      fps = lib.mkOption {
        type = lib.types.ints.positive;
        default = 60;
      };
      bitrateKbps = lib.mkOption {
        type = lib.types.ints.positive;
        default = 30000;
      };
      codec = lib.mkOption {
        type = lib.types.str;
        default = "HEVC";
      };
      app = lib.mkOption {
        type = lib.types.str;
        default = "Desktop";
        description = "Sunshine app name to stream.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.napd = {
      description = "nap cast receiver control plane";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];
      # napd adopts a running kiosk at startup (one lease to be renewed),
      # so restarting it mid-cast is safe.
      serviceConfig = {
        ExecStart = lib.concatStringsSep " " (
          [
            "${cfg.package}/bin/napd"
            "--bind ${cfg.bind}"
            "--kiosk-unit cage-tty1.service"
            "--connector ${cfg.connector}"
            "--host-file ${cfg.hostFile}"
            "--lease-secs ${toString cfg.leaseSecs}"
          ]
          ++ lib.optional (cfg.tokenFile != null) "--token-file ${cfg.tokenFile}"
        );
        Restart = "always";
        RestartSec = 2;
      };
    };

    systemd.tmpfiles.rules = [
      "d ${builtins.dirOf cfg.hostFile} 0755 root root -"
    ];

    users.groups.${cfg.kioskUser} = { };
    users.users.${cfg.kioskUser} = {
      isNormalUser = true;
      group = cfg.kioskUser;
      home = "/var/lib/${cfg.kioskUser}";
      createHome = true;
      homeMode = "0700";
      extraGroups = [ "video" ];
    };

    services.cage = {
      enable = true;
      user = cfg.kioskUser;
      program = lib.getExe kiosk;
      environment.WLR_LIBINPUT_NO_DEVICES = "1";
    };

    systemd.defaultUnit = lib.mkForce "multi-user.target";

    # Only napd ever starts the kiosk. The cage module wants it from
    # graphical.target, and graphical.target is active on a headless box,
    # so every nixos-rebuild switch would start the kiosk unasked (cast
    # off, monitor lit) and race its own activation. Empty wants makes the
    # unit strictly on-demand.
    # The hooks carry systemd's "+" prefix: cage-tty1 runs as the kiosk
    # user and Exec hooks inherit that, so without it every connector write
    # dies on EPERM behind the || true.
    # SuccessExitStatus=SIGKILL: the kiosk is stopped by SIGKILL on purpose
    # (cage+moonlight never exit on SIGTERM here; every observed stop rode
    # the timeout into SIGKILL anyway, and the kiosk is stateless), and
    # without it every stop leaves the unit "failed", which also fails any
    # activation that stops it. stopIfChanged=false keeps switches from
    # interrupting a live cast at all - unit changes apply on the next
    # natural kiosk restart.
    systemd.services."cage-tty1" = {
      wantedBy = lib.mkForce [ ];
      stopIfChanged = false;
      serviceConfig = {
        KillSignal = "SIGKILL";
        SuccessExitStatus = "SIGKILL";
        ExecStartPre = "+${lib.getExe monitorOn}";
        ExecStopPost = "+${lib.getExe monitorOff}";
      };
    };

    systemd.targets.graphical.wants = lib.mkForce [ ];

    # The monitor is dark whenever the kiosk is not running - the console
    # is never shown.
    systemd.services.nap-monitor-off = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = lib.getExe monitorOff;
      };
    };

    # `nap status` on the receiver box itself.
    environment.etc."nap/config.toml".text = ''
      receiver = "http://${cfg.bind}"
      ${lib.optionalString (cfg.tokenFile != null) "token_file = \"${cfg.tokenFile}\""}
    '';

    environment.systemPackages = [
      cfg.package
      pkgs.moonlight-qt
    ];
  };
}
