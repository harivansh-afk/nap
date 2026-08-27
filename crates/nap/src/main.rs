//! nap: Not AirPlay(tm). Cast this mac onto a linux box's monitor.
//!
//! `on` flips a state file; a launchd agent (KeepAlive PathState on that
//! file) runs `supervise`, the convergence ladder in mac/supervise.rs.
//! `off` removes it; the agent tears down and exits. `status` reads local
//! state and asks the receiver for its side.

// Off-mac only `status` exists, so most of the client and config surface
// is intentionally unused there.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

mod client;
mod config;
#[cfg(target_os = "macos")]
mod mac;

use config::Config;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut cmd: Option<String> = None;
    let mut config_path: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => config_path = args.next(),
            "-h" | "--help" => usage(0),
            other if cmd.is_none() => cmd = Some(other.to_string()),
            _ => usage(64),
        }
    }
    let Some(cmd) = cmd else { usage(64) };

    let cfg = Config::load(config_path.as_deref()).unwrap_or_else(|e| {
        eprintln!("nap: {e}");
        std::process::exit(2);
    });

    match cmd.as_str() {
        "status" => status(&cfg),
        "on" | "off" | "supervise" => sender(&cfg, &cmd),
        _ => usage(64),
    }
}

fn usage(code: i32) -> ! {
    eprintln!("usage: nap [--config PATH] on|off|status|supervise");
    std::process::exit(code);
}

fn status(cfg: &Config) {
    #[cfg(target_os = "macos")]
    mac::local_status(cfg);

    let receiver = client::Receiver::new(&cfg.receiver, cfg.token());
    match receiver.status() {
        Ok(s) => {
            println!(
                "session: {}",
                if s.session {
                    format!(
                        "live (sender {}, lease {}s)",
                        s.sender_addr.as_deref().unwrap_or("?"),
                        s.lease_remaining_secs.unwrap_or(0)
                    )
                } else {
                    "none".into()
                }
            );
            println!("kiosk: {}", s.kiosk);
            println!("monitor: {}", s.monitor);
        }
        Err(e) => println!("receiver: {e}"),
    }
}

#[cfg(target_os = "macos")]
fn sender(cfg: &Config, cmd: &str) {
    mac::run(cfg, cmd);
}

#[cfg(not(target_os = "macos"))]
fn sender(_cfg: &Config, cmd: &str) {
    eprintln!("nap: `{cmd}` only makes sense on the mac (sender) side");
    std::process::exit(2);
}
