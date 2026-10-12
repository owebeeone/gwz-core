//! The tunnel rows through this host's REAL machine proxy (plan step 4.7). `#[ignore]`d like `host_rows`: the guarded
//! TD10 transaction (the runner of `gwz-core-evidence/.../runs/2026-10-12-proxy`) sets the machine setting to name the
//! fixture proxy this test starts on `GWZ_ROW_PROXY_PORT`, runs the test, and restores the setting. The test reads the
//! setting with the product's own capture, makes it the endpoint's proxy, opens one discovery request against a local
//! HTTPS origin, and prints one `TUNNEL_ROW {json}` line for the runner to judge. Nothing it does touches the host's
//! settings. Decoy proxies named by the process environment must see nothing (design §6, RX/he01 to he12).

use super::tunnel_tests::{discover, plain_server, port_of, run};
use super::{MachineProxy, capture_machine};
use crate::git::endpoint::https_connection::MachineSetting;
use crate::git::endpoint::https_fixture::Tunnel;
use crate::transport_host::HttpsOpenFailure;
use gwz_transport::protocol::GitService;
use std::sync::atomic::Ordering;

fn number(name: &str) -> Option<u16> {
    std::env::var(name).ok().and_then(|text| text.parse().ok())
}

#[test]
#[ignore = "runs under the guarded TD10 machine-proxy transaction, which sets the machine setting first (plan step 4.7)"]
fn report_the_tunnel_through_the_real_machine_proxy() {
    let proxy_port = number("GWZ_ROW_PROXY_PORT").unwrap_or(0);
    let status = number("GWZ_ROW_STATUS").unwrap_or(200);
    let origin = std::env::var("GWZ_ROW_ORIGIN").unwrap_or_else(|_| "proxied".into());
    let report = run(async move {
        let server = plain_server().await;
        let port = port_of(&server);
        let tunnel = Tunnel::forwarding_on(proxy_port, status, port).await;
        let decoy = match number("GWZ_ROW_DECOY_PORT") {
            Some(decoy_port) => Some(Tunnel::forwarding_on(decoy_port, 200, port).await),
            None => None,
        };
        let machine = capture_machine();
        let class = match &machine {
            MachineProxy::Direct => "direct",
            MachineProxy::Named(_) => "named",
            MachineProxy::Refused(_) => "refused",
        };
        let mut config = server.config();
        machine.apply(&mut config);
        let destination = if origin == "loopback" {
            server.url.clone()
        } else {
            format!("https://proxied.test:{port}/repo")
        };
        let outcome = match &config.machine {
            // The configuration builds; the https open is what a refused setting refuses (decision D6).
            MachineSetting::Refused(text) => serde_json::json!({ "open_refused": text.as_ref() }),
            _ => match discover(&server, config.clone(), &destination).await {
                Ok(()) => serde_json::json!({ "result": "ok" }),
                Err(failure) => {
                    let code = format!("{:?}", failure.code);
                    let http_status = failure.facts.as_ref().and_then(|f| f.http_status);
                    let reason = HttpsOpenFailure {
                        failure,
                        anonymous: None,
                        attempts: None,
                        service: Some(GitService::UploadPackAdvertisement),
                        helpers_disabled: false,
                        cli_hint: true,
                        windows: true,
                        host: None,
                        redirect: None,
                        machine: config.machine.clone(),
                    }
                    .reason();
                    serde_json::json!({ "result": "failed", "code": code, "http_status": http_status, "reason": reason })
                }
            },
        };
        let seen = tunnel.seen.lock().unwrap().clone();
        serde_json::json!({
            "class": class,
            "origin": origin,
            "outcome": outcome,
            "tunnel_requests": seen.iter().map(|head| head.lines().next().unwrap_or("").to_owned()).collect::<Vec<_>>(),
            "tunnel_saw_authorization": seen.iter().any(|head| head.to_ascii_lowercase().contains("authorization")),
            "decoy_requests": decoy.map(|decoy| decoy.seen.lock().unwrap().len()),
            "origin_connections": server.connections.load(Ordering::SeqCst),
            "origin_port": port,
        })
    });
    println!("TUNNEL_ROW {report}");
}
