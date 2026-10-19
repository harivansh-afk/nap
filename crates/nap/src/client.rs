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
            agent: ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(5)))
                .timeout_global(Some(Duration::from_secs(10)))
                .build()
                .into(),
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    pub fn assert_session(&self, sender_addr: &str) -> Result<SessionReply, ClientError> {
        let req = self.authorize(self.agent.put(format!("{}/v1/session", self.base)));
        let mut resp = req
            .send_json(SessionRequest {
                sender_addr: sender_addr.to_string(),
            })
            .map_err(map_err)?;
        resp.body_mut()
            .read_json()
            .map_err(|e| ClientError::Unreachable(e.to_string()))
    }

    pub fn delete_session(&self) -> Result<Status, ClientError> {
        let mut resp = self
            .authorize(self.agent.delete(format!("{}/v1/session", self.base)))
            .call()
            .map_err(map_err)?;
        resp.body_mut()
            .read_json()
            .map_err(|e| ClientError::Unreachable(e.to_string()))
    }

    pub fn status(&self) -> Result<Status, ClientError> {
        let mut resp = self
            .authorize(self.agent.get(format!("{}/v1/status", self.base)))
            .call()
            .map_err(map_err)?;
        resp.body_mut()
            .read_json()
            .map_err(|e| ClientError::Unreachable(e.to_string()))
    }

    fn authorize<B>(&self, mut req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        req
    }
}

fn map_err(e: ureq::Error) -> ClientError {
    match e {
        ureq::Error::StatusCode(code) => ClientError::Rejected(code),
        other => ClientError::Unreachable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_requests_preserve_auth_json_and_rejections() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let receiver = Receiver::new(
            &format!("http://{}", server.server_addr()),
            Some("test-token".into()),
        );
        let worker = std::thread::spawn(move || {
            let status = serde_json::json!({
                "session": false,
                "sender_addr": null,
                "lease_remaining_secs": null,
                "kiosk": "inactive",
                "monitor": "disabled"
            });
            for (method, path, code) in [
                ("PUT", "/v1/session", 200),
                ("GET", "/v1/status", 200),
                ("DELETE", "/v1/session", 200),
                ("GET", "/v1/status", 401),
            ] {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.method().as_str(), method);
                assert_eq!(request.url(), path);
                assert!(request.headers().iter().any(|header| {
                    header.field.equiv("Authorization")
                        && header.value.as_str() == "Bearer test-token"
                }));
                let body = if method == "PUT" {
                    let sent: SessionRequest =
                        serde_json::from_reader(request.as_reader()).unwrap();
                    assert_eq!(sent.sender_addr, "192.0.2.1");
                    serde_json::json!({"changed": true, "lease_secs": 20, "status": status})
                } else {
                    status.clone()
                };
                request
                    .respond(
                        tiny_http::Response::from_string(body.to_string()).with_status_code(code),
                    )
                    .unwrap();
            }
        });
        assert!(
            receiver
                .assert_session("192.0.2.1")
                .unwrap_or_else(|e| panic!("{e}"))
                .changed
        );
        assert!(!receiver.status().unwrap_or_else(|e| panic!("{e}")).session);
        assert_eq!(
            receiver
                .delete_session()
                .unwrap_or_else(|e| panic!("{e}"))
                .kiosk,
            "inactive"
        );
        assert!(matches!(receiver.status(), Err(ClientError::Rejected(401))));
        worker.join().unwrap();
    }
}
