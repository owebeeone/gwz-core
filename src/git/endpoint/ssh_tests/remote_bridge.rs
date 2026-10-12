use crate::git::endpoint::ssh_channel;
use crate::git::endpoint::ssh_remote;
use crate::git::endpoint::stream_io;

use git2::transport::{Service, SmartSubtransport};
use gwz_transport::stream::{Config, Side, Stream};
use ssh_channel::GitService;
use ssh_remote::{OpenStream, RemoteTransport};
use std::{
    io,
    sync::{Arc, Mutex},
};
use stream_io::BlockingStream;

struct Fake {
    calls: Mutex<Vec<String>>,
    streams: Mutex<Vec<gwz_transport::stream::MessageEndpoint>>,
}
impl OpenStream for Fake {
    fn open(&self, url: &str, _: GitService) -> io::Result<BlockingStream> {
        self.calls.lock().unwrap().push(url.to_owned());
        let (stream, endpoint) = Stream::new(Config::new("bridge", 1, Side::Initiator)).unwrap();
        self.streams.lock().unwrap().push(endpoint);
        Ok(BlockingStream::new(stream))
    }
}
#[test]
fn per_remote_owner_opens_once_and_drop_cancels_without_a_second_command() {
    let endpoint = Arc::new(Fake {
        calls: Mutex::new(Vec::new()),
        streams: Mutex::new(Vec::new()),
    });
    let remote = RemoteTransport::new(endpoint.clone());
    let stream = remote
        .action("ssh://git@host/repo", Service::UploadPackLs)
        .unwrap();
    assert!(
        remote
            .action("ssh://git@host/other", Service::UploadPackLs)
            .is_err()
    );
    assert_eq!(*endpoint.calls.lock().unwrap(), vec!["ssh://git@host/repo"]);
    drop(remote);
    assert!(endpoint.streams.lock().unwrap()[0].stats().terminal);
    drop(stream);
}

#[test]
fn open_failure_preserves_a_network_error_and_does_not_fall_back() {
    struct Refuse;
    impl OpenStream for Refuse {
        fn open(&self, _: &str, _: GitService) -> io::Result<BlockingStream> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fixture refused\0identity",
            ))
        }
    }
    let remote = RemoteTransport::new(Arc::new(Refuse));
    let error = match remote.action("ssh://git@host/repo", Service::ReceivePackLs) {
        Err(error) => error,
        Ok(_) => panic!("refusal opened a stream"),
    };
    assert_eq!(error.class(), git2::ErrorClass::Net);
    assert!(error.message().contains("fixture refused"));
    assert!(remote.close().is_ok());
}

/// A host key the endpoint does not trust reaches libgit2 as the native route reports one: the SSH class, with the
/// host's words (design section 14, item 13, U8). Any other permission error stays a network one.
#[test]
fn a_refused_host_key_is_an_ssh_class_failure_carrying_the_hosts_words() {
    #[derive(Debug)]
    struct Worded;
    impl std::fmt::Display for Worded {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("gwz cannot trust the host key of host.example")
        }
    }
    impl std::error::Error for Worded {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&crate::git::endpoint::ssh_host_key::HostKeyRefused::NotListed)
        }
    }
    struct Refuse;
    impl OpenStream for Refuse {
        fn open(&self, _: &str, _: GitService) -> io::Result<BlockingStream> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, Worded))
        }
    }
    let remote = RemoteTransport::new(Arc::new(Refuse));
    let error = match remote.action("ssh://git@host.example/repo", Service::UploadPackLs) {
        Err(error) => error,
        Ok(_) => panic!("a refused host key opened a stream"),
    };
    assert_eq!(error.class(), git2::ErrorClass::Ssh);
    assert_eq!(error.code(), git2::ErrorCode::GenericError);
    assert!(
        error
            .message()
            .contains("cannot trust the host key of host.example")
    );
}
