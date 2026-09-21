#![allow(dead_code)]

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        #[path = "../../../src/git/endpoint/agent_job.rs"]
        mod agent_job;
        #[path = "../../../src/git/endpoint/ssh_key_container.rs"]
        mod ssh_key_container;

        use agent_job::Job;
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use std::{
            io,
            sync::{
                Arc,
                atomic::{AtomicUsize, Ordering},
                mpsc,
            },
            task::{Context, Poll, Waker},
            time::{Duration, Instant},
        };

        fn finish<T: Send + 'static>(job: &mut Job<T>) -> io::Result<T> {
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if let Poll::Ready(result) = job.poll_result(&mut Context::from_waker(Waker::noop())) {
                    return result;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn check(text: String) -> io::Result<()> {
            let mut job = Job::start(
                Some(Instant::now() + Duration::from_secs(2)),
                Duration::from_secs(1),
                move |control| ssh_key_container::check(&text, &control),
            )?;
            finish(&mut job)
        }

        fn armor(label: &str, bytes: &[u8]) -> String {
            format!(
                "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
                STANDARD.encode(bytes)
            )
        }

        fn ssh_string(value: &[u8], out: &mut Vec<u8>) {
            out.extend_from_slice(&(value.len() as u32).to_be_bytes());
            out.extend_from_slice(value);
        }

        fn openssh(cipher: &[u8], kdf: &[u8], options: &[u8]) -> Vec<u8> {
            let mut value = b"openssh-key-v1\0".to_vec();
            ssh_string(cipher, &mut value);
            ssh_string(kdf, &mut value);
            ssh_string(options, &mut value);
            value.push(0);
            value
        }

        #[test]
        fn accepts_unencrypted_traditional_pkcs8_and_openssh_framing() {
            for label in ["RSA PRIVATE KEY", "DSA PRIVATE KEY", "EC PRIVATE KEY"] {
                check(armor(label, &[1, 2, 3, 4])).unwrap();
            }
            check(armor(
                "PRIVATE KEY",
                &[0x30, 0x07, 0x02, 1, 0, 0x30, 0, 0x04, 0],
            ))
            .unwrap();
            check(armor(
                "OPENSSH PRIVATE KEY",
                &openssh(b"none", b"none", b""),
            ))
            .unwrap();
        }

        #[test]
        fn rejects_encryption_headers_and_extra_or_unknown_armor() {
            assert!(check(armor("ENCRYPTED PRIVATE KEY", &[1, 2])).is_err());
            assert!(check("-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nAQ==\n-----END RSA PRIVATE KEY-----\n".into()).is_err());
            assert!(
                check(format!(
                    "{}-----BEGIN RSA PRIVATE KEY-----\nAQ==\n-----END RSA PRIVATE KEY-----\n",
                    "x"
                ))
                .is_err()
            );
            assert!(
                check(format!(
                    "{}-----BEGIN RSA PRIVATE KEY-----\nAQ==\n-----END RSA PRIVATE KEY-----\n",
                    armor("RSA PRIVATE KEY", &[1])
                ))
                .is_err()
            );
            assert!(check(armor("UNKNOWN", &[1])).is_err());
            assert!(
                check("-----BEGIN RSA PRIVATE KEY-----\nAQ==\n-----END EC PRIVATE KEY-----\n".into())
                    .is_err()
            );
            assert!(
                check("-----BEGIN RSA PRIVATE KEY-----\n\n-----END RSA PRIVATE KEY-----\n".into()).is_err()
            );
        }

        #[test]
        fn rejects_truncated_and_ambiguous_declared_lengths() {
            assert!(check(armor("PRIVATE KEY", &[0x30, 0x07, 0x02, 1, 0])).is_err());
            assert!(check(armor("PRIVATE KEY", &[0x30, 0x01, 0x02, 1, 2])).is_err());
            assert!(check(armor("PRIVATE KEY", &[0x30, 0x82, 0xff, 0xff, 0x02, 1, 0])).is_err());
            assert!(
                check(armor(
                    "PRIVATE KEY",
                    &[0x30, 0x07, 0x02, 1, 2, 0x30, 0, 0x04, 0]
                ))
                .is_err()
            );
            assert!(
                check(armor(
                    "OPENSSH PRIVATE KEY",
                    &openssh(b"aes256-ctr", b"bcrypt", &[0xff; 16])
                ))
                .is_err()
            );
            assert!(check(armor("OPENSSH PRIVATE KEY", &[b'o'; 15])).is_err());
        }

        #[test]
        fn rejects_invalid_base64_even_after_a_valid_prefix() {
            assert!(
                check("-----BEGIN RSA PRIVATE KEY-----\nAQ==A\n-----END RSA PRIVATE KEY-----\n".into())
                    .is_err()
            );
            let mut body = STANDARD.encode([1u8; 60_000]);
            body.push('!');
            assert!(
                check(format!(
                    "-----BEGIN RSA PRIVATE KEY-----\n{body}\n-----END RSA PRIVATE KEY-----\n"
                ))
                .is_err()
            );
        }

        #[test]
        fn cancellation_is_observed_while_scanning_bounded_chunks() {
            let (started, seen) = mpsc::channel();
            let text = armor("RSA PRIVATE KEY", &[1u8; 700_000]);
            let mut job = Job::start(None, Duration::from_secs(1), move |control| {
                started.send(()).unwrap();
                ssh_key_container::check(&text, &control)
            })
            .unwrap();
            seen.recv_timeout(Duration::from_secs(1)).unwrap();
            job.cancel();
            assert_eq!(
                finish(&mut job).unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
        }

        #[test]
        fn checked_scan_observes_cancellation_after_first_chunk() {
            let (first, reached) = mpsc::channel();
            let (release, released) = mpsc::channel();
            let observed = Arc::new(AtomicUsize::new(0));
            let worker_observed = Arc::clone(&observed);
            let input = vec![b'a'; 1_048_000];
            let mut job = Job::start(None, Duration::from_secs(1), move |control| {
                ssh_key_container::scan_for_test(&input, &control, |_| {
                    let count = worker_observed.fetch_add(1, Ordering::AcqRel) + 1;
                    if count == 128 {
                        first.send(()).unwrap();
                        released.recv().unwrap();
                        return true;
                    }
                    false
                })?;
                control.check()
            })
            .unwrap();
            reached.recv_timeout(Duration::from_secs(1)).unwrap();
            job.cancel();
            release.send(()).unwrap();
            assert_eq!(
                finish(&mut job).unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
            assert_eq!(observed.load(Ordering::Acquire), 128);
        }
    }
}
