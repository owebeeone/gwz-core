use super::AgentSource;
use crate::git::endpoint::agent_job::{Control, wall_clock};
use std::{io, path::PathBuf, sync::Arc, time::Duration};

fn control() -> Arc<Control> {
    Control::scripted(None, Duration::ZERO, Duration::from_secs(5), wall_clock())
}

#[test]
fn the_optional_path_of_a_runtime_round_trips_through_the_enum() {
    assert_eq!(AgentSource::from_path(None), AgentSource::None);
    let path = PathBuf::from("agent.sock");
    assert_eq!(
        AgentSource::from_path(Some(path.clone())),
        AgentSource::Path(path)
    );
    assert_eq!(AgentSource::default(), AgentSource::None);
}

#[test]
fn no_source_is_the_no_agent_refusal() {
    let error = AgentSource::None.open(control()).err().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;

        #[test]
        fn a_path_source_connects_the_socket_it_names() {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("agent.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let peer = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0; 5];
                socket.read_exact(&mut request).unwrap();
                assert_eq!(request, [0, 0, 0, 1, 11]);
                socket.write_all(&[0, 0, 0, 5, 12, 0, 0, 0, 0]).unwrap();
            });
            let mut agent = AgentSource::Path(path).open(control()).unwrap();
            assert!(agent.identities().unwrap().is_empty());
            peer.join().unwrap();
        }

        #[test]
        fn a_path_source_that_serves_nothing_is_unavailable_without_naming_the_path() {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("sentinel-missing.sock");
            let error = AgentSource::Path(path).open(control()).err().unwrap();
            assert!(!error.to_string().contains("sentinel"), "{error}");
        }
    } else {
        #[test]
        fn a_path_source_has_no_form_on_windows() {
            let error = AgentSource::Path(PathBuf::from("agent.sock"))
                .open(control())
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
        }
    }
}
