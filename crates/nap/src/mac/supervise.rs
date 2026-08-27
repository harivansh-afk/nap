//! The convergence ladder, ported rung for rung from the shell version.
//!
//! One fix per pass; supervise re-runs almost immediately after an action
//! and sleeps 5s only when steady, so a cold start walks the whole ladder
//! in a few seconds. The receiver-side rungs (kiosk, published address)
//! collapsed into one idempotent session assert whose reply says whether
//! the receiver changed anything; the assert also runs on every steady
//! tick, doubling as the lease heartbeat. It still sits before the
//! sunshine rungs on purpose: the panel's HDMI wake and moonlight's retry
//! loop overlap sunshine bring-up (safe: the gate keeps sunshine down
//! until its conf holds the fresh display id, so an early kiosk sees
//! connection-refused, never a wrong display).

use super::{display, procs};
use crate::client::{ClientError, Receiver};
use crate::config::Config;

pub struct Cast {
    pub cfg: Config,
    pub receiver: Receiver,
}

#[derive(PartialEq)]
pub enum Pass {
    Acted,
    Steady,
}

impl Cast {
    pub fn new(cfg: Config) -> Cast {
        let receiver = Receiver::new(&cfg.receiver, cfg.token());
        Cast { cfg, receiver }
    }

    pub fn state_path(&self, name: &str) -> String {
        format!("{}/{name}", self.cfg.state_dir)
    }

    pub fn note(&self, msg: &str) {
        let _ = std::fs::create_dir_all(&self.cfg.state_dir);
        let _ = std::fs::write(self.state_path("status"), format!("{msg}\n"));
    }

    /// Sunshine on a stale/invalid output_name silently falls back to
    /// capturing the real desktop (CGMainDisplayID), so it only runs while
    /// the display id in its conf is trustworthy. The gate file backs the
    /// launchd KeepAlive PathState: down = kill and stay dead, up = start
    /// and restart on crash.
    pub fn sunshine_down(&self) {
        let _ = std::fs::remove_file(self.state_path("sunshine-on"));
        procs::launchctl(&[
            "kill",
            "SIGTERM",
            &format!("gui/{}/{}", procs::uid(), self.cfg.sunshine_label),
        ]);
    }

    pub fn sunshine_up(&self) {
        let _ = std::fs::create_dir_all(&self.cfg.state_dir);
        let _ = std::fs::write(self.state_path("sunshine-on"), "");
        procs::launchctl(&[
            "kickstart",
            "-k",
            &format!("gui/{}/{}", procs::uid(), self.cfg.sunshine_label),
        ]);
    }

    /// DeskPad only, never a real monitor: pin its persistent UUID on
    /// first sight (DeskPad reports display serial 1) and match on the pin
    /// from then on.
    fn deskpad_display(&self) -> Option<display::DisplayInfo> {
        let all = display::displays();
        if all.is_empty() {
            return None;
        }
        let pin_path = self.state_path("display-uuid");
        if let Ok(pin) = std::fs::read_to_string(&pin_path) {
            let pin = pin.trim();
            if let Some(i) = all.iter().position(|d| d.uuid == pin) {
                return all.into_iter().nth(i);
            }
        }
        let i = all.iter().position(|d| d.serial == 1)?;
        let d = all.into_iter().nth(i)?;
        let _ = std::fs::create_dir_all(&self.cfg.state_dir);
        let _ = std::fs::write(&pin_path, &d.uuid);
        Some(d)
    }

    fn deskpad_present(&self) -> bool {
        self.deskpad_display().is_some()
    }

    pub fn converge(&self) -> Pass {
        let (want_w, want_h) = self.cfg.parse_resolution().expect("validated at load");

        if !procs::deskpad_running(&self.cfg.deskpad_app) {
            self.sunshine_down();
            procs::launch_deskpad(&self.cfg.deskpad_app);
            self.note("launching deskpad");
            procs::await_cond(12, || self.deskpad_present());
            return Pass::Acted;
        }

        let Some(dp) = self.deskpad_display() else {
            self.sunshine_down();
            self.note("waiting for virtual display");
            procs::await_cond(8, || self.deskpad_present());
            return Pass::Acted;
        };

        // Setting the resolution re-rolls the display id, so only when
        // wrong, and never with sunshine still serving the id about to die.
        if (dp.width, dp.height) != (want_w, want_h) {
            self.sunshine_down();
            if let Err(e) = display::set_mode(dp.id, want_w, want_h, self.cfg.hz, self.cfg.origin) {
                eprintln!("nap: set mode: {e}");
            }
            self.note(&format!("setting resolution to {}", self.cfg.resolution));
            procs::await_cond(6, || {
                self.deskpad_display()
                    .is_some_and(|d| (d.width, d.height) == (want_w, want_h))
            });
            return Pass::Acted;
        }

        let Some(lan) = procs::lan_ip(&self.cfg.interface) else {
            self.note(&format!("no lan address on {}", self.cfg.interface));
            return Pass::Steady;
        };

        let reply = match self.receiver.assert_session(&lan) {
            Ok(r) => r,
            Err(e) => {
                self.note(&e.to_string());
                // The client's connect timeout already paced this pass;
                // Acted keeps the retry loop tight once the receiver is
                // back, matching the old ladder getting stuck on its
                // publish rung.
                return match e {
                    ClientError::Unreachable(_) => Pass::Acted,
                    ClientError::Rejected(_) => Pass::Steady,
                };
            }
        };
        if reply.changed {
            self.note("receiver converging (kiosk)");
            return Pass::Acted;
        }

        // Input stays on the mac: disabling all client input keeps
        // sunshine's virtual HID machinery out of the event path (HID
        // stalls froze the mac). lan_encryption_mode 2: video RTP is
        // cleartext by default (sunshine's lan default never even
        // advertises video encryption support); mandatory is safe because
        // the only client is our kiosk, which always opts in to ENCFLG_ALL
        // on AES-capable hardware. origin_web_ui_allowed pc: the web UI
        // answers localhost only (pairing approval runs on the mac anyway).
        let want = format!(
            "min_log_level = info\nkeyboard = disabled\nmouse = disabled\ncontroller = disabled\noutput_name = {}\nlan_encryption_mode = 2\norigin_web_ui_allowed = pc",
            dp.id
        );
        let current = std::fs::read_to_string(&self.cfg.sunshine_conf)
            .map(|s| s.trim_end_matches('\n').to_string())
            .unwrap_or_default();
        if current != want {
            if let Some(dir) = std::path::Path::new(&self.cfg.sunshine_conf).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&self.cfg.sunshine_conf, format!("{want}\n")) {
                eprintln!("nap: write {}: {e}", self.cfg.sunshine_conf);
            }
            self.sunshine_up();
            self.note(&format!("pointing sunshine at display {}", dp.id));
            procs::await_cond(8, || procs::sunshine_listening(self.cfg.sunshine_port));
            return Pass::Acted;
        }

        if !procs::sunshine_listening(self.cfg.sunshine_port) {
            self.sunshine_up();
            self.note("restarting sunshine");
            procs::await_cond(8, || procs::sunshine_listening(self.cfg.sunshine_port));
            return Pass::Acted;
        }

        self.note(&format!("streaming (display {})", dp.id));
        Pass::Steady
    }

    pub fn teardown(&self) {
        if let Err(e) = self.receiver.delete_session() {
            eprintln!("nap: teardown: {e}");
        }
        self.sunshine_down();
        procs::quit_deskpad(&self.cfg.deskpad_app);
        self.note("off");
    }

    pub fn supervise(&self) {
        let on = self.state_path("on");
        let _ = std::fs::create_dir_all(&self.cfg.state_dir);
        while std::path::Path::new(&on).exists() {
            let pass = self.converge();
            std::thread::sleep(std::time::Duration::from_millis(if pass == Pass::Acted {
                300
            } else {
                5000
            }));
        }
        self.teardown();
    }
}
