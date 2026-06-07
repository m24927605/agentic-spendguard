//! Loopback / allowed-CIDR helper for the HTTP companion.
//!
//! Per `docs/specs/coverage/D09_kong_ai_gateway/review-standards.md`
//! §2.8 the listener binds `127.0.0.1` by default; the explicit
//! `allow_pod_network` opt-in lets operators expose the listener to
//! pod-network IPs but does NOT relax the mTLS gate (§9.2). This
//! module centralises the IP-shape check so SLICE 1 + SLICE 6 share
//! the same gate logic.

use std::net::IpAddr;

/// True when the supplied address is `127.0.0.0/8` or `::1`. Matches
/// the standard library's `is_loopback` rule.
pub fn is_loopback(addr: &IpAddr) -> bool {
    addr.is_loopback()
}

/// True when the supplied address is a private RFC1918 / RFC4193 /
/// link-local CIDR. Pod-network exposure is typically scoped to one of
/// these ranges; binding a public IP is rejected upstream by the
/// `run_companion` startup gate.
pub fn is_pod_network(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            v4.is_private() || v4.is_link_local() || v4.is_loopback() || v4.is_unspecified()
            // 0.0.0.0 — only with allow_pod_network
        }
        IpAddr::V6(v6) => {
            // Unique local fc00::/7 + link-local fe80::/10 + loopback ::1.
            v6.is_loopback()
                || v6.is_unspecified() // ::
                || v6.segments()[0] & 0xfe00 == 0xfc00
                || v6.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn loopback_classifier() {
        assert!(is_loopback(&IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_loopback(&IpAddr::V4(Ipv4Addr::new(127, 255, 1, 1))));
        assert!(!is_loopback(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!is_loopback(&IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
    }

    #[test]
    fn pod_network_classifier_accepts_rfc1918() {
        for ip in [
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(172, 16, 0, 1),
            Ipv4Addr::new(192, 168, 0, 1),
        ] {
            assert!(is_pod_network(&IpAddr::V4(ip)));
        }
    }

    #[test]
    fn pod_network_classifier_rejects_public() {
        // 8.8.8.8 is a public DNS; binding the companion there would
        // be a severe security regression. We explicitly assert the
        // classifier won't pass it.
        assert!(!is_pod_network(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }
}
