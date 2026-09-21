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
        use std::{fs,io::{self,Read,Write},sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering},mpsc::{self,Receiver,Sender}},time::{Duration,Instant}};
        use gwz_transport::{pool::{Config,Key,Identity},protocol::{Opened,AuthMethod}};
        use agent_job::Job;
        use ssh_admission::Reader;
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
        fn endpoint_with_reader_config(c:Config,f:&common::SshdFixture,r:Registry,calls:Arc<AtomicUsize>,barrier:Arc<Mutex<Option<Receiver<()>>>>,reader:Reader) -> Endpoint {
            let known=f.known_hosts.clone();
            Endpoint::with_reader(c,r,reader,move |origin,registry| {
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
        fn endpoint_with_reader(f:&common::SshdFixture,r:Registry,calls:Arc<AtomicUsize>,barrier:Arc<Mutex<Option<Receiver<()>>>>,reader:Reader) -> Endpoint {
            endpoint_with_reader_config(config(),f,r,calls,barrier,reader)
        }
        fn endpoint(f:&common::SshdFixture,r:Registry,calls:Arc<AtomicUsize>,barrier:Arc<Mutex<Option<Receiver<()>>>>) -> Endpoint {
            endpoint_with_reader(f,r,calls,barrier,Arc::new(Registry::start))
        }
        fn stalled_reader(started:Sender<()>,release:Arc<Mutex<Option<Receiver<()>>>>) -> Reader {
            Arc::new(move |registry,key,path,deadline,cleanup| {
                let reservation=registry.reserve()?;
                let started=started.clone();
                let release=release.lock().unwrap().take().ok_or_else(|| io::Error::from(io::ErrorKind::Other))?;
                Ok(Job::start(deadline,cleanup,move |control| {
                    started.send(()).map_err(|_| io::ErrorKind::BrokenPipe)?;
                    release.recv().map_err(|_| io::ErrorKind::BrokenPipe)?;
                    let bytes=fs::read(path)?;
                    reservation.test_read(key,&control,move |buffer,control| {
                        control.check()?;
                        if bytes.len()>=buffer.len() { return Err(io::ErrorKind::InvalidInput.into()); }
                        buffer[..bytes.len()].copy_from_slice(&bytes);
                        Ok(bytes.len())
                    })
                })?)
            })
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
        #[test]
        fn stalled_admission_expires_without_setup_and_retains_charge_until_join() {
            let f=common::SshdFixture::new();let r=Registry::new();let calls=Arc::new(AtomicUsize::new(0));
            let (started,entered)=mpsc::channel();let (release,wait)=mpsc::channel();
            let reader=stalled_reader(started,Arc::new(Mutex::new(Some(wait))));
            let short=Config { allocation_timeout_ms:20, connect_timeout_ms:20, interaction_timeout_ms:20, ..config() };
            let endpoint=endpoint_with_reader_config(short,&f,r.clone(),calls.clone(),Arc::new(Mutex::new(None)),reader);
            let key=Key::ssh(&f.user,"127.0.0.1",f.port);let path=f.temp.path().join("client_ed25519");let endpoint2=endpoint.clone();
            let open=std::thread::spawn(move || endpoint2.open_selected(key,path,ssh_channel::GitService::UploadPack,"repo"));
            entered.recv_timeout(Duration::from_secs(3)).unwrap();
            let until=Instant::now()+Duration::from_secs(3);
            while endpoint.shutdown_status().pending_admissions!=1 { assert!(Instant::now()<until); std::thread::sleep(Duration::from_millis(1)); }
            let error=match open.join().unwrap() { Ok(_) => panic!("expired admission unexpectedly opened"), Err(error) => error };assert_eq!(error.kind(),io::ErrorKind::TimedOut);assert_eq!(calls.load(Ordering::SeqCst),0);assert_eq!(r.usage().0,1);
            endpoint.shutdown();
            let status=endpoint.shutdown_status();assert!(!status.cleanup_complete);assert_eq!(status.pending_admissions,1);
            release.send(()).unwrap();
            let until=Instant::now()+Duration::from_secs(3);
            while !endpoint.shutdown_status().cleanup_complete { assert!(Instant::now()<until); std::thread::sleep(Duration::from_millis(1)); }
            assert_eq!(endpoint.shutdown_status().pending_admissions,0);assert_eq!(r.usage(),(0,0));
        }
        #[test]
        fn stalled_admission_does_not_stop_an_existing_stream() {
            let f=common::SshdFixture::new();let r=Registry::new();let calls=Arc::new(AtomicUsize::new(0));let invocations=Arc::new(AtomicUsize::new(0));
            let (started,entered)=mpsc::channel();let (release_tx,wait)=mpsc::channel();let release=Arc::new(Mutex::new(Some(wait)));let normal=Arc::new(Registry::start);let stalled=stalled_reader(started,release);
            let reader:Reader={let invocations=invocations.clone();let normal=normal.clone();let stalled=stalled.clone();Arc::new(move |registry,key,path,deadline,cleanup| {if invocations.fetch_add(1,Ordering::SeqCst)==0 {(normal)(registry,key,path,deadline,cleanup)} else {(stalled)(registry,key,path,deadline,cleanup)}})};
            let short=Config { allocation_timeout_ms:100, connect_timeout_ms:100, interaction_timeout_ms:100, ..config() };
            let endpoint=endpoint_with_reader_config(short,&f,r,calls.clone(),Arc::new(Mutex::new(None)),reader);let key=Key::ssh(&f.user,"127.0.0.1",f.port);let path=f.temp.path().join("client_ed25519");
            let (mut stream,_)=endpoint.open_selected(key.clone(),path.clone(),ssh_channel::GitService::UploadPack,f.repository.to_str().unwrap()).unwrap();
            let endpoint2=endpoint.clone();let second=std::thread::spawn(move || endpoint2.open_selected(key,path,ssh_channel::GitService::UploadPack,"repo"));entered.recv_timeout(Duration::from_secs(3)).unwrap();
            stream.write_all(b"0000").unwrap();stream.end_write().unwrap();let mut bytes=Vec::new();stream.read_to_end(&mut bytes).unwrap();assert!(!bytes.is_empty());stream.close().unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);
            endpoint.shutdown();assert!(second.join().unwrap().is_err());release_tx.send(()).unwrap();
            let until=Instant::now()+Duration::from_secs(3);while !endpoint.shutdown_status().cleanup_complete {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(1));}
        }
    }
}
