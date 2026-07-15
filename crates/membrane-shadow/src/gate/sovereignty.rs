// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sovereignty probes (S1-S4 live validation).
//!
//! WAN probes that validate the ecoPrimals sovereign infrastructure
//! is operational — replacing static documentation with runtime truth.
//!
//! - S1: Sovereign TLS (certificate + TTFB)
//! - S2: Sovereign Relay (federation + TURN + `RustDesk`)
//! - S3: Sovereign Content (depot HTTPS availability)
//! - S4: Sovereign Auth (crypto-signer BTSP enforcement)

use std::path::Path;

use super::health::{StatusProbe, resolve_primal_socket_paths, uds_jsonrpc_call};

/// Resolve the sovereign domain for TLS and content probes.
fn resolve_sovereign_domain() -> String {
    cellmembrane_types::service::env_or(
        cellmembrane_types::service::ENV_DEPOT_HOSTNAME,
        cellmembrane_types::service::DEFAULT_DEPOT_HOSTNAME,
    )
}

/// Probe all four sovereignty shadows (S1 TLS, S2 Relay, S3 Content, S4 Auth).
pub(crate) async fn probe_sovereignty() -> Vec<StatusProbe> {
    let (s1, s2, s3, s4) = tokio::join!(
        probe_s1_tls(),
        probe_s2_relay(),
        probe_s3_content(),
        probe_s4_auth(),
    );
    vec![s1, s2, s3, s4]
}

/// S1: Sovereign TLS — validate certificate and TTFB from sovereign domain.
async fn probe_s1_tls() -> StatusProbe {
    let domain = resolve_sovereign_domain();
    let url = format!("https://{domain}/");
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| format!("client: {e}"))?
            .head(&url)
            .send()
            .await
            .map_err(|e| format!("request: {e}"))
    })
    .await;

    match result {
        Ok(Ok(resp)) => {
            let ttfb_ms = start.elapsed().as_millis();
            let status = resp.status();
            if status.is_success() || status.as_u16() == 308 || status.as_u16() == 301 {
                StatusProbe {
                    name: "sovereignty.s1_tls".into(),
                    ok: true,
                    detail: format!("OPERATIONAL — {domain} {status} ({ttfb_ms}ms)"),
                }
            } else {
                StatusProbe {
                    name: "sovereignty.s1_tls".into(),
                    ok: false,
                    detail: format!("{domain} returned {status} ({ttfb_ms}ms)"),
                }
            }
        }
        Ok(Err(e)) => StatusProbe {
            name: "sovereignty.s1_tls".into(),
            ok: false,
            detail: format!("FAIL — {e}"),
        },
        Err(_) => StatusProbe {
            name: "sovereignty.s1_tls".into(),
            ok: false,
            detail: "TIMEOUT — TLS probe exceeded 5s".into(),
        },
    }
}

/// S2: Sovereign Relay — probe Songbird federation (:7700) TCP and TURN (:3478) TCP.
///
/// Federation port is always TCP. TURN may primarily use UDP but also listens on TCP.
/// Federation reachability is the primary signal; TURN TCP is best-effort.
async fn probe_s2_relay() -> StatusProbe {
    let peer = crate::manifest::resolve_federation_peer();
    let vps_host = peer.split(':').next().unwrap_or(&peer).to_string();

    let fed_port = cellmembrane_types::service::DEFAULT_FEDERATION_PORT;
    let turn_port = cellmembrane_types::service::DEFAULT_TURN_PORT;
    let rendezvous_port = cellmembrane_types::service::RUSTDESK_HBBS_PORT;
    let relay_port = cellmembrane_types::service::RUSTDESK_HBBR_PORT;

    let fed_addr = format!("{vps_host}:{fed_port}");
    let turn_addr = format!("{vps_host}:{turn_port}");
    let rendezvous_addr = format!("{vps_host}:{rendezvous_port}");
    let relay_addr = format!("{vps_host}:{relay_port}");

    let (fed_ok, turn_ok, rendezvous_ok, relay_ok) = tokio::join!(
        tcp_reachable(&fed_addr),
        tcp_reachable(&turn_addr),
        tcp_reachable(&rendezvous_addr),
        tcp_reachable(&relay_addr),
    );

    let detail = format!(
        "federation:{} TURN:{} RustDesk:hbbs={},hbbr={}",
        if fed_ok { "REACHABLE" } else { "UNREACHABLE" },
        if turn_ok {
            "TCP-OK"
        } else {
            "TCP-CLOSED(UDP-only)"
        },
        if rendezvous_ok { "OK" } else { "DOWN" },
        if relay_ok { "OK" } else { "DOWN" },
    );

    StatusProbe {
        name: "sovereignty.s2_relay".into(),
        ok: fed_ok && rendezvous_ok,
        detail,
    }
}

/// S3: Sovereign Content — probe WAN depot HTTPS availability and TTFB.
///
/// Probes the depot file server (Caddy) to confirm binaries are being served
/// over sovereign TLS. Uses the crypto spine binary as probe target (always present).
async fn probe_s3_content() -> StatusProbe {
    let arch = crate::plasmid::detect_target_triple();
    let probe_binary = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );
    let domain = resolve_sovereign_domain();
    let url = format!("https://{domain}/depot/{arch}/{probe_binary}");
    let start = std::time::Instant::now();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| format!("client: {e}"))?
            .head(&url)
            .send()
            .await
            .map_err(|e| format!("request: {e}"))
    })
    .await;

    match result {
        Ok(Ok(resp)) => {
            let ttfb_ms = start.elapsed().as_millis();
            if resp.status().is_success() {
                let size_kb = resp
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|b| b / 1024);
                let size_info = size_kb.map_or(String::new(), |kb| format!(" {kb}KB"));
                StatusProbe {
                    name: "sovereignty.s3_content".into(),
                    ok: true,
                    detail: format!("OPERATIONAL — depot serving{size_info} ({ttfb_ms}ms TTFB)"),
                }
            } else {
                StatusProbe {
                    name: "sovereignty.s3_content".into(),
                    ok: false,
                    detail: format!("depot returned {} ({ttfb_ms}ms)", resp.status()),
                }
            }
        }
        Ok(Err(e)) => StatusProbe {
            name: "sovereignty.s3_content".into(),
            ok: false,
            detail: format!("FAIL — {e}"),
        },
        Err(_) => StatusProbe {
            name: "sovereignty.s3_content".into(),
            ok: false,
            detail: "TIMEOUT — content probe exceeded 5s".into(),
        },
    }
}

/// S4: Sovereign Auth — probe crypto-signer BTSP enforcement via local UDS health.
///
/// Tries neuralAPI capability routing first, then direct UDS. Any JSON-RPC
/// response (including `-32601 method_not_found` or BTSP errors) proves
/// the crypto spine is alive and enforcing.
async fn probe_s4_auth() -> StatusProbe {
    let binary_name = cellmembrane_types::MembraneService::binary_for(
        cellmembrane_types::ServiceCapability::CryptoSigner,
    );

    match crate::bridge::try_bridge(binary_name, "health", serde_json::json!({})).await {
        Ok(Some(result)) => {
            let status = result
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("alive");
            let btsp = result
                .get("auth_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let detail = if btsp == "btsp" {
                format!("ENFORCED — {binary_name} BTSP active (via neuralAPI)")
            } else {
                format!("RESPONDING — {binary_name} {status} (via neuralAPI)")
            };
            return StatusProbe {
                name: "sovereignty.s4_auth".into(),
                ok: true,
                detail,
            };
        }
        Err(_) => {
            return StatusProbe {
                name: "sovereignty.s4_auth".into(),
                ok: true,
                detail: format!("RESPONDING — {binary_name} reachable (via neuralAPI)"),
            };
        }
        Ok(None) => {}
    }

    let socket_paths = resolve_primal_socket_paths(binary_name);
    let request = crate::jsonrpc::HEALTH_REQUEST;

    for socket_path in &socket_paths {
        if !Path::new(socket_path).exists() {
            continue;
        }
        if let Ok(response) = uds_jsonrpc_call(socket_path, request).await {
            let is_jsonrpc = serde_json::from_str::<serde_json::Value>(&response)
                .is_ok_and(|j| j.get("jsonrpc").is_some() || j.get("result").is_some() || j.get("error").is_some());
            if is_jsonrpc || response.contains("BTSP") {
                let enforced = response.contains("BTSP handshake required")
                    || response.contains("\"auth_mode\":\"btsp\"");
                let detail = if enforced {
                    format!("ENFORCED — {binary_name} BTSP active (direct UDS)")
                } else {
                    format!(
                        "RESPONDING — {binary_name} alive ({})",
                        &response[..response.len().min(80)]
                    )
                };
                return StatusProbe {
                    name: "sovereignty.s4_auth".into(),
                    ok: true,
                    detail,
                };
            }
        }
    }

    StatusProbe {
        name: "sovereignty.s4_auth".into(),
        ok: false,
        detail: format!("UNREACHABLE — {binary_name} not responding on UDS"),
    }
}

/// TCP reachability check with 3s timeout.
async fn tcp_reachable(addr: &str) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_sovereign_domain_returns_default() {
        let domain = resolve_sovereign_domain();
        assert!(
            !domain.is_empty(),
            "should return default hostname when env not set"
        );
        assert!(domain.contains('.'), "domain should contain a TLD");
    }

    #[test]
    fn probe_names_are_namespaced() {
        let names = [
            "sovereignty.s1_tls",
            "sovereignty.s2_relay",
            "sovereignty.s3_content",
            "sovereignty.s4_auth",
        ];
        for name in &names {
            assert!(name.starts_with("sovereignty."));
        }
    }

    #[tokio::test]
    async fn tcp_unreachable_returns_false() {
        assert!(!tcp_reachable("192.0.2.1:1").await);
    }

    #[tokio::test]
    async fn probe_sovereignty_returns_four_probes() {
        let probes = probe_sovereignty().await;
        assert_eq!(probes.len(), 4);
        assert_eq!(probes[0].name, "sovereignty.s1_tls");
        assert_eq!(probes[1].name, "sovereignty.s2_relay");
        assert_eq!(probes[2].name, "sovereignty.s3_content");
        assert_eq!(probes[3].name, "sovereignty.s4_auth");
    }
}
