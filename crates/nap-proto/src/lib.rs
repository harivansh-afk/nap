//! Wire types for the nap control plane.
//!
//! One session, asserted idempotently. `PUT /v1/session` both creates and
//! renews: the sender's steady tick doubles as the heartbeat, and the
//! receiver tears the session down on its own when the lease expires.
//!
//! Endpoints:
//!   PUT    /v1/session   body: SessionRequest  -> SessionReply
//!   DELETE /v1/session                         -> Status
//!   GET    /v1/status                          -> Status
//!
//! Auth (optional): `Authorization: Bearer <token>`.

use serde::{Deserialize, Serialize};

/// Sent by the sender on every supervise tick while the cast is on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRequest {
    /// Address the receiver's video client should stream from (the
    /// sender's LAN IP; the video path is deliberately not the control
    /// path, which may ride a slower overlay network).
    pub sender_addr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionReply {
    /// True when this assert changed receiver state (kiosk started or
    /// restarted, address rewritten). The sender treats a changed reply
    /// like one of its own fixes and re-runs its ladder immediately.
    pub changed: bool,
    /// Lease duration granted, in seconds. Assert again before it runs out.
    pub lease_secs: u64,
    pub status: Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    /// A live session exists (lease not yet expired).
    pub session: bool,
    /// The address the kiosk is currently pointed at, if any.
    pub sender_addr: Option<String>,
    /// Seconds until the lease expires, if a session is live.
    pub lease_remaining_secs: Option<u64>,
    /// systemd unit state of the kiosk ("active", "inactive", ...).
    pub kiosk: String,
    /// DRM connector driven state ("enabled", "disabled", ...).
    pub monitor: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorReply {
    pub error: String,
}
