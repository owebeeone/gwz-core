//! HTTPS credential route integration harness.
cfg_if::cfg_if! { if #[cfg(unix)] {
            use super::*;
            use super::fixture::{attach, input, response, ConnectionId, Server};
            use base64::{engine::general_purpose::STANDARD, Engine as _};
            use hyper::header::AUTHORIZATION;
            use http_body_util::BodyExt;
            use std::{
                fs,
                path::Path,
                sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}},
                time::Duration,
            };
            use tempfile::TempDir;
            use tokio_util::sync::CancellationToken;

            fn auth_header(token: &str) -> Vec<u8> {
                format!("Basic {}", STANDARD.encode(format!("fixture:{token}"))).into_bytes()
            }

            fn recording_gh(
                log: &Path,
                host_a: &str,
                token_a: &str,
                host_b: &str,
                token_b: &str,
            ) -> (TempDir, https_auth::Config) {
                let directory = tempfile::tempdir().unwrap();
                let executable = directory.path().join("gh");
                crate::git::endpoint::helper_script::write_git_fixture(
                    &executable,
                    "\
if [ \"${1} ${2} ${3} ${4}\" != '-c core.askPass= credential fill' ]; then exit 4; fi\n\
input=$(/bin/cat)\n\
printf '%s\\n' \"$input\" >> \"$GH_LOG\"\n\
case \"$input\" in\n\
  *\"url=https://$GH_HOST_A/\"*) token=\"$GH_TOKEN_A\" ;;\n\
  *\"url=https://$GH_HOST_B/\"*) token=\"$GH_TOKEN_B\" ;;\n\
  *) exit 5 ;;\n\
esac\n\
printf 'username=fixture\\npassword=%s\\n\\n' \"$token\"\n",
                );
                (
                    directory,
                    https_auth::Config {
                        executable,
                        environment: vec![
                            ("GH_LOG".into(), log.as_os_str().into()),
                            ("GH_HOST_A".into(), host_a.into()),
                            ("GH_TOKEN_A".into(), token_a.into()),
                            ("GH_HOST_B".into(), host_b.into()),
                            ("GH_TOKEN_B".into(), token_b.into()),
                        ],
                    },
                )
            }

            fn authority(url: &str) -> String {
                let parsed = url::Url::parse(url).unwrap();
                format!(
                    "localhost:{}",
                    parsed.port_or_known_default().unwrap()
                )
            }

            async fn finish(prepared: Prepared) {
                let (stream, task) = attach(prepared);
                stream.end_write().await.unwrap();
                let mut buffer = [0; 113];
                while stream.read(&mut buffer).await.unwrap() != 0 {}
                stream.close().await.unwrap();
                task.await.unwrap();
            }


#[path="https_auth_integration_tests/challenge.rs"]
mod challenge;
#[path="https_auth_integration_tests/route.rs"]
mod route;
#[path="https_auth_integration_tests/policy.rs"]
mod policy;

} }
