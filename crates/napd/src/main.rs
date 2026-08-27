//! napd: the nap receiver.
//!
//! Owns everything the sender used to reach over ssh for: the kiosk unit,
//! the published sender address, and (indirectly, via the kiosk unit's
//! Exec hooks) the panel. The sender only ever asserts a session; napd
//! makes it true and tears it down when the lease expires.
//!
//! The kiosk unit itself carries the hardened behavior (SIGKILL stop,
//! DRM force-on in ExecStartPre, force-off in ExecStopPost); napd never
//! touches sysfs directly except to *read* connector state for status.

use nap_proto::{ErrorReply, SessionReply, SessionRequest, Status};
use std::io::Read;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_BODY: usize = 64 * 1024;

#[derive(Clone)]
struct Config {
    bind: String,
    token: Option<Vec<u8>>,
    kiosk_unit: String,
    connector: String,
    host_file: String,
    lease: Duration,
}

struct Session {
    sender_addr: String,
    expires_at: Instant,
}

type State = Arc<Mutex<Option<Session>>>;

fn main() {
    let cfg = parse_args();
    let state: State = Arc::new(Mutex::new(None));

    // Adopt a kiosk that outlived a napd restart: give it one lease to be
    // renewed by the sender's next tick, or the reaper tears it down. A
    // fresh napd must never leave an orphaned kiosk streaming forever.
    if kiosk_state(&cfg) == "active" {
        let addr = std::fs::read_to_string(&cfg.host_file)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        println!("adopting running kiosk (sender {addr:?}), one lease to renew");
        *state.lock().unwrap() = Some(Session {
            sender_addr: addr,
            expires_at: Instant::now() + cfg.lease,
        });
    }

    {
        let cfg = cfg.clone();
        let state = state.clone();
        std::thread::spawn(move || reaper(cfg, state));
    }

    let server = tiny_http::Server::http(&cfg.bind)
        .unwrap_or_else(|e| die(&format!("bind {}: {e}", cfg.bind)));
    println!("napd listening on {}", cfg.bind);

    for request in server.incoming_requests() {
        handle(&cfg, &state, request);
    }
}

fn reaper(cfg: Config, state: State) {
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let expired = {
            let mut s = state.lock().unwrap();
            match s.as_ref() {
                Some(sess) if Instant::now() >= sess.expires_at => {
                    *s = None;
                    true
                }
                _ => false,
            }
        };
        if expired {
            println!("lease expired, tearing down");
            stop_kiosk(&cfg);
        }
    }
}

fn handle(cfg: &Config, state: &State, mut request: tiny_http::Request) {
    if !authorized(cfg, &request) {
        respond_json(
            request,
            401,
            &ErrorReply {
                error: "unauthorized".into(),
            },
        );
        return;
    }

    let method = request.method().clone();
    let url = request.url().to_string();
    match (method.as_str(), url.as_str()) {
        ("PUT", "/v1/session") => {
            let mut body = String::new();
            let ok = request
                .as_reader()
                .take(MAX_BODY as u64)
                .read_to_string(&mut body)
                .is_ok();
            let req: SessionRequest = match ok.then(|| serde_json::from_str(&body).ok()).flatten() {
                Some(r) => r,
                None => {
                    respond_json(
                        request,
                        400,
                        &ErrorReply {
                            error: "bad session body".into(),
                        },
                    );
                    return;
                }
            };
            if req.sender_addr.is_empty() {
                respond_json(
                    request,
                    400,
                    &ErrorReply {
                        error: "empty sender_addr".into(),
                    },
                );
                return;
            }
            let reply = assert_session(cfg, state, req);
            respond_json(request, 200, &reply);
        }
        ("DELETE", "/v1/session") => {
            println!("session delete");
            *state.lock().unwrap() = None;
            stop_kiosk(cfg);
            respond_json(request, 200, &status(cfg, state));
        }
        ("GET", "/v1/status") => {
            respond_json(request, 200, &status(cfg, state));
        }
        _ => {
            respond_json(
                request,
                404,
                &ErrorReply {
                    error: "not found".into(),
                },
            );
        }
    }
}

/// The idempotent core. One fix per concern, mirrors the sender's ladder
/// discipline: publish the address, keep the kiosk up, restart it if the
/// address changed under it (moonlight reads the host file once per
/// connect loop; a live stream to a stale address never heals on its own).
fn assert_session(cfg: &Config, state: &State, req: SessionRequest) -> SessionReply {
    let mut changed = false;

    let published = std::fs::read_to_string(&cfg.host_file)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if published != req.sender_addr {
        write_host_file(cfg, &req.sender_addr);
        changed = true;
    }

    let kiosk_was = kiosk_state(cfg);
    if kiosk_was != "active" {
        println!("starting kiosk for {}", req.sender_addr);
        systemctl(&["start", &cfg.kiosk_unit]);
        changed = true;
    } else {
        let streaming_to = {
            let s = state.lock().unwrap();
            s.as_ref().map(|sess| sess.sender_addr.clone())
        };
        if let Some(prev) = streaming_to {
            if !prev.is_empty() && prev != req.sender_addr {
                println!(
                    "sender moved {prev} -> {}, restarting kiosk",
                    req.sender_addr
                );
                systemctl(&["restart", &cfg.kiosk_unit]);
                changed = true;
            }
        }
    }

    *state.lock().unwrap() = Some(Session {
        sender_addr: req.sender_addr,
        expires_at: Instant::now() + cfg.lease,
    });

    SessionReply {
        changed,
        lease_secs: cfg.lease.as_secs(),
        status: status(cfg, state),
    }
}

fn status(cfg: &Config, state: &State) -> Status {
    let (session, sender_addr, remaining) = {
        let s = state.lock().unwrap();
        match s.as_ref() {
            Some(sess) => (
                true,
                Some(sess.sender_addr.clone()),
                Some(
                    sess.expires_at
                        .saturating_duration_since(Instant::now())
                        .as_secs(),
                ),
            ),
            None => (false, None, None),
        }
    };
    Status {
        session,
        sender_addr,
        lease_remaining_secs: remaining,
        kiosk: kiosk_state(cfg),
        monitor: monitor_state(cfg),
    }
}

fn stop_kiosk(cfg: &Config) {
    systemctl(&["stop", &cfg.kiosk_unit]);
}

fn kiosk_state(cfg: &Config) -> String {
    let out = Command::new("systemctl")
        .args(["is-active", &cfg.kiosk_unit])
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => "unknown".into(),
    }
}

fn monitor_state(cfg: &Config) -> String {
    std::fs::read_to_string(format!("/sys/class/drm/{}/enabled", cfg.connector))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn systemctl(args: &[&str]) {
    match Command::new("systemctl").args(args).status() {
        Ok(st) if st.success() => {}
        Ok(st) => eprintln!("systemctl {args:?} exited {st}"),
        Err(e) => eprintln!("systemctl {args:?}: {e}"),
    }
}

fn write_host_file(cfg: &Config, addr: &str) {
    let path = std::path::Path::new(&cfg.host_file);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    let write = std::fs::write(&tmp, addr).and_then(|_| std::fs::rename(&tmp, path));
    match write {
        Ok(()) => println!("published sender {addr}"),
        Err(e) => eprintln!("write {}: {e}", cfg.host_file),
    }
}

fn authorized(cfg: &Config, request: &tiny_http::Request) -> bool {
    let Some(token) = &cfg.token else { return true };
    let presented = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("authorization"))
        .map(|h| h.value.as_str().as_bytes().to_vec())
        .unwrap_or_default();
    let want = [b"Bearer ", token.as_slice()].concat();
    constant_time_eq(&presented, &want)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn respond_json<T: serde::Serialize>(request: tiny_http::Request, code: u16, body: &T) {
    let json = serde_json::to_string(body).unwrap_or_else(|_| "{}".into());
    let response = tiny_http::Response::from_string(json)
        .with_status_code(code)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                .expect("static header"),
        );
    if let Err(e) = request.respond(response) {
        eprintln!("respond: {e}");
    }
}

fn parse_args() -> Config {
    let mut cfg = Config {
        bind: "127.0.0.1:46270".into(),
        token: None,
        kiosk_unit: "cage-tty1.service".into(),
        connector: "card1-HDMI-A-1".into(),
        host_file: "/var/lib/nap/host".into(),
        lease: Duration::from_secs(20),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut val = |name: &str| {
            args.next()
                .unwrap_or_else(|| die(&format!("{name} needs a value")))
        };
        match arg.as_str() {
            "--bind" => cfg.bind = val("--bind"),
            "--kiosk-unit" => cfg.kiosk_unit = val("--kiosk-unit"),
            "--connector" => cfg.connector = val("--connector"),
            "--host-file" => cfg.host_file = val("--host-file"),
            "--lease-secs" => {
                cfg.lease = Duration::from_secs(
                    val("--lease-secs")
                        .parse()
                        .unwrap_or_else(|_| die("--lease-secs must be a number")),
                )
            }
            "--token-file" => {
                let path = val("--token-file");
                let raw = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| die(&format!("read {path}: {e}")));
                let token = raw.trim();
                if token.is_empty() {
                    die(&format!("{path} is empty"));
                }
                cfg.token = Some(token.as_bytes().to_vec());
            }
            "--help" | "-h" => {
                println!(
                    "usage: napd [--bind ADDR:PORT] [--token-file PATH] \
                     [--kiosk-unit UNIT] [--connector NAME] [--host-file PATH] \
                     [--lease-secs N]"
                );
                std::process::exit(0);
            }
            other => die(&format!("unknown flag {other}")),
        }
    }
    cfg
}

fn die(msg: &str) -> ! {
    eprintln!("napd: {msg}");
    std::process::exit(2);
}
