//! The server address's walk (plan step 2.3e; parked server design §3 "The walk, on Linux and macOS" and "The walk and
//! the connect"), on Linux and macOS: its link cap and link resolution, its order, the socket's directory, the type
//! tables and the macOS probe's decisions with injected values, what the walk holds and the connect and bind paths it
//! gives, and real automounter mounts; and Windows' checks after the grammar on a real host. A real fixture the host
//! does not have fails its row naming it ("no fixture"); those rows are `#[ignore]`d and run with `--ignored` on a host
//! that has, or as root can make, one.
cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix {
            use super::super::super::local::{
                SUN_PATH_MAX,
                address::{
                    DIR_MNTSTATUS_TRIGGER, HOST, Held, Kind, LINK_LIMIT, MNT_AUTOMOUNTED, MNT_IGNORE_OWNERSHIP, MNT_LOCAL,
                    MacAttributes, MacNext, Place, Platform, Probe, Seen, UseError, VDIR, VLNK, WalkRefusal,
                    HostWalked, RowUse, judge_opened, judge_probe, linux_refused_type, macos_refused_mount, walk_host,
                    walk_with,
                },
            };
            use std::{
                collections::HashMap,
                io::{Read, Write},
                os::unix::{
                    ffi::OsStrExt,
                    fs::{FileTypeExt, symlink},
                    net::{UnixListener, UnixStream},
                },
                path::{Path, PathBuf},
            };

            #[test]
            fn the_host_grammar_holds_the_socket_path_this_host_binds() {
                assert_eq!(HOST.socket_path_max(), Some(SUN_PATH_MAX));
            }

            /// A private directory whose own path holds no link (`/var` is a link on macOS), on a mount the walk accepts:
            /// on macOS not the temporary directory, which may be a RAM disk mounted with `MNT_IGNORE_OWNERSHIP` (the
            /// lanes' `/Volumes/gwztmp` is), but `/private/tmp` on the data volume.
            fn base() -> (tempfile::TempDir, PathBuf) {
                let dir = if HOST == Platform::Macos {
                    tempfile::tempdir_in("/private/tmp").unwrap()
                } else {
                    tempfile::tempdir().unwrap()
                };
                let path = std::fs::canonicalize(dir.path()).unwrap();
                (dir, path)
            }

            /// The host's walk of `path`, without a trigger hold: what the walk judged and holds (the carrier takes a
            /// `Walk`, made under the process's gate; `walk_entry_rows`).
            fn walk(path: &[u8]) -> Result<HostWalked, WalkRefusal> {
                walk_host(path)
            }

            fn bytes(path: &Path) -> Vec<u8> {
                path.as_os_str().as_bytes().to_vec()
            }

            /// The walk's refusal inside a failed connect or bind.
            fn refusal<T: std::fmt::Debug>(result: Result<T, UseError>) -> WalkRefusal {
                match result.unwrap_err() {
                    UseError::Refused(refusal) => refusal,
                    other => panic!("not a refusal: {other:?}"),
                }
            }

            /// Where the host's walk of `path` ends, or its refusal.
            fn place(path: &Path) -> Result<Place, WalkRefusal> {
                walk(&bytes(path)).map(|walked| walked.place().clone())
            }

            /// `count` links in a chain, `link-1` to `link-2` and so on, the last to the directory `real`.
            fn chain(dir: &Path, count: usize) -> PathBuf {
                std::fs::create_dir(dir.join("real")).unwrap();
                for index in 1..=count {
                    let target = if index == count { "real".to_owned() } else { format!("link-{}", index + 1) };
                    symlink(target, dir.join(format!("link-{index}"))).unwrap();
                }
                dir.join("link-1")
            }

            #[test]
            fn a_chain_of_eight_links_passes_and_a_ninth_refuses() {
                assert_eq!(LINK_LIMIT, 8);
                let (_guard, dir) = base();
                let start = chain(&dir, 8);
                assert_eq!(place(&start.join("ws")), Ok(Place::Ended(bytes(&dir.join("real/ws")))));
                let (_guard, dir) = base();
                let start = chain(&dir, 9);
                assert_eq!(
                    place(&start.join("ws")),
                    Err(WalkRefusal { component: bytes(&dir.join("link-9")), kind: Kind::TooManyLinks })
                );
            }

            #[test]
            fn links_inside_a_target_count_toward_the_cap() {
                let (_guard, dir) = base();
                // Four links to reach `real`, then a link in `real` whose target goes through the chain again.
                let start = chain(&dir, 4);
                symlink(dir.join("link-1").join("deeper"), dir.join("real/again")).unwrap();
                std::fs::create_dir(dir.join("real/deeper")).unwrap();
                // 4 + 1 + 4 = 9 links.
                let refusal = place(&start.join("again/ws")).unwrap_err();
                assert_eq!(refusal.kind, Kind::TooManyLinks);
            }

            #[test]
            fn a_relative_link_resolves_against_its_walked_directory() {
                let (_guard, dir) = base();
                std::fs::create_dir_all(dir.join("a/b")).unwrap();
                std::fs::create_dir(dir.join("c")).unwrap();
                symlink("../c/./", dir.join("a/up")).unwrap();
                symlink("a", dir.join("x")).unwrap();
                // `x` is read first, so `up`'s `..` is taken in `a`, not in the directory `x` sits in.
                assert_eq!(place(&dir.join("x/up/ws")), Ok(Place::Ended(bytes(&dir.join("c/ws")))));
            }

            #[test]
            fn a_dot_dot_in_a_target_is_taken_after_the_link_before_it() {
                // `L -> d/../t` with `d -> other/sub`: the kernel follows `d`, then takes `..` in `other`.
                let (_guard, dir) = base();
                std::fs::create_dir_all(dir.join("other/sub")).unwrap();
                std::fs::write(dir.join("other/t"), b"").unwrap();
                symlink("other/sub", dir.join("d")).unwrap();
                symlink("d/../t", dir.join("L")).unwrap();
                let kernel = std::fs::canonicalize(dir.join("L")).unwrap();
                assert_eq!(kernel, dir.join("other/t"));
                assert_eq!(place(&dir.join("L")), Ok(Place::Reached(bytes(&kernel))));
            }

            #[test]
            fn the_socket_is_reached_and_a_missing_component_ends_the_walk() {
                let (_guard, dir) = base();
                let socket = dir.join("ws-0123456789abcdef");
                let _listener = UnixListener::bind(&socket).unwrap();
                let walked = walk(&bytes(&socket)).unwrap();
                assert_eq!(*walked.place(), Place::Reached(bytes(&socket)));
                assert!(walked.last() && walked.name() == b"ws-0123456789abcdef");
                assert!(matches!(walked.held(), Some(Held::Other(_))), "{walked:?}");
                let walked = walk(&bytes(&dir.join("gone/deeper/ws"))).unwrap();
                assert_eq!(*walked.place(), Place::Ended(bytes(&dir.join("gone"))));
                assert!(!walked.last() && walked.name() == b"gone" && walked.held().is_none());
            }

            #[test]
            fn a_name_below_a_socket_or_a_file_refuses_like_the_kernels_enotdir() {
                // Code N1 / State N-1: the kernel's `connect("<socket>/ws")` is `ENOTDIR`; the walk must not end holding
                // the socket as if the address named it.
                let (_guard, dir) = base();
                let socket = dir.join("sock");
                let _listener = UnixListener::bind(&socket).unwrap();
                std::fs::write(dir.join("file"), b"").unwrap();
                for below in ["sock/ws", "file/ws"] {
                    let address = dir.join(below);
                    let kernel = UnixStream::connect(&address).unwrap_err();
                    assert_eq!(kernel.raw_os_error(), Some(libc::ENOTDIR), "{below}");
                    let component = bytes(address.parent().unwrap());
                    assert_eq!(
                        walk(&bytes(&address)).map(|walked| walked.place().clone()),
                        Err(WalkRefusal { component, kind: Kind::NotDirectory }),
                        "{below}"
                    );
                }
            }

            #[test]
            fn a_link_target_that_goes_on_below_a_socket_refuses_like_the_kernel() {
                // `sock/.`, `sock/..` and `sock/` all fail with `ENOTDIR` in the kernel; the walk read the trailing `/`
                // as nothing and ended holding the socket.
                for target in ["sock/.", "sock/..", "sock/", "sock//", "sock/./", "sock/../sock"] {
                    let mut fake = Fake::new(vec![
                        ("/w", Entry::Dir),
                        ("/w/sock", Entry::Socket),
                        ("/w/L", Entry::Link(target)),
                    ]);
                    assert_eq!(
                        walk_with(b"/w/L", &mut fake).map(|walked| walked.place().clone()),
                        Err(WalkRefusal { component: b"/w/sock".to_vec(), kind: Kind::NotDirectory }),
                        "{target}"
                    );
                    // The same link in the middle of an address.
                    let mut fake = Fake::new(vec![
                        ("/w", Entry::Dir),
                        ("/w/sock", Entry::Socket),
                        ("/w/L", Entry::Link(target)),
                    ]);
                    assert_eq!(
                        walk_with(b"/w/L/ws", &mut fake).map(|walked| walked.place().clone()),
                        Err(WalkRefusal { component: b"/w/sock".to_vec(), kind: Kind::NotDirectory }),
                        "{target}/ws"
                    );
                }
                // A directory with a trailing `/` is a directory: it names no socket.
                let mut fake = Fake::new(vec![("/w", Entry::Dir), ("/w/d", Entry::Dir), ("/w/L", Entry::Link("d/"))]);
                assert_eq!(walk_with(b"/w/L", &mut fake).unwrap_err().kind, Kind::Directory);
                let mut fake = Fake::new(vec![("/w", Entry::Dir), ("/w/d", Entry::Dir), ("/w/L", Entry::Link("d/"))]);
                assert_eq!(fake_place(b"/w/L/ws", &mut fake), Ok(Place::Ended(b"/w/d/ws".to_vec())));
            }

            #[test]
            fn an_empty_link_target_names_nothing_and_never_a_sibling() {
                // State N-2: the kernel's lookup of `E/f` where `E -> ""` is `ENOENT`; the walk went on as if `E` were
                // `.` and reached `/w/f`.
                let entries = || vec![("/w", Entry::Dir), ("/w/E", Entry::Link("")), ("/w/f", Entry::Socket)];
                let mut fake = Fake::new(entries());
                let walked = walk_with(b"/w/E/f", &mut fake).unwrap();
                assert_eq!(*walked.place(), Place::Ended(b"/w/E".to_vec()));
                assert!(!walked.last() && walked.held().is_none(), "{walked:?}");
                assert_eq!(fake.looked, ["/w", "/w/E"]);
                let mut fake = Fake::new(entries());
                let walked = walk_with(b"/w/E", &mut fake).unwrap();
                assert_eq!(*walked.place(), Place::Ended(b"/w/E".to_vec()));
                assert!(walked.held().is_none(), "{walked:?}");
            }

            #[test]
            fn a_missing_component_before_the_last_holds_no_socket() {
                let mut fake = Fake::new(vec![("/w", Entry::Dir)]);
                let walked = walk_with(b"/w/gone/deeper/ws", &mut fake).unwrap();
                assert_eq!(*walked.place(), Place::Ended(b"/w/gone".to_vec()));
                assert!(!walked.last() && walked.held().is_none(), "{walked:?}");
                assert_eq!(fake.looked, ["/w", "/w/gone"]);
            }

            #[test]
            fn a_link_as_the_last_component_and_one_above_it_are_judged_alike() {
                // Both reach the same socket directory; the temporary directory the links sit in may be on a mount that
                // ignores ownership (the lanes' RAM disk is), which must not refuse the socket's own directory.
                let (_guard, dir) = base();
                std::fs::create_dir(dir.join("real")).unwrap();
                let links = tempfile::tempdir().unwrap();
                symlink(dir.join("real/ws"), links.path().join("ws")).unwrap();
                symlink(dir.join("real"), links.path().join("d")).unwrap();
                let expected = Ok(Place::Ended(bytes(&dir.join("real/ws"))));
                assert_eq!(place(&links.path().join("ws")), expected);
                assert_eq!(place(&links.path().join("d/ws")), expected);
            }

            #[test]
            fn connecting_through_the_walk_reaches_the_walked_listener() {
                let (_guard, dir) = base();
                let socket = dir.join("ws");
                let listener = UnixListener::bind(&socket).unwrap();
                let walked = walk(&bytes(&socket)).unwrap();
                let mut client = walked.connect().unwrap();
                client.write_all(b"x").unwrap();
                let (mut served, _) = listener.accept().unwrap();
                let mut byte = [0u8; 1];
                served.read_exact(&mut byte).unwrap();
                assert_eq!(&byte, b"x");
            }

            #[test]
            fn a_connect_or_bind_needs_what_the_walk_held() {
                let (_guard, dir) = base();
                let other = UnixListener::bind(dir.join("other")).unwrap();
                other.set_nonblocking(true).unwrap();
                std::fs::create_dir(dir.join("sub")).unwrap();
                // Code N1 / State N-1: a missing component before the last gives neither a connect nor a bind.
                let early = walk(&bytes(&dir.join("gone/ws"))).unwrap();
                assert_eq!(refusal(early.connect()).kind, Kind::Unwalked);
                assert_eq!(refusal(early.bind()).kind, Kind::Unwalked);
                // A directory reached as the last component is no socket.
                let walked = walk(&bytes(&dir.join("sub"))).unwrap();
                assert_eq!(refusal(walked.connect()).kind, Kind::Directory);
                // A missing last name: nothing answers there, and a link planted there after the walk is not followed.
                let walked = walk(&bytes(&dir.join("ws"))).unwrap();
                symlink(dir.join("other"), dir.join("ws")).unwrap();
                assert_eq!(refusal(walked.connect()).kind, Kind::NoSocket);
                assert_eq!(other.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
            }

            #[test]
            fn a_bind_through_the_walk_makes_the_socket_in_the_walked_directory() {
                let (_guard, dir) = base();
                let socket = dir.join("ws");
                let walked = walk(&bytes(&socket)).unwrap();
                let listener = walked.bind().unwrap();
                assert!(std::fs::symlink_metadata(&socket).unwrap().file_type().is_socket());
                // The listener is the walked one: a connect through a new walk reaches it.
                let _client = walk(&bytes(&socket)).unwrap().connect().unwrap();
                listener.accept().unwrap();
            }

            /// What the injected file system holds at a path.
            enum Entry {
                Dir,
                /// A socket (or any file that is not a directory or a link).
                Socket,
                Link(&'static str),
                Refuse(Kind),
            }

            /// A file system of `entries`, recording what was looked up.
            struct Fake {
                entries: HashMap<&'static str, Entry>,
                looked: Vec<String>,
                refuse_socket_directory: Option<&'static str>,
            }

            impl Fake {
                fn new(entries: Vec<(&'static str, Entry)>) -> Self {
                    Fake { entries: entries.into_iter().collect(), looked: Vec::new(), refuse_socket_directory: None }
                }
            }

            impl Probe for Fake {
                type Dir = String;
                type Leaf = ();

                fn root(&mut self) -> Result<String, Kind> {
                    Ok(String::new())
                }

                fn look(&mut self, dir: &String, name: &[u8]) -> Result<Seen<String, ()>, Kind> {
                    let path = format!("{dir}/{}", String::from_utf8_lossy(name));
                    self.looked.push(path.clone());
                    match self.entries.get(path.as_str()) {
                        None => Ok(Seen::Missing),
                        Some(Entry::Dir) => Ok(Seen::Directory(path)),
                        Some(Entry::Socket) => Ok(Seen::Other(())),
                        Some(Entry::Link(target)) => Ok(Seen::Link(target.as_bytes().to_vec())),
                        Some(Entry::Refuse(kind)) => Err(kind.clone()),
                    }
                }

                fn socket_directory(&mut self, dir: &String) -> Result<(), Kind> {
                    if self.refuse_socket_directory == Some(dir.as_str()) {
                        return Err(Kind::IgnoresOwnership);
                    }
                    Ok(())
                }
            }

            fn fake_place(path: &[u8], fake: &mut Fake) -> Result<Place, WalkRefusal> {
                walk_with(path, fake).map(|walked| walked.place().clone())
            }

            #[test]
            fn a_refused_component_is_named_and_nothing_below_it_is_looked_up() {
                let mut fake = Fake::new(vec![
                    ("/net", Entry::Dir),
                    ("/net/auto", Entry::Refuse(Kind::FileSystem("autofs (0x187)".to_owned()))),
                ]);
                let refusal = walk_with(b"/net/auto/host/ws", &mut fake).unwrap_err();
                assert_eq!(refusal.component, b"/net/auto");
                assert_eq!(refusal.code(), 83);
                let text = refusal.to_string();
                assert!(text.contains("\"/net/auto\" is on a file system of type autofs (0x187)"), "{text}");
                assert_eq!(fake.looked, ["/net", "/net/auto"]);
            }

            #[test]
            fn a_link_is_walked_again_from_the_root_and_its_directory_is_the_sockets() {
                let entries = || {
                    vec![("/var", Entry::Link("private/var")), ("/private", Entry::Dir), ("/private/var", Entry::Dir)]
                };
                let mut fake = Fake::new(entries());
                assert_eq!(fake_place(b"/var/ws", &mut fake), Ok(Place::Ended(b"/private/var/ws".to_vec())));
                assert_eq!(fake.looked, ["/var", "/private", "/private/var", "/private/var/ws"]);
                let mut fake = Fake::new(entries());
                fake.refuse_socket_directory = Some("/private/var");
                assert_eq!(
                    fake_place(b"/var/ws", &mut fake),
                    Err(WalkRefusal { component: b"/private/var".to_vec(), kind: Kind::IgnoresOwnership })
                );
            }

            #[test]
            fn the_socket_directory_is_judged_where_a_last_link_lands() {
                // `/bad` refuses as a socket's directory; its `ws` is a link into `/good`, which does not.
                let entries = || vec![("/bad", Entry::Dir), ("/bad/ws", Entry::Link("/good/ws")), ("/good", Entry::Dir)];
                let mut fake = Fake::new(entries());
                fake.refuse_socket_directory = Some("/bad");
                assert_eq!(fake_place(b"/bad/ws", &mut fake), Ok(Place::Ended(b"/good/ws".to_vec())));
                assert_eq!(fake.looked, ["/bad", "/bad/ws", "/good", "/good/ws"]);
                let mut fake = Fake::new(entries());
                fake.refuse_socket_directory = Some("/bad");
                assert_eq!(
                    fake_place(b"/bad/other", &mut fake),
                    Err(WalkRefusal { component: b"/bad".to_vec(), kind: Kind::IgnoresOwnership })
                );
            }

            #[test]
            fn the_components_looked_up_are_the_kernels_for_a_dot_dot_after_a_link() {
                let mut fake = Fake::new(vec![
                    ("/w", Entry::Dir),
                    ("/w/L", Entry::Link("d/../t")),
                    ("/w/d", Entry::Link("/x/sub")),
                    ("/x", Entry::Dir),
                    ("/x/sub", Entry::Dir),
                ]);
                assert_eq!(fake_place(b"/w/L", &mut fake), Ok(Place::Ended(b"/x/t".to_vec())));
                assert_eq!(fake.looked, ["/w", "/w/L", "/w/d", "/x", "/x/sub", "/x/t"]);
            }

            #[test]
            fn a_path_that_resolves_to_a_directory_by_dot_dot_names_no_socket() {
                for target in ["..", "/", "sub/.."] {
                    let mut fake = Fake::new(vec![("/w", Entry::Dir), ("/w/L", Entry::Link(target)), ("/w/sub", Entry::Dir)]);
                    let refusal = walk_with(b"/w/L", &mut fake).unwrap_err();
                    assert_eq!(refusal.kind, Kind::Directory, "{target}");
                }
            }

            #[test]
            fn an_unreadable_component_reads_its_errno_in_words() {
                let text = |errno| Kind::Unreadable { call: "openat", errno }.to_string();
                assert_eq!(text(Some(13)), "not readable (openat failed: errno 13)");
                assert_eq!(text(None), "not readable (openat failed)");
            }

            #[test]
            fn the_linux_type_table_refuses_every_listed_type() {
                for (magic, name) in [
                    (0x0187, "autofs"), (0x6969, "nfs"), (0x517B, "smbfs"), (0xFF53_4D42, "cifs"),
                    (0xFE53_4D42, "smb2"), (0x5346_414F, "openafs"), (0x6B41_4653, "kafs"), (0x00C3_6400, "ceph"),
                    (0x0102_1997, "9p"), (0x7375_7245, "coda"), (0x564C, "ncp"), (0x7461_636F, "ocfs2"),
                    (0x0116_1970, "gfs2"), (0x2003_0528, "orangefs"), (0x786F_4256, "vboxsf"), (0x6573_5546, "fuse"),
                    (0x0BD0_0BD0, "lustre"), (0x4750_4653, "gpfs"), (0x1983_0326, "beegfs"), (0xAAD7_AAEA, "panfs"),
                    (0x0131_11A8, "ibrix"), (0xBEEF_DEAD, "stornext"), (0x6163_6673, "acfs"), (0x7C7C_6673, "prl_fs"),
                    (0xBACB_ACBC, "vmhgfs"),
                ] {
                    assert_eq!(linux_refused_type(magic), Some(name), "0x{magic:x}");
                }
                // ext4, tmpfs, btrfs, xfs, overlayfs, procfs, sysfs: local.
                for magic in [0xEF53, 0x0102_1994, 0x9123_683E, 0x5846_5342, 0x794C_7630, 0x9FA0, 0x6265_6572] {
                    assert_eq!(linux_refused_type(magic), None, "0x{magic:x}");
                }
            }

            #[test]
            fn the_macos_mount_table_refuses_by_flag_and_by_name() {
                const LOCAL: u32 = MNT_LOCAL;
                const AUTOMOUNTED: u32 = MNT_AUTOMOUNTED;
                assert_eq!(macos_refused_mount(LOCAL | MNT_IGNORE_OWNERSHIP, b"apfs"), None);
                assert_eq!(macos_refused_mount(LOCAL, b"apfs"), None);
                assert_eq!(macos_refused_mount(0, b"apfs"), Some(Kind::NotLocal));
                assert_eq!(macos_refused_mount(LOCAL | AUTOMOUNTED, b"apfs"), Some(Kind::Automounted));
                for name in [
                    "autofs", "nfs", "smbfs", "afpfs", "webdav", "ftp", "acfs", "virtiofs", "fusefs", "osxfusefs",
                    "osxfuse", "macfuse", "macfuse_sshfs", "osxfuse_x",
                ] {
                    // macFUSE's `-o local` sets MNT_LOCAL: the name refuses all the same.
                    assert_eq!(
                        macos_refused_mount(LOCAL, name.as_bytes()),
                        Some(Kind::FileSystem(name.to_owned())),
                        "{name}"
                    );
                }
                assert_eq!(macos_refused_mount(LOCAL, b"macfusex"), None);
            }

            /// A component as macOS's probe saw it.
            fn seen(objtype: u32, mount_status: Option<u32>) -> MacAttributes {
                MacAttributes { fsid: [1, 0, 0, 1, 2, 0, 0, 0], fileid: 42, objtype, mount_status }
            }

            #[test]
            fn a_macos_trigger_is_refused_before_anything_is_opened() {
                assert_eq!(judge_probe(&seen(VDIR, Some(DIR_MNTSTATUS_TRIGGER))), Err(Kind::Trigger));
                // A direct-map trigger is also a mount point (`DIR_MNTSTATUS_MNTPOINT`, 1).
                assert_eq!(judge_probe(&seen(VDIR, Some(DIR_MNTSTATUS_TRIGGER | 1))), Err(Kind::Trigger));
                assert_eq!(judge_probe(&seen(VDIR, Some(0))), Ok(MacNext::Open));
                assert_eq!(judge_probe(&seen(VDIR, Some(1))), Ok(MacNext::Open));
                assert_eq!(judge_probe(&seen(VLNK, None)), Ok(MacNext::ReadLink));
                // A regular file (1) and a socket (VSOCK, 6) are neither read nor opened.
                assert_eq!(judge_probe(&seen(1, None)), Ok(MacNext::Other));
                assert_eq!(judge_probe(&seen(6, None)), Ok(MacNext::Other));
            }

            #[test]
            fn a_macos_directory_without_its_mount_status_is_refused() {
                assert_eq!(
                    judge_probe(&seen(VDIR, None)),
                    Err(Kind::Unreadable { call: "getattrlistat", errno: None })
                );
            }

            #[test]
            fn the_macos_directory_opened_must_be_the_one_the_probe_saw() {
                let probed = seen(VDIR, Some(0));
                assert_eq!(judge_opened(&probed, (probed.fsid, probed.fileid), MNT_LOCAL, b"apfs"), Ok(()));
                let mut other_volume = probed.fsid;
                other_volume[0] ^= 1;
                assert_eq!(judge_opened(&probed, (other_volume, probed.fileid), MNT_LOCAL, b"apfs"), Err(Kind::Changed));
                assert_eq!(judge_opened(&probed, (probed.fsid, 43), MNT_LOCAL, b"apfs"), Err(Kind::Changed));
                // The same directory on a refused mount.
                assert_eq!(judge_opened(&probed, (probed.fsid, probed.fileid), 0, b"apfs"), Err(Kind::NotLocal));
            }

            cfg_if::cfg_if! {
                if #[cfg(target_vendor = "apple")] {
                    use super::super::super::local::{address::NoTrigger, row_gate};

                    #[test]
                    fn the_mount_flags_are_the_systems() {
                        assert_eq!(
                            (MNT_LOCAL, MNT_IGNORE_OWNERSHIP, MNT_AUTOMOUNTED),
                            (libc::MNT_LOCAL as u32, libc::MNT_IGNORE_OWNERSHIP as u32, libc::MNT_AUTOMOUNTED as u32)
                        );
                    }

                    #[test]
                    fn auto_home_is_refused_as_an_automounter_mount() {
                        let refused = WalkRefusal {
                            component: b"/System/Volumes/Data/home".to_vec(),
                            kind: Kind::FileSystem("autofs".to_owned()),
                        };
                        assert_eq!(walk(b"/System/Volumes/Data/home/gwz/ws").map(|walked| walked.place().clone()), Err(refused.clone()));
                        // `/home` is a link to it, read and walked again.
                        assert_eq!(walk(b"/home/gwz/ws").map(|walked| walked.place().clone()), Err(refused));
                    }

                    #[test]
                    fn a_dot_dot_after_a_link_into_auto_home_is_refused_there() {
                        // `L -> d/../gwz.sock` with `d -> /home/<name>`: the kernel asks automountd for `<name>` before it
                        // takes the `..`, so the walk must refuse at auto_home rather than cancel `d/..`.
                        let (_guard, dir) = base();
                        symlink("/home/gwz-address-row-nobody", dir.join("d")).unwrap();
                        symlink("d/../gwz.sock", dir.join("L")).unwrap();
                        assert_eq!(
                            place(&dir.join("L")),
                            Err(WalkRefusal {
                                component: b"/System/Volumes/Data/home".to_vec(),
                                kind: Kind::FileSystem("autofs".to_owned()),
                            })
                        );
                    }

                    #[test]
                    fn the_system_per_user_directory_walks_through_its_links() {
                        // `/var` is a link to `private/var`; the per-user directory is below it.
                        assert_eq!(place(Path::new("/var/folders")), Ok(Place::Reached(b"/private/var/folders".to_vec())));
                    }

                    #[test]
                    fn the_connect_path_fits_sun_path_where_the_walked_place_does_not() {
                        // A 103-byte address through `/var`, which the walk reads as `/private/var`: 111 bytes.
                        let dir = tempfile::tempdir_in("/var/tmp").unwrap();
                        let name = dir.path().file_name().unwrap().to_str().unwrap().to_owned();
                        let prefix = format!("/var/tmp/{name}/");
                        let address = format!("{prefix}{}", "s".repeat(SUN_PATH_MAX - prefix.len()));
                        assert_eq!(address.len(), SUN_PATH_MAX);
                        let listener = UnixListener::bind(&address).unwrap();
                        let walked = walk(address.as_bytes()).unwrap();
                        let Place::Reached(resolved) = walked.place() else { panic!("{walked:?}") };
                        assert_eq!(resolved.len(), SUN_PATH_MAX + "/private".len());
                        // The connect goes by the address (103 bytes), not by the walked place (111).
                        let _client = walked.connect().unwrap();
                        listener.accept().unwrap();
                        // The host binds the address too, once the socket's directory is walked.
                        let missing = format!("{prefix}{}", "t".repeat(SUN_PATH_MAX - prefix.len()));
                        let _bound = walk(missing.as_bytes()).unwrap().bind().unwrap();
                        assert!(std::fs::symlink_metadata(&missing).unwrap().file_type().is_socket());
                    }

                    #[test]
                    fn a_bind_path_over_sun_path_refuses_naming_both_lengths() {
                        // The walk takes any path (the grammar's limit is the caller's); the bind goes by the address, so
                        // one over `sun_path` is refused with its length and the limit, and nothing is created.
                        let (_guard, dir) = base();
                        let address = dir.join("n".repeat(SUN_PATH_MAX + 7 - dir.as_os_str().len()));
                        assert_eq!(address.as_os_str().len(), SUN_PATH_MAX + 8);
                        let walked = walk(&bytes(&address)).unwrap();
                        let refused = refusal(walked.bind());
                        assert_eq!(refused.kind, Kind::TooLong { length: SUN_PATH_MAX + 8, limit: SUN_PATH_MAX });
                        assert!(refused.to_string().contains("a path of 111 bytes, and sun_path holds 103"), "{refused}");
                        assert!(!address.exists());
                    }

                    #[test]
                    fn no_trigger_holds_the_device_or_the_process_policy() {
                        let gate = row_gate();
                        let held = gate.hold().unwrap();
                        assert!(held.holds_device(), "{held:?}");
                        drop(held);
                        // Where the device cannot be opened, the process policy is set off and restored on drop.
                        let before = NoTrigger::policy_is_off();
                        let held = gate.hold_with(c"/dev/gwz-address-row-no-such-device").unwrap();
                        assert!(!held.holds_device());
                        assert!(NoTrigger::policy_is_off());
                        drop(held);
                        assert_eq!(NoTrigger::policy_is_off(), before);
                        // Code N2: holders that overlap in either drop order leave the policy off until the last one
                        // is dropped, and then as it was. (One row: the policy is the process's, so two rows would race.)
                        for first_dropped_first in [true, false] {
                            let first = gate.hold_with(c"/dev/gwz-address-row-no-such-device").unwrap();
                            let second = gate.hold_with(c"/dev/gwz-address-row-no-such-device").unwrap();
                            if first_dropped_first {
                                drop(first);
                                assert!(NoTrigger::policy_is_off(), "the second holder lost the policy");
                                drop(second);
                            } else {
                                drop(second);
                                assert!(NoTrigger::policy_is_off(), "the first holder lost the policy");
                                drop(first);
                            }
                            assert_eq!(NoTrigger::policy_is_off(), before, "first dropped first: {first_dropped_first}");
                        }
                        // Code F1 / State F2 (2.3e2 round 1; State S3-2): two gates of one process that overlap leave
                        // the policy as found, in either drop order: the second finds it off and takes no restore duty.
                        for first_gate_dropped_first in [true, false] {
                            let (one, two) = (row_gate(), row_gate());
                            let first = one.hold_with(c"/dev/gwz-address-row-no-such-device").unwrap();
                            let second = two.hold_with(c"/dev/gwz-address-row-no-such-device").unwrap();
                            assert!(NoTrigger::policy_is_off());
                            if first_gate_dropped_first {
                                drop(first);
                                drop(second);
                            } else {
                                drop(second);
                                drop(first);
                            }
                            assert_eq!(NoTrigger::policy_is_off(), before, "first gate dropped first: {first_gate_dropped_first}");
                        }
                    }
                } else if #[cfg(any(target_os = "linux", target_os = "android"))] {
                    mod linux_paths {
                        //! The connect and bind paths through what the walk holds: a component swapped after the walk
                        //! does not move them (design §3 "The walk and the connect", Linux).
                        use super::super::super::super::local::address::{RowProc, RowUse};
                        use super::{Kind, Place, UseError, base, bytes, refusal, walk};
                        use std::os::unix::{
                            fs::{FileTypeExt, symlink},
                            net::UnixListener,
                        };

                        #[test]
                        fn a_connect_through_the_walk_reaches_the_walked_listener_after_a_swap() {
                            let (_guard, dir) = base();
                            std::fs::create_dir(dir.join("a")).unwrap();
                            std::fs::create_dir(dir.join("b")).unwrap();
                            let walked_listener = UnixListener::bind(dir.join("a/ws")).unwrap();
                            let other = UnixListener::bind(dir.join("b/ws")).unwrap();
                            other.set_nonblocking(true).unwrap();
                            let walked = walk(&bytes(&dir.join("a/ws"))).unwrap();
                            std::fs::rename(dir.join("a"), dir.join("a-walked")).unwrap();
                            symlink(dir.join("b"), dir.join("a")).unwrap();
                            let _client = walked.connect().unwrap();
                            walked_listener.accept().unwrap();
                            assert_eq!(other.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
                        }

                        #[test]
                        fn a_bind_through_the_walk_lands_in_the_walked_directory_after_a_swap() {
                            let (_guard, dir) = base();
                            std::fs::create_dir(dir.join("a")).unwrap();
                            std::fs::create_dir(dir.join("b")).unwrap();
                            let walked = walk(&bytes(&dir.join("a/ws"))).unwrap();
                            assert_eq!(*walked.place(), Place::Ended(bytes(&dir.join("a/ws"))));
                            std::fs::rename(dir.join("a"), dir.join("a-walked")).unwrap();
                            symlink(dir.join("b"), dir.join("a")).unwrap();
                            let _listener = walked.bind().unwrap();
                            assert!(std::fs::symlink_metadata(dir.join("a-walked/ws")).unwrap().file_type().is_socket());
                            assert!(!dir.join("b/ws").exists());
                        }

                        #[test]
                        fn without_procfs_nothing_connects_or_binds() {
                            let (_guard, dir) = base();
                            let _listener = UnixListener::bind(dir.join("ws")).unwrap();
                            let walked = walk(&bytes(&dir.join("ws"))).unwrap();
                            let fake = tempfile::tempdir().unwrap();
                            let proc = bytes(fake.path());
                            assert_eq!(refusal(walked.connect_in(&proc)).kind, Kind::NoProc);
                            // A directory tree that only looks like procfs is not procfs.
                            std::fs::create_dir_all(fake.path().join("self/fd")).unwrap();
                            assert_eq!(refusal(walked.connect_in(&proc)).kind, Kind::NoProc);
                            assert_eq!(refusal(walked.bind_in(&proc)).kind, Kind::NoProc);
                        }

                        #[test]
                        fn a_name_the_descriptor_route_cannot_hold_refuses_at_bind_with_its_lengths() {
                            // Code N5: `/proc/self/fd/<n>/` costs 15 bytes and the digits of `n`, so a last component of
                            // 100 bytes in `/tmp` (105 bytes, inside the grammar's 107) does not fit `sun_path` there.
                            let address = format!("/tmp/{}", "n".repeat(100));
                            assert!(address.len() <= 107);
                            let walked = walk(address.as_bytes()).unwrap();
                            let UseError::Refused(refused) = walked.bind().unwrap_err() else { panic!("not a refusal") };
                            assert!(matches!(refused.kind, Kind::TooLong { limit: 107, .. }), "{refused}");
                            assert!(!std::path::Path::new(&address).exists());
                        }
                    }

                    mod linux {
                        //! Real mounts. A host has an automounter's or a network file system's mount only by its own
                        //! configuration, so these rows are `#[ignore]`d and run with `--ignored`; each searches the
                        //! host's mounts for a fixture (autofs as root is made with `mount(2)` and a pipe, which needs
                        //! no daemon) and fails naming the missing one when there is none.
                        use super::{Kind, base, bytes, walk};
                        use std::{
                            ffi::CString,
                            io,
                            os::{
                                fd::{AsRawFd, FromRawFd, OwnedFd},
                                unix::ffi::OsStrExt,
                            },
                            path::{Path, PathBuf},
                        };

                        /// `/proc/self/mountinfo`'s escapes (`\040` and the like) undone.
                        fn unescape(field: &str) -> String {
                            let mut out = Vec::new();
                            let raw = field.as_bytes();
                            let mut index = 0;
                            while index < raw.len() {
                                let octal = raw.get(index + 1..index + 4).and_then(|digits| {
                                    std::str::from_utf8(digits).ok().and_then(|text| u8::from_str_radix(text, 8).ok())
                                });
                                match (raw[index], octal) {
                                    (b'\\', Some(byte)) => {
                                        out.push(byte);
                                        index += 4;
                                    }
                                    (byte, _) => {
                                        out.push(byte);
                                        index += 1;
                                    }
                                }
                            }
                            String::from_utf8_lossy(&out).into_owned()
                        }

                        /// The first mount whose type `wanted` takes and over which nothing is mounted.
                        fn fixture(wanted: fn(&str) -> bool) -> Option<PathBuf> {
                            let text = std::fs::read_to_string("/proc/self/mountinfo").expect("/proc/self/mountinfo");
                            let mounts: Vec<(PathBuf, String)> = text
                                .lines()
                                .filter_map(|line| {
                                    let fields: Vec<&str> = line.split(' ').collect();
                                    let dash = fields.iter().position(|field| *field == "-")?;
                                    Some((PathBuf::from(unescape(fields.get(4)?)), (*fields.get(dash + 1)?).to_owned()))
                                })
                                .collect();
                            mounts.iter().enumerate().find_map(|(index, (point, kind))| {
                                let covered = mounts[index + 1..].iter().any(|(other, _)| other == point);
                                (wanted(kind) && !covered).then(|| point.clone())
                            })
                        }

                        /// The walk of a name inside `point` refuses at `point` or above it, as a mount kind.
                        fn refused_at(point: &Path) {
                            let refusal = walk(&bytes(&point.join("gwz-address-row")))
                                .expect_err("a walk into the mount was not refused");
                            assert!(
                                bytes(point).starts_with(&refusal.component),
                                "refused at {refusal}, not at or above {}",
                                point.display()
                            );
                            assert!(matches!(refusal.kind, Kind::FileSystem(_) | Kind::Automount), "{refusal}");
                            eprintln!("address_rows: fixture {}: {refusal}", point.display());
                        }

                        fn no_fixture(what: &str, why: &str) -> ! {
                            panic!("no fixture: this host has no {what} mount and none can be made here ({why})")
                        }

                        /// An autofs mount of this process's own, unmounted when dropped.
                        struct Autofs {
                            point: CString,
                            _pipe: [OwnedFd; 2],
                        }

                        impl Drop for Autofs {
                            fn drop(&mut self) {
                                // SAFETY: `point` is NUL-terminated.
                                unsafe { libc::umount2(self.point.as_ptr(), libc::MNT_DETACH) };
                            }
                        }

                        fn make_autofs(point: &Path) -> Result<Autofs, String> {
                            // SAFETY: no preconditions.
                            if unsafe { libc::geteuid() } != 0 {
                                return Err("mounting autofs needs root".to_owned());
                            }
                            let mut ends = [0 as libc::c_int; 2];
                            // SAFETY: `ends` is valid for two descriptors.
                            if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
                                return Err(format!("pipe2: {}", io::Error::last_os_error()));
                            }
                            // SAFETY: both descriptors were just made and are owned by no one else.
                            let (read, write) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
                            // SAFETY: no preconditions.
                            let group = unsafe { libc::getpgrp() };
                            let options = CString::new(format!(
                                "fd={},pgrp={group},minproto=5,maxproto=5,indirect",
                                write.as_raw_fd()
                            ))
                            .unwrap();
                            let target = CString::new(point.as_os_str().as_bytes()).unwrap();
                            // SAFETY: every pointer is a NUL-terminated string that outlives the call.
                            let result = unsafe {
                                libc::mount(c"gwz-address-row".as_ptr(), target.as_ptr(), c"autofs".as_ptr(), 0,
                                    options.as_ptr().cast())
                            };
                            if result != 0 {
                                return Err(format!("mount(autofs): {}", io::Error::last_os_error()));
                            }
                            Ok(Autofs { point: target, _pipe: [read, write] })
                        }

                        #[test]
                        #[ignore = "real Linux autofs mount: run with --ignored (root makes one); fails naming the missing fixture"]
                        fn an_autofs_mount_is_refused() {
                            if let Some(point) = fixture(|kind| kind == "autofs") {
                                refused_at(&point);
                                return;
                            }
                            let (_guard, dir) = base();
                            let point = dir.join("autofs");
                            std::fs::create_dir(&point).unwrap();
                            let mount = make_autofs(&point).unwrap_or_else(|why| no_fixture("autofs", &why));
                            refused_at(&point);
                            drop(mount);
                        }

                        #[test]
                        #[ignore = "real Linux NFS mount: run with --ignored on a host that has one; fails naming the missing fixture"]
                        fn an_nfs_mount_is_refused() {
                            let point = fixture(|kind| kind == "nfs" || kind == "nfs4")
                                .unwrap_or_else(|| no_fixture("NFS", "making one needs an NFS server"));
                            refused_at(&point);
                        }

                        #[test]
                        #[ignore = "real Linux FUSE mount: run with --ignored on a host that has one; fails naming the missing fixture"]
                        fn a_fuse_mount_is_refused() {
                            let point = fixture(|kind| kind == "fuse" || kind == "fuseblk" || kind.starts_with("fuse."))
                                .unwrap_or_else(|| no_fixture("FUSE", "making one needs a FUSE file-system daemon"));
                            refused_at(&point);
                        }
                    }
                }
            }
        }
    } else if #[cfg(windows)] {
        mod windows {
            use super::super::super::local::address::{ServerAddress, WindowsFault, check_pipe, checks};

            #[test]
            fn a_pipe_name_passes_both_checks_on_this_host() {
                let name = r"\\.\pipe\gwz-address-row";
                assert_eq!(checks(name), Ok(()));
                // Plan 2.3e2: the pipe form of a parsed address gives the name the connect and create take.
                let checked = check_pipe(&ServerAddress::Pipe(name.to_owned())).expect("a pipe").expect("checked");
                assert_eq!(checked.name(), name);
            }

            #[test]
            fn a_name_get_full_path_name_changes_is_refused() {
                assert!(matches!(checks(r"\\.\pipe\a\..\b"), Err(WindowsFault::Renamed { .. })));
                // A pipe variant built by hand, which the grammar refuses, names no pipe: nothing is checked or opened.
                assert!(check_pipe(&ServerAddress::Pipe(r"\\.\pipe\a\..\b".to_owned())).is_none());
                assert!(check_pipe(&ServerAddress::Stdio).is_none());
            }
        }
    }
}
