//! Sender configuration. Loaded from, in order: $NAP_CONFIG,
//! ~/.config/nap/config.toml, /etc/nap/config.toml. The nix module renders
//! /etc/nap/config.toml; the earlier paths exist for hand-run experiments.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Base URL of napd on the receiver, e.g. "http://spark:46270".
    /// Control traffic is tiny; an overlay network (tailscale) is fine
    /// here even where it is too slow for the video path.
    pub receiver: String,
    /// File holding the bearer token napd expects, if auth is on.
    #[serde(default)]
    pub token_file: Option<String>,

    /// Virtual display geometry, applied with scaling off.
    #[serde(default = "d_resolution")]
    pub resolution: String,
    #[serde(default = "d_hz")]
    pub hz: u32,
    /// Global display-space origin of the virtual display relative to the
    /// arrangement (where the extended display sits next to the builtin).
    #[serde(default = "d_origin")]
    pub origin: [i32; 2],

    /// Interface whose address is handed to the receiver as the video
    /// target. The video path deliberately stays on the LAN.
    #[serde(default = "d_interface")]
    pub interface: String,

    #[serde(default = "d_deskpad_app")]
    pub deskpad_app: String,
    #[serde(default = "d_sunshine_label")]
    pub sunshine_label: String,
    #[serde(default = "d_cast_label")]
    pub cast_label: String,
    #[serde(default = "d_sunshine_conf")]
    pub sunshine_conf: String,
    #[serde(default = "d_sunshine_port")]
    pub sunshine_port: u16,
    #[serde(default = "d_state_dir")]
    pub state_dir: String,
}

fn d_resolution() -> String {
    "3440x1440".into()
}
fn d_hz() -> u32 {
    60
}
fn d_origin() -> [i32; 2] {
    [-573, -1440]
}
fn d_interface() -> String {
    "en0".into()
}
fn d_deskpad_app() -> String {
    "/Applications/DeskPad.app".into()
}
fn d_sunshine_label() -> String {
    "org.nixos.sunshine".into()
}
fn d_cast_label() -> String {
    "org.nixos.nap-cast".into()
}
fn d_sunshine_conf() -> String {
    "~/.config/sunshine/sunshine.conf".into()
}
fn d_sunshine_port() -> u16 {
    47989
}
fn d_state_dir() -> String {
    "~/.local/state/nap".into()
}

impl Config {
    pub fn load(explicit: Option<&str>) -> Result<Config, String> {
        let path = match explicit {
            Some(p) => p.to_string(),
            None => candidate_paths()
                .into_iter()
                .find(|p| std::path::Path::new(p).exists())
                .ok_or_else(|| {
                    format!(
                        "no config found (searched: {})",
                        candidate_paths().join(", ")
                    )
                })?,
        };
        let raw = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
        let mut cfg: Config = toml::from_str(&raw).map_err(|e| format!("parse {path}: {e}"))?;
        cfg.sunshine_conf = expand_tilde(&cfg.sunshine_conf);
        cfg.state_dir = expand_tilde(&cfg.state_dir);
        if let Some(t) = &cfg.token_file {
            cfg.token_file = Some(expand_tilde(t));
        }
        cfg.parse_resolution()?;
        Ok(cfg)
    }

    pub fn parse_resolution(&self) -> Result<(u64, u64), String> {
        let (w, h) = self
            .resolution
            .split_once('x')
            .ok_or_else(|| format!("bad resolution {:?}", self.resolution))?;
        Ok((
            w.parse()
                .map_err(|_| format!("bad resolution {:?}", self.resolution))?,
            h.parse()
                .map_err(|_| format!("bad resolution {:?}", self.resolution))?,
        ))
    }

    pub fn token(&self) -> Option<String> {
        let path = self.token_file.as_ref()?;
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                let t = raw.trim();
                (!t.is_empty()).then(|| t.to_string())
            }
            Err(e) => {
                eprintln!("nap: read token {path}: {e}");
                None
            }
        }
    }
}

fn candidate_paths() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("NAP_CONFIG") {
        v.push(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        v.push(format!("{home}/.config/nap/config.toml"));
    }
    v.push("/etc/nap/config.toml".into());
    v
}

pub fn expand_tilde(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_string(),
    }
}
