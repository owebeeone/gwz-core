//! The rows that read this host's real WinHTTP machine proxy (plan step 4.6). They are `#[ignore]`d: they change
//! nothing, but what they see is whatever the machine setting is now, and only the guarded transaction of the TD10
//! protocol (`netsh winhttp set proxy` under a deadline guard, restored and verified byte-identical after every row)
//! may set it. The runner (`gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-12-proxy`) sets the setting,
//! runs this test with `--ignored --nocapture`, reads its one `MACHINE_PROXY_ROW` line, and restores.

use super::{MachineProxy, capture_machine};

#[test]
#[ignore = "reads this host's WinHTTP machine proxy, which the guarded TD10 transaction sets first (plan step 4.6)"]
fn report_the_captured_machine_proxy() {
    let machine = capture_machine();
    let probes = std::env::var("GWZ_ROW_PROBES").unwrap_or_default();
    let report = match &machine {
        MachineProxy::Direct => serde_json::json!({ "class": "direct" }),
        MachineProxy::Refused(text) => {
            serde_json::json!({ "class": "refused", "refusal": text.as_ref() })
        }
        MachineProxy::Named(named) => {
            let decisions: Vec<_> = probes
                .split(',')
                .filter(|probe| !probe.is_empty())
                .filter_map(|probe| {
                    let (host, port) = probe.rsplit_once(':')?;
                    let port: u16 = port.parse().ok()?;
                    let host = host.trim_start_matches('[').trim_end_matches(']');
                    Some(serde_json::json!({
                        "host": host,
                        "port": port,
                        "bypassed": named.bypass.bypasses(host, port),
                    }))
                })
                .collect();
            serde_json::json!({
                "class": "named",
                "proxy": { "host": named.proxy.host, "port": named.proxy.port },
                "probes": decisions,
            })
        }
    };
    println!("MACHINE_PROXY_ROW {report}");
}
