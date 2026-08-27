//! Local process plumbing: launchd, DeskPad lifecycle, LAN address,
//! sunshine's listening check. Absolute system-binary paths, same as the
//! shell version, so a stripped launchd PATH can never break a rung.

use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

pub fn deskpad_running(app: &str) -> bool {
    let name = std::path::Path::new(app)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("DeskPad");
    Command::new("/usr/bin/pgrep")
        .args(["-xq", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// -gj: hidden and in the background; the virtual display needs no
/// visible window and the preview is patched out anyway.
pub fn launch_deskpad(app: &str) {
    let _ = Command::new("/usr/bin/open").args(["-gj", app]).status();
}

pub fn quit_deskpad(app: &str) {
    let name = std::path::Path::new(app)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("DeskPad");
    let _ = Command::new("/usr/bin/osascript")
        .args(["-e", &format!("quit app \"{name}\"")])
        .status();
}

pub fn launchctl(args: &[&str]) {
    let _ = Command::new("/bin/launchctl").args(args).status();
}

pub fn uid() -> String {
    Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn lan_ip(interface: &str) -> Option<String> {
    let out = Command::new("/usr/sbin/ipconfig")
        .args(["getifaddr", interface])
        .output()
        .ok()?;
    let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!ip.is_empty()).then_some(ip)
}

pub fn sunshine_listening(port: u16) -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// Poll until `cond` holds or `secs` elapse. Fix branches wait for their
/// own postcondition, which is what makes fast re-runs safe: without it a
/// quick next pass would see e.g. sunshine's port not yet bound and
/// kickstart -k it mid-start, forever.
pub fn await_cond(secs: u64, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if cond() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}
