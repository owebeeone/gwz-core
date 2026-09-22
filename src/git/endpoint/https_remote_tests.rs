use super::*;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct State {
    ended: usize,
    flushed: usize,
    closed: usize,
    cancelled: usize,
    data: Vec<u8>,
}
struct Fake(Arc<Mutex<State>>);
impl Read for Fake {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        assert!(self.0.lock().unwrap().ended > 0);
        if out.is_empty() {
            return Ok(0);
        }
        Ok(0)
    }
}
impl Write for Fake {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let n = data.len().min(3);
        self.0.lock().unwrap().data.extend_from_slice(&data[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flushed += 1;
        Ok(())
    }
}
impl HalfClose for Fake {
    fn end_write(&self) -> io::Result<()> {
        self.0.lock().unwrap().ended += 1;
        Ok(())
    }
    fn finish(&self) -> io::Result<()> {
        self.0.lock().unwrap().closed += 1;
        Ok(())
    }
    fn cancel(&self) {
        self.0.lock().unwrap().cancelled += 1;
    }
}
#[test]
fn rpc_first_nonempty_read_ends_body_once_not_flush_or_empty_read() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut rpc = RpcIo::new(Fake(state.clone()), false);
    rpc.write_all(b"a request larger than a partial write")
        .unwrap();
    rpc.flush().unwrap();
    assert_eq!(rpc.read(&mut []).unwrap(), 0);
    assert_eq!(state.lock().unwrap().ended, 0);
    assert_eq!(rpc.read(&mut [0; 16]).unwrap(), 0);
    assert_eq!(rpc.read(&mut [0; 16]).unwrap(), 0);
    assert_eq!(state.lock().unwrap().ended, 1);
    assert_eq!(state.lock().unwrap().closed, 1);
    assert!(rpc.write(b"late").is_err());
    drop(rpc);
    assert_eq!(state.lock().unwrap().cancelled, 0);
}
#[test]
fn rpc_advertisement_rejects_body_and_unfinished_drop_cancels() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut rpc = RpcIo::new(Fake(state.clone()), true);
    assert!(rpc.write(b"body").is_err());
    drop(rpc);
    assert_eq!(state.lock().unwrap().cancelled, 1);
}

fn open_failure(
    service: GitService,
    status: i64,
    method: gwz_transport::protocol::AuthMethod,
    authenticated: Option<bool>,
) -> git2::Error {
    let failure = crate::transport_host::HttpsOpenFailure {
        failure: gwz_transport::protocol::Failure {
            code: gwz_transport::protocol::ErrorCode::RepositoryRefused,
            effect: gwz_transport::protocol::Effect::None,
            facts: Some(gwz_transport::protocol::Facts {
                method,
                authenticated,
                http_status: Some(status),
                ..Default::default()
            }),
        },
        anonymous_status: None,
    };
    map_open_error(io::Error::new(io::ErrorKind::Other, failure), service)
}

#[test]
fn only_anonymous_or_gh_discovery_refusal_maps_to_private_repository_marker() {
    let error = open_failure(
        GitService::UploadPackAdvertisement,
        404,
        gwz_transport::protocol::AuthMethod::Gh,
        None,
    );
    assert_eq!(error.code(), git2::ErrorCode::NotFound);
    assert_eq!(error.class(), git2::ErrorClass::Http);
    assert_eq!(error.message(), REPOSITORY_REFUSED);

    let authenticated = open_failure(
        GitService::UploadPackAdvertisement,
        404,
        gwz_transport::protocol::AuthMethod::Gh,
        Some(true),
    );
    assert_eq!(authenticated.code(), git2::ErrorCode::GenericError);
    assert_ne!(authenticated.message(), REPOSITORY_REFUSED);
}

#[test]
fn exchange_refusal_never_maps_to_private_repository_marker() {
    let error = open_failure(
        GitService::ReceivePackExchange,
        403,
        gwz_transport::protocol::AuthMethod::None,
        None,
    );
    assert_eq!(error.code(), git2::ErrorCode::GenericError);
    assert_ne!(error.message(), REPOSITORY_REFUSED);
}
