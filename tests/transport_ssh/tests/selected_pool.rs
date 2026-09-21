#![allow(dead_code, unused_imports)]
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod common;
        use common::{ssh_channel,ssh_connection};
        #[path="../../../src/git/endpoint/agent_job.rs"] mod agent_job;
        #[path="../../../src/git/endpoint/ssh_key_container.rs"] mod ssh_key_container;
        #[path="../../../src/git/endpoint/ssh_key_snapshot.rs"] mod ssh_key_snapshot;
        #[path="../../../src/git/endpoint/ssh_key_auth.rs"] mod ssh_key_auth;
        #[path="../../../src/git/endpoint/ssh_network.rs"] mod ssh_network;
        #[path="../../../src/git/endpoint/ssh_admission.rs"] mod ssh_admission;
        #[path="../../../src/git/endpoint/ssh_pool.rs"] mod ssh_pool;
        #[path="../../../src/git/endpoint/ssh_pump.rs"] mod ssh_pump;
        #[path="../../../src/git/endpoint/ssh_setup.rs"] mod ssh_setup;
        #[path="../../../src/git/endpoint/ssh_shutdown.rs"] mod ssh_shutdown;
        #[path="../../../src/git/endpoint/ssh_worker.rs"] mod ssh_worker;
        #[path="../../../src/git/endpoint/stream_io.rs"] mod stream_io;
        use std::{fs,io::{self,Read,Write},sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}},time::{Duration,Instant}};
        use gwz_transport::{pool::{Config,Key,Identity},protocol::{Opened,AuthMethod}};
        use ssh_key_snapshot::Registry;
        use ssh_worker::Endpoint;
        fn config() -> Config {Config {total:1,per_host:1,per_user_host:1,cleanup_timeout_ms:50,..Config::default()}}
        fn done(endpoint:&Endpoint) {
            let until=Instant::now()+Duration::from_secs(5);
            while !endpoint.shutdown_status().cleanup_complete {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
        }
        fn exchange(mut stream:stream_io::BlockingStream) {
            stream.write_all(b"0000").unwrap();stream.end_write().unwrap();let mut bytes=Vec::new();stream.read_to_end(&mut bytes).unwrap();assert!(!bytes.is_empty());stream.close().unwrap();
        }
        fn endpoint(f:&common::SshdFixture,r:Registry,calls:Arc<AtomicUsize>,barrier:Arc<Mutex<Option<std::sync::mpsc::Receiver<()>>>>) -> Endpoint {
            let known=f.known_hosts.clone();
            Endpoint::with_registry(config(),r,move |origin,registry| {
                ssh_setup::SetupConnector::new(origin,Duration::from_millis(50),move |key:&Key,identity:&Identity| ->io::Result<ssh_setup::Setup> {
                    let pin=registry.lookup(key,identity)?;let key=key.clone();let known=known.clone();let barrier=barrier.clone();let calls=calls.clone();
                    Ok(Box::new(move |c| {
                        calls.fetch_add(1,Ordering::SeqCst);
                        let wait=barrier.lock().unwrap().take(); if let Some(wait)=wait {wait.recv().unwrap();}
                        let (connection,host)=ssh_network::establish(&key,&known,&c)?;
                        ssh_key_auth::authenticate(connection,&host,pin,c).and_then(ssh_setup::Authenticated::selected)
                    }))
                })
            },1000).unwrap()
        }
        #[test]
        fn concurrent_initial_batch_uses_one_connection_and_keeps_idle_pin() {
            let f=common::SshdFixture::new();let r=Registry::new();let calls=Arc::new(AtomicUsize::new(0));let (release,wait)=std::sync::mpsc::channel();
            let endpoint=endpoint(&f,r.clone(),calls.clone(),Arc::new(Mutex::new(Some(wait))));
            let key=Key::ssh(&f.user,"127.0.0.1",f.port);let path=f.temp.path().join("client_ed25519");
            let threads:Vec<_>=(0..6).map(|_| {let endpoint=endpoint.clone();let key=key.clone();let path=path.clone();let repo=f.repository.clone();std::thread::spawn(move || {
                let (stream,opened)=endpoint.open_selected(key,path,ssh_channel::GitService::UploadPack,repo.to_str().unwrap()).unwrap();exchange(stream);opened
            })}).collect();
            let until=Instant::now()+Duration::from_secs(3);
            while calls.load(Ordering::SeqCst)==0 || endpoint.pending_requests()!=6 || endpoint.shutdown_status().pending_admissions!=0 {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
            assert_eq!(calls.load(Ordering::SeqCst),1);assert_eq!(r.usage().0,1);release.send(()).unwrap();
            let results:Vec<Opened>=threads.into_iter().map(|t|t.join().unwrap()).collect();
            assert!(results.iter().all(|o|o.connection_id==results[0].connection_id && o.facts.method==AuthMethod::SshKey && o.facts.authenticated==Some(true)));
            assert_eq!(results.iter().filter(|o|!o.reused).count(),1);assert_eq!(results.iter().filter(|o|o.facts.credential_offered).count(),1);
            assert_eq!(calls.load(Ordering::SeqCst),1);assert_eq!(r.usage().0,1);
            endpoint.shutdown();done(&endpoint);assert_eq!(r.usage(),(0,0));
        }
        #[test]
        fn every_reuse_rereads_current_file_and_same_bytes_alternate_path_reuses() {
            let f=common::SshdFixture::new();let r=Registry::new();let calls=Arc::new(AtomicUsize::new(0));
            let endpoint=endpoint(&f,r.clone(),calls.clone(),Arc::new(Mutex::new(None)));
            let key=Key::ssh(&f.user,"127.0.0.1",f.port);let path=f.temp.path().join("client_ed25519");let alternate=f.temp.path().join("alternate");fs::copy(&path,&alternate).unwrap();
            let open=|path|endpoint.open_selected(key.clone(),path,ssh_channel::GitService::UploadPack,f.repository.to_str().unwrap());
            let (stream,first)=open(path.clone()).unwrap();exchange(stream);
            let (stream,reused)=open(alternate.clone()).unwrap();exchange(stream);assert_eq!(first.connection_id,reused.connection_id);assert!(reused.reused);
            fs::write(&alternate,"invalid").unwrap();assert!(open(alternate.clone()).is_err());fs::remove_file(alternate.clone()).unwrap();assert!(open(alternate).is_err());assert_eq!(calls.load(Ordering::SeqCst),1);
            let (stream,reused)=open(path).unwrap();exchange(stream);assert!(reused.reused);endpoint.shutdown();done(&endpoint);assert_eq!(r.usage(),(0,0));
        }
    }
}
