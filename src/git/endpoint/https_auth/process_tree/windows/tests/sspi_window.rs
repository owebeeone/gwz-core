//! The spawn gate against gwz-sspi's own worker-launch window (WH2 contract C9; operator decisions D25 and D31).
//!
//! gwz-sspi 0.1.2 opens its window inside its own `CreateProcessW`, so unlike the helper launch's window above, a
//! row cannot hold it open by hand. The rows launch real workers through the product's native route (`NativeHost`,
//! whose gate is the one the supervisor's launch guard takes) while other threads spawn the probe as a standard
//! `Command`, which names no handle list, and count the anonymous pipes each probe inherited. A worker's pipes are
//! anonymous ones made by CreatePipe, which have no name and the default 4096-byte buffers; none of gwz-core's own
//! helper pipes is (they are named), so a probe that holds more of them than a probe spawned while no worker launches
//! did was handed a worker's. The gated phase also checks the gate itself: no spawn held it while a worker's window did.
//!
//! Whether a spawn lands inside a window is chance, so the control launches until one does (it is bounded, and says
//! how many launches it took) and the gated phase launches a fixed number with the same spawners. The control is what
//! makes the gated phase mean something: the same harness, with the guard and the spawners ungated, sees the pipes;
//! a detector check shows that a probe counts the two ends of an inheritable anonymous pipe it is handed; and a check of
//! the gate and the host's guard with threads alone shows that they exclude each other.
use super::*;
use crate::{git::endpoint::https_worker::native::NativeHost, git::spawn_gate::SpawnGate};
use gwz_sspi::{
    AuthRequest, Cancellation, Deadline, Identity, LaunchGuard, Options, Package, SecretBytes,
    SecretText, Supervisor, TokenLimit, WorkerExecutable,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
};

/// Launches in the gated row.
const GATED_LAUNCHES: usize = 100;
/// The most launches the control row makes before it gives up on a spawn landing in a window.
const CONTROL_LAUNCHES: usize = 600;
/// Threads spawning probes while the launches run.
const SPAWNERS: usize = 3;

fn executable() -> WorkerExecutable {
    // A program that ends at once: the supervisor creates it suspended, resumes it, finds no Hello and settles the
    // failed worker, which is all these rows need of a launch (the window is over by then).
    WorkerExecutable::new(system32().join("hostname.exe"), [0; 32]).unwrap()
}

fn request() -> AuthRequest {
    let mut binding = [0; 53];
    binding[..21].copy_from_slice(b"tls-server-end-point:");
    AuthRequest {
        package: Package::Ntlm,
        target: SecretText::new("HTTP/localhost").unwrap(),
        identity: Identity::CurrentLogon,
        channel_binding: SecretBytes::new(&binding),
        token_limit: TokenLimit::new(65536).unwrap(),
        digest: None,
    }
}

/// One worker launch, through to the supervisor's verdict on the worker (it never is a worker).
fn launch(executor: &tokio::runtime::Runtime, supervisor: &Supervisor) {
    let deadline = Deadline::new(Instant::now() + Duration::from_secs(60));
    let started = executor.block_on(supervisor.start(request(), deadline, Cancellation::new()));
    assert!(started.is_err(), "a program that is no worker became one");
}

/// What a probe held: the anonymous pipes (the ones CreatePipe makes, which have no name) are what a worker's are.
struct Probed {
    anonymous: usize,
    foreign: usize,
    private: usize,
    values: String,
}

/// What the probes inherited: the most anonymous pipes any one of them held.
#[derive(Default)]
struct Seen {
    probes: AtomicUsize,
    anonymous: AtomicUsize,
    /// The count, handle values and foreign count of the probe that held the most, for a failure to say what it held.
    worst: Mutex<(usize, String, usize)>,
    /// Every probe: when its spawn held the gate (from, to), and what it held.
    records: Mutex<Vec<(Instant, Instant, usize, String)>>,
}

impl Seen {
    /// The anonymous pipes a probe held beyond the `baseline` the quiescent probe held: a worker's.
    fn worker_pipes(&self, baseline: usize) -> usize {
        self.anonymous
            .load(Ordering::Acquire)
            .saturating_sub(baseline)
    }
}

/// Runs the probe once, as `spawn` starts it.
fn run_probe(
    spawn: &dyn Fn(&mut std::process::Command) -> io::Result<std::process::Child>,
) -> Probed {
    let (_directory, output, mut command) = standard_probe();
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    assert!(spawn(&mut command).unwrap().wait().unwrap().success());
    let seen = std::fs::read_to_string(&output).unwrap();
    let field = |name: &str| {
        seen.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap_or_else(|| panic!("the probe reported {name}: {seen:?}"))
            .to_owned()
    };
    Probed {
        anonymous: field("anonymous=").parse().unwrap(),
        foreign: field("foreign=").parse().unwrap(),
        private: field("private=").parse().unwrap(),
        values: field("values="),
    }
}

/// Threads that spawn the probe over and over until `stop`, through `gate` or ungated.
fn spawners(
    gate: Option<SpawnGate>,
    stop: &Arc<AtomicBool>,
    seen: &Arc<Seen>,
) -> Vec<JoinHandle<()>> {
    (0..SPAWNERS)
        .map(|_| {
            let (gate, stop, seen) = (gate.clone(), stop.clone(), seen.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let span = std::cell::Cell::new(None);
                    let probed = match &gate {
                        Some(gate) => run_probe(&|command| {
                            let hold = gate.hold();
                            let from = Instant::now();
                            let child = command.spawn();
                            span.set(Some((from, Instant::now())));
                            drop(hold);
                            child
                        }),
                        None => run_probe(&|command| command.spawn()),
                    };
                    if let Some((from, to)) = span.get() {
                        let record = (from, to, probed.anonymous, probed.values.clone());
                        seen.records.lock().unwrap().push(record);
                    }
                    seen.anonymous.fetch_max(probed.anonymous, Ordering::AcqRel);
                    let mut worst = seen.worst.lock().unwrap();
                    if probed.anonymous >= worst.0 {
                        *worst = (probed.anonymous, probed.values, probed.foreign);
                    }
                    seen.probes.fetch_add(1, Ordering::Release);
                }
            })
        })
        .collect()
}

/// The anonymous pipes a probe holds with no worker launching: the baseline the rows subtract.
fn baseline() -> usize {
    let probed = run_probe(&|command| command.spawn());
    eprintln!(
        "baseline: {} inheritable anonymous pipes ({}), {} that are not inheritable, {} foreign handles",
        probed.anonymous, probed.values, probed.private, probed.foreign
    );
    probed.anonymous
}

/// The detector sees what it is for: the two ends of an anonymous pipe, as CreatePipe makes a worker's, that are
/// inheritable when the probe is spawned show as two more anonymous pipes than the baseline.
fn the_probe_counts_the_anonymous_pipes_it_inherits() {
    use windows_sys::Win32::{
        Foundation::CloseHandle, Security::SECURITY_ATTRIBUTES, System::Pipes::CreatePipe,
    };
    let baseline = baseline();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: initialized outputs and attributes; both handles are closed below.
    let made = unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) };
    assert_ne!(made, 0);
    let probed = run_probe(&|command| command.spawn());
    // SAFETY: the two handles CreatePipe made.
    unsafe {
        CloseHandle(read);
        CloseHandle(write);
    }
    assert_eq!(
        probed.anonymous,
        baseline + 2,
        "the probe did not count the two ends of an inheritable anonymous pipe"
    );
}

fn settle(executor: &tokio::runtime::Runtime, supervisor: &Supervisor) {
    let report = executor
        .block_on(supervisor.shutdown(Deadline::new(Instant::now() + Duration::from_secs(60))));
    assert!(report.outstanding.is_empty(), "{:?}", report.outstanding);
}

fn executor() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// The control: a supervisor whose guard holds nothing, and spawners that take no gate. Returns how many launches
/// it took before a spawn inherited a worker's pipes, how many probes ran, and how many worker pipes the most
/// affected probe held.
fn control(executor: &tokio::runtime::Runtime) -> (usize, usize, usize) {
    let supervisor =
        Supervisor::with_launch_guard(executable(), Options::default(), LaunchGuard::new(|| ()))
            .unwrap();
    let baseline = baseline();
    let (stop, seen) = (Arc::new(AtomicBool::new(false)), Arc::new(Seen::default()));
    let threads = spawners(None, &stop, &seen);
    let mut launches = 0;
    let began = Instant::now();
    while launches < CONTROL_LAUNCHES && seen.worker_pipes(baseline) == 0 {
        launch(executor, &supervisor);
        launches += 1;
    }
    stop.store(true, Ordering::Release);
    for thread in threads {
        thread.join().unwrap();
    }
    settle(executor, &supervisor);
    eprintln!("control: {launches} launches took {:?}", began.elapsed());
    eprintln!(
        "control: the fullest probe held {:?}",
        seen.worst.lock().unwrap()
    );
    (
        launches,
        seen.probes.load(Ordering::Acquire),
        seen.worker_pipes(baseline),
    )
}

/// Who is inside the gate, and how often a second one came in.
#[derive(Default)]
struct Occupancy {
    inside: Mutex<Vec<&'static str>>,
    violations: AtomicUsize,
}

impl Occupancy {
    fn enter(&self, who: &'static str) {
        let mut inside = self.inside.lock().unwrap();
        if !inside.is_empty() {
            self.violations.fetch_add(1, Ordering::AcqRel);
        }
        inside.push(who);
    }

    fn leave(&self, who: &'static str) {
        let mut inside = self.inside.lock().unwrap();
        let at = inside.iter().position(|inside| *inside == who);
        inside.remove(at.unwrap_or_else(|| panic!("{who} left without being in")));
    }
}

/// The gate and the supervisor's guard alone, with threads and no process: holders of the gate and holds of the
/// guard (the one the host's `NativeHost` makes for its supervisor) are never inside together, and a hold of either
/// refuses a second. It isolates the gate and the guard from the processes the phases below start.
fn the_guard_and_the_gate_exclude_each_other() {
    let gate = SpawnGate::new();
    let handed = std::cell::RefCell::new(None);
    let _host = NativeHost::on_gate(gate.clone(), |guard| {
        *handed.borrow_mut() = Some(guard);
        Err(gwz_sspi::ErrorKind::UnsupportedPlatform)
    });
    let guard = handed.take().expect("the host's launch guard");
    let occupancy = Arc::new(Occupancy::default());
    let stop = Arc::new(AtomicBool::new(false));
    let holders: Vec<JoinHandle<()>> = (0..SPAWNERS)
        .map(|_| {
            let (gate, stop, occupancy) = (gate.clone(), stop.clone(), occupancy.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let hold = gate.hold();
                    occupancy.enter("a spawn");
                    std::thread::sleep(Duration::from_micros(300));
                    occupancy.leave("a spawn");
                    drop(hold);
                }
            })
        })
        .collect();
    let mut not_refused = 0;
    for _ in 0..200 {
        let hold = guard.hold();
        if gate.try_hold().is_some() {
            not_refused += 1;
        }
        occupancy.enter("a worker's window");
        std::thread::sleep(Duration::from_micros(300));
        occupancy.leave("a worker's window");
        drop(hold);
    }
    stop.store(true, Ordering::Release);
    for holder in holders {
        holder.join().unwrap();
    }
    let violations = occupancy.violations.load(Ordering::Acquire);
    eprintln!(
        "guard and gate: {violations} entries while another was in, {not_refused} holds that did not refuse a second"
    );
    assert_eq!((violations, not_refused), (0, 0));
}

/// What the gated phase with its windows noted found.
struct Gated {
    probes: usize,
    leaked: usize,
    /// The spawns that ran while a worker's window held the gate.
    overlapping: usize,
    /// The windows the guard held, which are one for each launch.
    windows: usize,
    /// The spawns that ran between the first window's start and the last window's end.
    inside: usize,
}

/// The production constructor's chain: `NativeHost::new`, the constructor gwz-cli and gwz-py call, makes the supervisor
/// and the gate, and the spawners take that gate as the backend's spawns do. Returns the probes that ran and how many
/// worker pipes the most affected one held. Nothing wraps the guard: a constructor that dropped it would show here.
fn gated_by_the_constructor(executor: &tokio::runtime::Runtime) -> (usize, usize) {
    let host = NativeHost::new(Ok(executable()));
    let supervisor = host.supervisor().clone().expect("a supervisor on Windows");
    let baseline = baseline();
    let (stop, seen) = (Arc::new(AtomicBool::new(false)), Arc::new(Seen::default()));
    let threads = spawners(Some(host.gate().clone()), &stop, &seen);
    for _ in 0..GATED_LAUNCHES {
        launch(executor, &supervisor);
    }
    stop.store(true, Ordering::Release);
    for thread in threads {
        thread.join().unwrap();
    }
    settle(executor, &supervisor);
    eprintln!(
        "constructor: the fullest probe held {:?}",
        seen.worst.lock().unwrap()
    );
    (
        seen.probes.load(Ordering::Acquire),
        seen.worker_pipes(baseline),
    )
}

/// The same chain with the guard wrapped (`NativeHost::on_gate` over the host's own guard) to note when each worker's
/// window held the gate, so that the gate is checked apart from what the probes happened to inherit.
fn gated(executor: &tokio::runtime::Runtime) -> Gated {
    // The product's composition, the host's guard wrapped to note when each worker's window held the gate.
    let windows: Arc<Mutex<Vec<(Instant, Instant)>>> = Arc::default();
    let host = NativeHost::on_gate(SpawnGate::new(), |guard| {
        let windows = windows.clone();
        Supervisor::with_launch_guard(
            executable(),
            Options::default(),
            LaunchGuard::new(move || Window {
                hold: guard.hold(),
                from: Instant::now(),
                windows: windows.clone(),
            }),
        )
        .map(Arc::new)
        .map_err(|error| error.kind())
    });
    let supervisor = host.supervisor().clone().expect("a supervisor on Windows");
    let baseline = baseline();
    let (stop, seen) = (Arc::new(AtomicBool::new(false)), Arc::new(Seen::default()));
    let threads = spawners(Some(host.gate().clone()), &stop, &seen);
    let began = Instant::now();
    for _ in 0..GATED_LAUNCHES {
        launch(executor, &supervisor);
    }
    stop.store(true, Ordering::Release);
    for thread in threads {
        thread.join().unwrap();
    }
    settle(executor, &supervisor);
    eprintln!(
        "gated: {GATED_LAUNCHES} launches took {:?}",
        began.elapsed()
    );
    eprintln!(
        "gated: the fullest probe held {:?}",
        seen.worst.lock().unwrap()
    );
    // The gate itself, apart from what the probes happened to inherit: no spawn that held the gate ran at any time
    // a worker's window held it. (Both spans are taken inside the holds, so under exclusion they cannot meet.)
    let windows = windows.lock().unwrap();
    let records = seen.records.lock().unwrap();
    let overlapping = records
        .iter()
        .filter(|(from, to, _, _)| {
            windows
                .iter()
                .any(|(window_from, window_to)| window_from < to && from < window_to)
        })
        .count();
    eprintln!(
        "gated: {overlapping} of {} spawns ran while a worker's window held the gate ({} windows)",
        records.len(),
        windows.len()
    );
    let (first, last) = (
        windows.iter().map(|(from, _)| *from).min(),
        windows.iter().map(|(_, to)| *to).max(),
    );
    let inside = records
        .iter()
        .filter(|(from, to, _, _)| {
            first.is_some_and(|first| *from >= first) && last.is_some_and(|last| *to <= last)
        })
        .count();
    Gated {
        probes: seen.probes.load(Ordering::Acquire),
        leaked: seen.worker_pipes(baseline),
        overlapping,
        windows: windows.len(),
        inside,
    }
}

/// A worker's window as the guard held it: the gate's hold, and when it was taken.
struct Window {
    hold: Box<dyn Send>,
    from: Instant,
    windows: Arc<Mutex<Vec<(Instant, Instant)>>>,
}

impl Drop for Window {
    fn drop(&mut self) {
        self.windows
            .lock()
            .unwrap()
            .push((self.from, Instant::now()));
        let _ = &self.hold;
    }
}

/// The row's environment: the body below does nothing unless the row that starts it sets this, so a run of the ignored
/// rows in a shell does nothing either.
const ROW: &str = "GWZ_TEST_SSPI_WINDOW_ROW";

// The row runs in a process of its own. Its count of anonymous pipes is a count of every inheritable anonymous pipe
// the process holds, and in the test run's process another test's would be counted too; in its own, the only ones
// are the baseline's and a worker's.
#[test]
fn a_standard_spawn_during_a_worker_launch_inherits_its_pipes_only_when_the_gate_is_not_held() {
    let (_, module) = module_path!().split_once("::").unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("{module}::the_row_in_its_own_process"),
            "--ignored",
            "--nocapture",
        ])
        .env(ROW, "1")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{text}");
    assert!(output.status.success(), "the row's process failed: {text}");
    assert!(
        text.contains("1 passed"),
        "the row's process ran the row: {text}"
    );
}

#[test]
#[ignore = "the body of the row above, started by it in a process of its own; not a test"]
fn the_row_in_its_own_process() {
    if std::env::var_os(ROW).is_none() {
        return;
    }
    // The detector first and then the control: a worker's pipes of the control's launches would otherwise be
    // inheritable by the gated phase's probes (they hold another gate), so the phases never run at once.
    let executor = executor();
    the_probe_counts_the_anonymous_pipes_it_inherits();
    the_guard_and_the_gate_exclude_each_other();
    let (launches, probes, leaked) = control(&executor);
    eprintln!("control: {launches} launches, {probes} probes, a probe held {leaked} worker pipes");
    assert!(
        leaked > 0,
        "control: no standard spawn landed in a worker's creation window in {launches} launches and {probes} probes"
    );
    let gated = gated(&executor);
    eprintln!(
        "gated: {} probes, a probe held {} worker pipes; {} windows; {} spawns inside their range, {} of them while one held the gate",
        gated.probes, gated.leaked, gated.windows, gated.inside, gated.overlapping
    );
    assert_eq!(
        gated.windows, GATED_LAUNCHES,
        "the guard was held once for each launch"
    );
    assert!(
        gated.inside >= SPAWNERS,
        "the spawners ran during the launches ({} spawns inside the windows' range)",
        gated.inside
    );
    assert_eq!(
        gated.overlapping, 0,
        "a gated spawn ran while a worker's window held the gate"
    );
    assert!(
        gated.probes >= SPAWNERS,
        "the spawners ran during the launches ({} probes)",
        gated.probes
    );
    assert_eq!(gated.leaked, 0, "a gated spawn inherited a worker's pipes");
    let (probes, leaked) = gated_by_the_constructor(&executor);
    eprintln!("constructor: {probes} probes, a probe held {leaked} worker pipes");
    assert!(
        probes >= SPAWNERS,
        "the spawners ran during the launches ({probes} probes)"
    );
    assert_eq!(
        leaked, 0,
        "a spawn gated by NativeHost::new's gate inherited a worker's pipes"
    );
}
