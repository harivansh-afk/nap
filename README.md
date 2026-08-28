# nap

Nap is a simple rust daemon and CLI that lets you use a linux machine as an airplay reciever

A mac creates a virtual display ([DeskPad](https://github.com/Stengo/DeskPad)), [Sunshine](https://github.com/LizardByte/Sunshine) captures it and encodes VideoToolbox HEVC, and the linux box decodes with moonlight-qt fullscreen in a cage kiosk on tty1. nap is the part that makes this an appliance instead of a pile of scripts: a convergence supervisor on the mac, a receiver daemon on the linux box, and a small HTTP control plane between them. Video rides the LAN; control can ride anything, including a tailnet.

```
mac (sender)                          linux box (receiver)
  nap supervise ── PUT /v1/session ──▶ napd
  DeskPad (virtual display)             ├─ starts/stops the moonlight kiosk
  Sunshine (capture + encode) ◀──────── └─ publishes the sender address
             video (LAN, AES-GCM)  moonlight-qt in cage on tty1
```

## Why not just scripts

This started as scripts. The extracted design keeps every behavior those scripts learned the hard way and adds one thing shell could not do cleanly: leases. The sender asserts its session on every 5 second tick; if the mac sleeps or leaves the network, the lease expires and the receiver tears the kiosk down and darkens the panel on its own. If the sender's LAN address changes mid-cast, the receiver restarts the kiosk at the new address instead of letting moonlight dial a stale one forever.

The rest of the hard-won behavior is intact:

- One fix per supervisor pass, each fix waiting (bounded) for its own postcondition, re-run ~0.3s after acting. A cold start walks the whole ladder in a few seconds; steady state ticks every 5s.
- Sunshine only runs while its config holds a current display id. On a stale id, upstream Sunshine silently captures the real desktop, so the gate fails closed: the supervisor kills Sunshine before any action that invalidates the id and raises it only after rewriting the config.
- The kiosk starts before Sunshine on purpose. The panel's HDMI wake and moonlight's retry loop overlap Sunshine bring-up.
- Video encryption is mandatory (`lan_encryption_mode = 2`), client input is disabled (Sunshine's virtual HID machinery froze the mac), and the web UI answers localhost only.
- The kiosk stops by SIGKILL. cage+moonlight never exit on SIGTERM, the kiosk is stateless, and an honest instant kill beats a 5s corpse window.
- The monitor sleeps by forcing the DRM connector off and wakes by forcing it on, never `detect` (the reprobe after a force-off reads disconnected with the monitor attached).
- The virtual display is found by a pinned persistent UUID, never by guessing which display is which. Real monitors on the mac are ignored.

## Install

Flake input:

```nix
inputs.nap.url = "git+https://git.harivan.sh/harivansh-afk/nap";
```

Receiver (NixOS):

```nix
imports = [ nap.nixosModules.receiver ];
services.nap.receiver = {
  enable = true;
  bind = "100.x.y.z:46270";        # a tailnet address is a good choice
  tokenFile = "/run/secrets/nap-token";  # optional
  connector = "card1-HDMI-A-1";    # your monitor, as named under /sys/class/drm
};
```

Sender (nix-darwin):

```nix
imports = [ nap.darwinModules.sender ];
services.nap.sender = {
  enable = true;
  receiver = "http://100.x.y.z:46270";
  tokenFile = "/path/to/nap-token";      # optional, pair of the above
  resolution = "3440x1440";
  origin = [ (-573) (-1440) ];     # where the display sits in the arrangement
};
```

Then:

```
nap on       # flips a state file; the launchd agent converges
nap status   # local state + the receiver's view (kiosk, monitor, lease)
nap off      # tears down; the monitor goes dark
```

## One-time setup

Sunshine comes from homebrew on the mac (`brew install sunshine`); nixpkgs's Sunshine is linux-only. Three grants cannot be declared and are done once per machine:

1. Sunshine's Screen Recording permission (macOS TCC prompt on first capture).
2. Sunshine web UI credentials: `sunshine --creds`.
3. Moonlight pairing, run on the receiver while a cast is up (Sunshine only serves mid-cast):
   `sudo -u moonlight env QT_QPA_PLATFORM=offscreen moonlight pair <mac-ip> --pin NNNN`,
   then approve via the Sunshine web UI on the mac.

DeskPad builds from pinned source at activation (needs Xcode; skipped without it). The released binary lacks the ultrawide modes, and the build carries a patch that strips the preview window's capture call, so DeskPad itself needs no Screen Recording grant and macOS never shows the legacy-capture nag.

## Control plane

Three endpoints, JSON over HTTP, optional `Authorization: Bearer <token>`:

| | |
|---|---|
| `PUT /v1/session` | Idempotent assert: publish the sender address, start the kiosk, renew the lease. The reply says whether anything changed. |
| `DELETE /v1/session` | Teardown: kiosk stopped, panel dark. |
| `GET /v1/status` | Session, lease remaining, kiosk unit state, DRM connector state. |

The lease defaults to 20 seconds, three missed ticks. napd adopts a running kiosk after its own restart and gives it one lease to be renewed, so restarting napd mid-cast is safe.

## Layout

```
crates/nap-proto   wire types
crates/napd        receiver daemon (linux)
crates/nap         sender CLI + supervisor (macos; `status` works anywhere)
nix/receiver.nix   NixOS module: napd + cage/moonlight kiosk + DRM handling
nix/sender.nix     nix-darwin module: launchd agents + config + DeskPad build
```

The sender talks to CoreGraphics directly (display enumeration by persistent UUID, mode setting, arrangement origin), so there is no displayplacer dependency.
