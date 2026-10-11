//! What the machines tell the user (§9): every operation's notes drain
//! together, one reader gets each once, and the sink turns them into lines.
use super::*;
use crate::git::endpoint::setup_retry::{
    Change,
    limit::{note_sink::NoteSink, notes::Note},
};

/// A sink that keeps its lines.
fn collecting() -> (NoteSink, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let kept = lines.clone();
    (
        NoteSink::new(move |line| kept.lock().unwrap().push(line.to_owned())),
        lines,
    )
}

#[test]
fn a_long_retry_after_is_drained_with_its_site_once_for_the_whole_governor() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(20_000), false, 10);
    assert_eq!(
        rig.governor.drain_all_notes(),
        [(rig.site(), Note::Hold { wait_ms: 20_000 })]
    );
    assert_eq!(rig.governor.drain_all_notes(), [], "each note once");
    drop(lease);
}

#[test]
fn the_notes_of_an_operation_that_has_ended_are_still_delivered() {
    let mut rig = Rig::new(8, true);
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(20_000), false, 10);
    rig.governor.end_operation("op", 20);
    assert_eq!(
        rig.governor.drain_all_notes(),
        [(rig.site(), Note::Hold { wait_ms: 20_000 })]
    );
    assert_eq!(rig.governor.drain_all_notes(), [], "and only once");
    drop(lease);
}

#[test]
fn a_sink_is_given_each_note_as_its_line_and_never_twice() {
    let mut rig = Rig::new(8, true);
    let (sink, lines) = collecting();
    sink.deliver(&rig.governor);
    assert_eq!(*lines.lock().unwrap(), Vec::<String>::new());
    let (lease, connection) = rig.open();
    rig.op()
        .refused(&key(), connection, Throttle, Some(20_000), false, 10);
    sink.deliver(&rig.governor);
    sink.deliver(&rig.governor);
    assert_eq!(
        *lines.lock().unwrap(),
        ["note: host:443 asked gwz to wait 20 s; waiting"]
    );
    drop(lease);
}

#[test]
fn a_key_going_down_makes_one_note_until_it_is_healthy_again() {
    let mut rig = Rig::new(8, true);
    let (sink, lines) = collecting();
    rig.op().health(&key(), Change::Down, 10);
    rig.op().health(&key(), Change::Down, 11);
    sink.deliver(&rig.governor);
    assert_eq!(
        *lines.lock().unwrap(),
        ["note: host:443 is not answering; waiting up to 30 s to retry"]
    );
    rig.op().health(&key(), Change::Healed, 12);
    rig.op().health(&key(), Change::Down, 13);
    sink.deliver(&rig.governor);
    assert_eq!(lines.lock().unwrap().len(), 2, "a new episode");
}
