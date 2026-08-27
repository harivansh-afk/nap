//! HTTP client for the napd control plane.

use nap_proto::{SessionReply, SessionRequest, Status};
use std::time::Duration;

pub struct Receiver {
    agent: ureq::Agent,
    base: String,
    token: Option<String>,
}

pub enum ClientError {
    /// TCP/DNS/timeout: the receiver is not reachable right now.
    Unreachable(String),
    /// The receiver answered with an error status (401, 400, ...).
    Rejected(u16),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Unreachable(e) => write!(f, "receiver unreachable: {e}"),
            ClientError::Rejected(401) => write!(f, "receiver refused token"),
            ClientError::Rejected(code) => write!(f, "receiver rejected request ({code})"),
        }
    }
}

impl Receiver {
    pub fn new(base: &str, token: Option<String>) -> Receiver {
        Receiver {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(5))
                .timeout(Duration::from_secs(10))
                .build(),
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    pub fn assert_session(&self, sender_addr: &str) -> Result<SessionReply, ClientError> {
        let req = self.request("PUT", "/v1/session");
        let resp = req
            .send_json(SessionRequest {
                sender_addr: sender_addr.to_string(),
            })
            .map_err(map_err)?;
        resp.into_json()
            .map_err(|e| ClientError::Unreachable(e.to_string()))
    }

    pub fn delete_session(&self) -> Result<Status, ClientError> {
        let resp = self
            .request("DELETE", "/v1/session")
            .call()
            .map_err(map_err)?;
        resp.into_json()
            .map_err(|e| ClientError::Unreachable(e.to_string()))
    }

    pub fn status(&self) -> Result<Status, ClientError> {
        let resp = self.request("GET", "/v1/status").call().map_err(map_err)?;
        resp.into_json()
            .map_err(|e| ClientError::Unreachable(e.to_string()))
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let mut req = self.agent.request(method, &format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        req
    }
}

fn map_err(e: ureq::Error) -> ClientError {
    match e {
        ureq::Error::Status(code, _) => ClientError::Rejected(code),
        ureq::Error::Transport(t) => ClientError::Unreachable(t.to_string()),
    }
}
