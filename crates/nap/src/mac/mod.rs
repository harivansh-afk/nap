pub mod display;
pub mod procs;
pub mod supervise;

use crate::config::Config;
use supervise::Cast;

pub fn run(cfg: &Config, cmd: &str) {
    let cast = Cast::new(cfg.clone());
    match cmd {
        "on" => {
            let _ = std::fs::create_dir_all(&cfg.state_dir);
            let _ = std::fs::write(cast.state_path("on"), "");
            // Kickstart (without -k) nudges the agent awake immediately
            // instead of waiting for launchd to notice the PathState.
            procs::launchctl(&[
                "kickstart",
                &format!("gui/{}/{}", procs::uid(), cfg.cast_label),
            ]);
            println!("cast: on (converging; check nap status)");
        }
        "off" => {
            let _ = std::fs::remove_file(cast.state_path("on"));
            cast.teardown();
            println!("cast: off (monitor going dark)");
        }
        "supervise" => cast.supervise(),
        _ => unreachable!(),
    }
}

pub fn local_status(cfg: &Config) {
    let on = std::path::Path::new(&cfg.state_dir).join("on").exists();
    println!("cast: {}", if on { "on" } else { "off" });
    let state = std::fs::read_to_string(format!("{}/status", cfg.state_dir))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    println!("state: {state}");
    println!(
        "deskpad: {}",
        if procs::deskpad_running(&cfg.deskpad_app) {
            "running"
        } else {
            "stopped"
        }
    );
    println!(
        "sunshine: {}",
        if procs::sunshine_listening(cfg.sunshine_port) {
            "listening"
        } else {
            "down"
        }
    );
}
