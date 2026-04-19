//! Locator builders + port-walk.

use std::net::{TcpListener, UdpSocket};
use stylos_common::{Result, StylosError};

pub fn listen_endpoints(port: u16) -> Vec<String> {
    vec![format!("udp/0.0.0.0:{port}"), format!("tcp/0.0.0.0:{port}")]
}

pub fn walk_available_port(start: u16, cap: u16) -> Result<u16> {
    for p in start..start.saturating_add(cap) {
        let tcp_ok = TcpListener::bind(("0.0.0.0", p)).is_ok();
        let udp_ok = UdpSocket::bind(("0.0.0.0", p)).is_ok();
        if tcp_ok && udp_ok { return Ok(p); }
    }
    Err(StylosError::Transport(format!(
        "no free port in [{start}, {}) for TCP+UDP dual bind", start.saturating_add(cap)
    )))
}
