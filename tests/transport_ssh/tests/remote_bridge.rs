#![allow(dead_code)]
#[path = "../../../src/git/endpoint/ssh_channel.rs"]
mod ssh_channel;
#[path = "../../../src/git/endpoint/ssh_connection.rs"]
mod ssh_connection;
#[path = "../../../src/git/endpoint/ssh_remote.rs"]
mod ssh_remote;
#[path = "../../../src/git/endpoint/stream_io.rs"]
mod stream_io;

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
