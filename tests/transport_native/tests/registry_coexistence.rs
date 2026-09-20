use git2::build::RepoBuilder;
use git2::transport::{Service, SmartSubtransport, SmartSubtransportStream, Transport, register};
use git2::{Error, ErrorClass, ErrorCode, FetchOptions, RemoteCallbacks, Repository, Signature};
use std::fs;
use std::path::Path;
use std::sync::Once;
use tempfile::TempDir;

struct Foreign;

impl SmartSubtransport for Foreign {
    fn action(
        &self,
        _url: &str,
        _service: Service,
    ) -> Result<Box<dyn SmartSubtransportStream>, Error> {
        Err(Error::new(
            ErrorCode::GenericError,
            ErrorClass::Net,
            "foreign registry",
        ))
    }

    fn close(&self) -> Result<(), Error> {
        Ok(())
    }
}

struct Bound;

impl SmartSubtransport for Bound {
    fn action(
        &self,
        _url: &str,
        _service: Service,
    ) -> Result<Box<dyn SmartSubtransportStream>, Error> {
        Err(Error::new(
            ErrorCode::GenericError,
            ErrorClass::Net,
            "per-remote binding",
        ))
    }

    fn close(&self) -> Result<(), Error> {
        Ok(())
    }
}

fn foreign_error(base: &Path, name: &str) -> Error {
    let repo = Repository::init(base.join(name)).unwrap();
    let mut remote = repo
        .remote_anonymous("foreign-proof://fixture/repo")
        .unwrap();
    remote.fetch(&["refs/heads/main"], None, None).unwrap_err()
}

fn seed_file_repository(temp: &TempDir) -> String {
    let path = temp.path().join("file-source");
    let repo = Repository::init(&path).unwrap();
    fs::write(path.join("README"), "native\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("README")).unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = Signature::now("fixture", "fixture@example.invalid").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "native", &tree, &[])
        .unwrap();
    drop(tree);
    gwz_transport_native_proof::file_url(&repo)
}

#[test]
fn foreign_registry_is_explicit_and_coexists_with_per_remote_binding() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| unsafe {
        register("foreign-proof", |remote| {
            Transport::smart(remote, true, Foreign)
        })
        .unwrap();
    });
    let temp = TempDir::new().unwrap();
    let before = foreign_error(temp.path(), "foreign-before");
    assert_eq!(before.code(), ErrorCode::GenericError);
    assert_eq!(before.class(), ErrorClass::Net);
    assert_eq!(before.message(), "foreign registry");

    let repo = Repository::init(temp.path().join("bound")).unwrap();
    let mut remote = repo
        .remote_anonymous("ssh://fixture.invalid/bound.git")
        .unwrap();
    let mut options = FetchOptions::new();
    let nested_root = temp.path().to_owned();
    let mut callbacks = RemoteCallbacks::new();
    callbacks.smart_transport(true, move |_remote| {
        let nested = foreign_error(&nested_root, "foreign-during");
        assert_eq!(nested.message(), "foreign registry");
        Ok(Bound)
    });
    options.remote_callbacks(callbacks);
    let bound = remote
        .fetch(&["refs/heads/main"], Some(&mut options), None)
        .unwrap_err();
    assert_eq!(bound.message(), "per-remote binding");
    drop(options);
    drop(remote);

    let file_url = seed_file_repository(&temp);
    let file_clone = temp.path().join("file-clone");
    RepoBuilder::new().clone(&file_url, &file_clone).unwrap();

    let after = foreign_error(temp.path(), "foreign-after");
    assert_eq!(after.message(), "foreign registry");
}
