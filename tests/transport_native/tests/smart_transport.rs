use git2::build::RepoBuilder;
use git2::transport::{Service, SmartSubtransport, SmartSubtransportStream};
use git2::{
    Error, ErrorClass, ErrorCode, FetchOptions, PushOptions, RemoteCallbacks, Repository, Signature,
};
use std::fs;
use std::io::{self, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tempfile::TempDir;

#[derive(Clone, Default)]
struct State {
    actions: Arc<Mutex<Vec<Service>>>,
    urls: Arc<Mutex<Vec<String>>>,
    writes: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl State {
    fn services(&self) -> Vec<Service> {
        self.actions.lock().unwrap().clone()
    }
}

struct Endpoint {
    state: State,
    repository: PathBuf,
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct ChildStream {
    child: Option<Child>,
    input: ChildStdin,
    output: ChildStdout,
    writes: Arc<AtomicUsize>,
}

impl Drop for ChildStream {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Read for ChildStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.output.read(bytes)
    }
}

impl Write for ChildStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.input.write(bytes)?;
        self.writes.fetch_add(written, Ordering::SeqCst);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.input.flush()
    }
}

impl SmartSubtransport for Endpoint {
    fn action(
        &self,
        url: &str,
        service: Service,
    ) -> Result<Box<dyn SmartSubtransportStream>, Error> {
        self.state.actions.lock().unwrap().push(service);
        self.state.urls.lock().unwrap().push(url.to_owned());
        let command = match service {
            Service::UploadPackLs | Service::UploadPack => "upload-pack",
            Service::ReceivePackLs | Service::ReceivePack => "receive-pack",
        };
        let mut child = Command::new("git")
            .args([command, self.repository.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| Error::from_str(&format!("fixture service: {error}")))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| Error::from_str("fixture stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| Error::from_str("fixture stdout unavailable"))?;
        Ok(Box::new(ChildStream {
            child: Some(child),
            input,
            output,
            writes: self.state.writes.clone(),
        }))
    }

    fn close(&self) -> Result<(), Error> {
        self.state.closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn endpoint(state: State, repository: &Path) -> Endpoint {
    Endpoint {
        state,
        repository: repository.to_owned(),
    }
}

fn callbacks<F>(factory: F) -> RemoteCallbacks<'static>
where
    F: FnMut(&git2::Remote<'_>) -> Result<Endpoint, Error> + 'static,
{
    let mut callbacks = RemoteCallbacks::new();
    callbacks.smart_transport(false, factory);
    callbacks
}

fn seed_repository(temp: &TempDir) -> (Repository, PathBuf) {
    let source_path = temp.path().join("source");
    let bare_path = temp.path().join("remote.git");
    let source = Repository::init(&source_path).unwrap();
    fs::write(source_path.join("README"), "fixture\n").unwrap();
    let mut index = source.index().unwrap();
    index.add_path(Path::new("README")).unwrap();
    let tree = source.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = Signature::now("fixture", "fixture@example.invalid").unwrap();
    source.set_head("refs/heads/main").unwrap();
    source
        .commit(Some("HEAD"), &signature, &signature, "fixture", &tree, &[])
        .unwrap();
    let bare = Repository::init_bare(&bare_path).unwrap();
    let mut remote = source.remote("seed", bare_path.to_str().unwrap()).unwrap();
    bare.set_head("refs/heads/main").unwrap();
    remote
        .push(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();
    drop(remote);
    drop(bare);
    drop(tree);
    (source, bare_path)
}

#[test]
fn named_anonymous_and_clone_routes_are_owned_and_networkless() {
    let temp = TempDir::new().unwrap();
    let (source, bare) = seed_repository(&temp);
    let expected = source.head().unwrap().target().unwrap();
    let named_state = State::default();
    let anonymous_state = State::default();
    let clone_state = State::default();
    let named_url = "ssh://fixture.invalid/named.git";
    let anonymous_url = "ssh://fixture.invalid/anonymous.git";

    let repo = Repository::init(temp.path().join("named")).unwrap();
    repo.remote("origin", named_url).unwrap();
    let mut named = repo.find_remote("origin").unwrap();
    let named_factory_state = named_state.clone();
    let bare_for_named = bare.clone();
    let progress_calls = Arc::new(AtomicUsize::new(0));
    let progress_counter = progress_calls.clone();
    let mut fetch = FetchOptions::new();
    let mut named_callbacks = callbacks(move |remote| {
        assert_eq!(remote.name().unwrap(), Some("origin"));
        Ok(endpoint(named_factory_state.clone(), &bare_for_named))
    });
    named_callbacks.transfer_progress(move |_progress| {
        progress_counter.fetch_add(1, Ordering::SeqCst);
        true
    });
    fetch.remote_callbacks(named_callbacks);
    named
        .fetch(&["refs/heads/main"], Some(&mut fetch), None)
        .unwrap();
    assert_eq!(repo.find_commit(expected).unwrap().id(), expected);
    assert_eq!(named_state.services(), vec![Service::UploadPackLs]);
    assert!(progress_calls.load(Ordering::SeqCst) > 0);
    drop(named);

    let anonymous_repo = Repository::init(temp.path().join("anonymous")).unwrap();
    let mut anonymous = anonymous_repo.remote_anonymous(anonymous_url).unwrap();
    let anonymous_factory_state = anonymous_state.clone();
    let bare_for_anonymous = bare.clone();
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(move |remote| {
        assert!(remote.name().unwrap().is_none());
        Ok(endpoint(
            anonymous_factory_state.clone(),
            &bare_for_anonymous,
        ))
    }));
    anonymous
        .fetch(&["refs/heads/main"], Some(&mut fetch), None)
        .unwrap();
    assert_eq!(anonymous_repo.find_commit(expected).unwrap().id(), expected);
    assert_eq!(anonymous_state.services(), vec![Service::UploadPackLs]);
    drop(anonymous);

    let clone_path = temp.path().join("clone");
    let clone_url = "ssh://fixture.invalid/clone.git";
    let clone_factory_state = clone_state.clone();
    let bare_for_clone = bare.clone();
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(move |remote| {
        assert_eq!(remote.name().unwrap(), Some("origin"));
        Ok(endpoint(clone_factory_state.clone(), &bare_for_clone))
    }));
    let cloned = RepoBuilder::new()
        .fetch_options(fetch)
        .clone(clone_url, &clone_path)
        .unwrap();
    assert_eq!(cloned.head().unwrap().target(), Some(expected));
    assert_eq!(clone_state.services(), vec![Service::UploadPackLs]);
    assert_eq!(named_state.drops.load(Ordering::SeqCst), 1);
    assert_eq!(anonymous_state.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn upload_and_receive_use_the_same_owned_endpoint_state() {
    let temp = TempDir::new().unwrap();
    let (source, bare) = seed_repository(&temp);
    let state = State::default();
    let mut remote = source
        .remote("origin", "ssh://fixture.invalid/stateful.git")
        .unwrap();
    let factory_state = state.clone();
    let bare_for_factory = bare.clone();
    let mut fetch = FetchOptions::new();
    fetch.remote_callbacks(callbacks(move |_remote| {
        Ok(endpoint(factory_state.clone(), &bare_for_factory))
    }));
    remote
        .fetch(&["refs/heads/main"], Some(&mut fetch), None)
        .unwrap();

    fs::write(temp.path().join("source").join("NEXT"), "next\n").unwrap();
    let mut index = source.index().unwrap();
    index.add_path(Path::new("NEXT")).unwrap();
    let tree = source.find_tree(index.write_tree().unwrap()).unwrap();
    let parent = source.head().unwrap().target().unwrap();
    let parent = source.find_commit(parent).unwrap();
    let signature = Signature::now("fixture", "fixture@example.invalid").unwrap();
    let expected = source
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "next",
            &tree,
            &[&parent],
        )
        .unwrap();
    let mut push = PushOptions::new();
    push.remote_callbacks(callbacks(|_remote| {
        Err(Error::new(
            ErrorCode::GenericError,
            ErrorClass::Net,
            "replacement",
        ))
    }));
    remote
        .push(&["refs/heads/main:refs/heads/main"], Some(&mut push))
        .unwrap();

    let pushed = Repository::open_bare(&bare).unwrap();
    assert_eq!(
        pushed.find_reference("refs/heads/main").unwrap().target(),
        Some(expected)
    );
    let services = state.services();
    assert!(services.contains(&Service::UploadPackLs));
    assert!(services.contains(&Service::ReceivePackLs));
    assert!(state.writes.load(Ordering::SeqCst) > 0);
    assert!(state.closes.load(Ordering::SeqCst) >= 1);
}

#[test]
fn retained_remote_does_not_replace_transport_after_disconnect() {
    let temp = TempDir::new().unwrap();
    let (_source, bare) = seed_repository(&temp);
    let repo = Repository::init(temp.path().join("retained")).unwrap();
    repo.remote("origin", "ssh://fixture.invalid/retained.git")
        .unwrap();
    let mut remote = repo.find_remote("origin").unwrap();
    let first = State::default();
    let replacement_calls = Arc::new(AtomicUsize::new(0));
    let first_factory_state = first.clone();
    let bare_for_first = bare.clone();
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(move |_remote| {
        Ok(endpoint(first_factory_state.clone(), &bare_for_first))
    }));
    remote
        .fetch(&["refs/heads/main"], Some(&mut options), None)
        .unwrap();
    drop(options);
    remote.disconnect().unwrap();
    let replacement_counter = replacement_calls.clone();
    let bare_for_second = bare.clone();
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(move |_remote| {
        replacement_counter.fetch_add(1, Ordering::SeqCst);
        Ok(endpoint(State::default(), &bare_for_second))
    }));
    remote
        .fetch(&["refs/heads/main"], Some(&mut options), None)
        .unwrap();
    assert_eq!(replacement_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        first.services(),
        vec![Service::UploadPackLs, Service::UploadPackLs]
    );
    assert_eq!(first.drops.load(Ordering::SeqCst), 0);
    drop(remote);
    assert_eq!(first.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn factory_errors_and_panics_cross_the_rust_boundary() {
    let temp = TempDir::new().unwrap();
    let (_source, bare) = seed_repository(&temp);
    let repo = Repository::init(temp.path().join("errors")).unwrap();
    repo.remote("origin", "ssh://fixture.invalid/error.git")
        .unwrap();
    let mut remote = repo.find_remote("origin").unwrap();
    let bare_for_error = bare.clone();
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(move |_remote| {
        let _ = &bare_for_error;
        Err(Error::new(
            ErrorCode::GenericError,
            ErrorClass::Net,
            "factory failure",
        ))
    }));
    let error = remote
        .fetch(&["refs/heads/main"], Some(&mut options), None)
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::GenericError);
    assert_eq!(error.class(), ErrorClass::Net);
    assert_eq!(error.message(), "factory failure");

    let mut panic_remote = repo
        .remote_anonymous("ssh://fixture.invalid/panic.git")
        .unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut options = FetchOptions::new();
        options.remote_callbacks(callbacks(|_remote| -> Result<Endpoint, Error> {
            panic!("factory panic")
        }));
        let _ = panic_remote.fetch(&["refs/heads/main"], Some(&mut options), None);
    }));
    let payload = result.expect_err("factory panic must unwind");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("factory panic")
    );
}

#[test]
fn nested_and_concurrent_factories_keep_distinct_contexts() {
    let temp = TempDir::new().unwrap();
    let (_source, bare) = seed_repository(&temp);
    let mut workers = Vec::new();
    for name in ["one", "two"] {
        let bare = bare.clone();
        workers.push(thread::spawn(move || {
            let state = State::default();
            let local = tempfile::tempdir().unwrap();
            let repo = Repository::init(local.path()).unwrap();
            let url = format!("ssh://fixture.invalid/{name}.git");
            repo.remote(name, &url).unwrap();
            let mut remote = repo.find_remote(name).unwrap();
            let state_for_factory = state.clone();
            let bare_for_factory = bare.clone();
            let mut options = FetchOptions::new();
            options.remote_callbacks(callbacks(move |remote| {
                let route = remote.name().unwrap().unwrap().to_owned();
                assert_eq!(route, name);
                let inner_root = tempfile::tempdir().unwrap();
                let inner_repo = Repository::init(inner_root.path()).unwrap();
                let mut inner = inner_repo
                    .remote("inner", "ssh://fixture.invalid/inner.git")
                    .unwrap();
                let mut inner_options = FetchOptions::new();
                inner_options.remote_callbacks(callbacks(|remote| {
                    assert_eq!(remote.name().unwrap(), Some("inner"));
                    Err(Error::from_str("inner route"))
                }));
                let error = inner
                    .fetch(&["refs/heads/main"], Some(&mut inner_options), None)
                    .unwrap_err();
                assert_eq!(error.message(), "inner route");
                assert_eq!(remote.name().unwrap(), Some(name));
                Ok(endpoint(state_for_factory.clone(), &bare_for_factory))
            }));
            remote
                .fetch(&["refs/heads/main"], Some(&mut options), None)
                .unwrap();
            assert_eq!(state.services(), vec![Service::UploadPackLs]);
            name.to_owned()
        }));
    }
    let mut routes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    routes.sort();
    assert_eq!(routes, vec!["one", "two"]);
}

#[test]
fn file_transport_remains_native_before_and_after_custom_transport() {
    let temp = TempDir::new().unwrap();
    let (_source, bare) = seed_repository(&temp);
    let file_url = gwz_transport_native_proof::file_url(&Repository::open_bare(&bare).unwrap());
    let before = temp.path().join("file-before");
    RepoBuilder::new().clone(&file_url, &before).unwrap();

    let state = State::default();
    let repo = Repository::init(temp.path().join("custom")).unwrap();
    repo.remote("origin", "ssh://fixture.invalid/custom.git")
        .unwrap();
    let mut remote = repo.find_remote("origin").unwrap();
    let state_for_factory = state.clone();
    let bare_for_factory = bare.clone();
    let mut options = FetchOptions::new();
    options.remote_callbacks(callbacks(move |_remote| {
        Ok(endpoint(state_for_factory.clone(), &bare_for_factory))
    }));
    remote
        .fetch(&["refs/heads/main"], Some(&mut options), None)
        .unwrap();
    assert_eq!(state.actions.lock().unwrap().len(), 1);

    let during = temp.path().join("file-during");
    RepoBuilder::new().clone(&file_url, &during).unwrap();
    drop(remote);
    drop(options);
    assert_eq!(state.drops.load(Ordering::SeqCst), 1);
    let after = temp.path().join("file-after");
    RepoBuilder::new().clone(&file_url, &after).unwrap();
}
