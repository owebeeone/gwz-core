//! The `Retry-After` hold (§4.5) and the notes of §9.
use super::{hold::*, notes::*};

#[test]
fn a_hold_runs_for_retry_after_capped_at_thirty_seconds() {
    let mut hold = Hold::default();
    assert_eq!(hold.set(3_600_000, Origin::Discovery, 100), Some(30_000));
    assert!(hold.in_force(30_099));
    assert_eq!(hold.deadline(100), Some(30_100));
}

#[test]
fn a_short_hold_lifts_at_its_end_and_makes_no_note_or_discard() {
    let mut hold = Hold::default();
    assert_eq!(
        hold.set(1_000, Origin::Discovery, 0),
        None,
        "1 s is not longer than 1 s"
    );
    assert!(hold.in_force(999));
    assert!(!hold.in_force(1_000));
    assert!(!hold.discard_due(1_000));
}

#[test]
fn a_long_hold_asks_for_one_discard_and_lifts_only_after_it_is_done() {
    let mut hold = Hold::default();
    assert_eq!(hold.set(20_000, Origin::Discovery, 100), Some(20_000));
    assert!(!hold.discard_due(20_099));
    assert!(hold.discard_due(20_100));
    assert!(!hold.discard_due(20_100), "asked once");
    assert!(
        hold.in_force(25_000),
        "still held until the discard is done"
    );
    assert_eq!(hold.deadline(25_000), Some(25_000), "wake now to finish it");
    hold.idle_discarded();
    assert!(!hold.in_force(25_000));
    assert_eq!(hold.deadline(25_000), None);
}

#[test]
fn a_second_retry_after_extends_the_hold_without_a_second_note() {
    let mut hold = Hold::default();
    assert_eq!(hold.set(5_000, Origin::Discovery, 0), Some(5_000));
    assert_eq!(hold.set(8_000, Origin::Discovery, 1_000), None);
    assert!(hold.in_force(8_999));
    assert_eq!(
        hold.set(2_000, Origin::Discovery, 2_000),
        None,
        "never shortened"
    );
    assert!(hold.in_force(8_999));
}

#[test]
fn only_a_post_hold_blocks_a_continuing_member() {
    let mut hold = Hold::default();
    hold.set(5_000, Origin::Discovery, 0);
    assert!(hold.in_force(1) && !hold.blocks_continuing(1));
    let mut hold = Hold::default();
    hold.set(5_000, Origin::Post, 0);
    assert!(hold.blocks_continuing(1));
    assert!(hold.discard_due(5_000));
    assert!(hold.blocks_continuing(5_000), "until the discard has run");
    hold.idle_discarded();
    assert!(!hold.blocks_continuing(5_000));
}

#[test]
fn the_overload_note_waits_for_the_wave_and_names_the_final_n_once() {
    let mut notes = Notes::default();
    notes.overload();
    notes.flush(0, 3, 32, false);
    assert_eq!(notes.drain(), []);
    notes.flush(10, 8, 32, true);
    assert_eq!(notes.drain(), [Note::Overload { n: 8, ceiling: 32 }]);
    notes.overload();
    notes.flush(20, 6, 32, true);
    assert_eq!(notes.drain(), [], "once per host");
}

#[test]
fn a_later_change_is_noted_at_most_once_per_two_seconds() {
    let mut notes = Notes::default();
    notes.overload();
    notes.flush(100, 8, 32, true);
    notes.drain();
    notes.n_changed(12);
    notes.flush(200, 12, 32, true);
    assert_eq!(notes.drain(), []);
    assert_eq!(notes.deadline(), Some(2_100));
    notes.n_changed(14);
    notes.flush(2_100, 14, 32, true);
    assert_eq!(notes.drain(), [Note::Changed { n: 14 }]);
    assert_eq!(notes.deadline(), None);
    // Back to the noted value: nothing to say.
    notes.n_changed(8);
    notes.n_changed(14);
    notes.flush(9_000, 14, 32, true);
    assert_eq!(notes.drain(), []);
}

#[test]
fn back_at_the_ceiling_is_noted_only_after_a_decrease_was() {
    let mut notes = Notes::default();
    notes.saturated(32);
    assert_eq!(notes.drain(), [], "the initial state is not a return");
    notes.overload();
    notes.flush(0, 8, 32, true);
    notes.n_changed(20);
    notes.saturated(32);
    notes.flush(5_000, 32, 32, true);
    assert_eq!(
        notes.drain(),
        [
            Note::Overload { n: 8, ceiling: 32 },
            Note::BackAtCeiling { ceiling: 32 }
        ]
    );
}

#[test]
fn an_overload_that_was_undone_before_its_wave_resolved_is_not_noted() {
    let mut notes = Notes::default();
    notes.overload();
    notes.saturated(32);
    notes.flush(0, 32, 32, true);
    assert_eq!(notes.drain(), []);
}

#[test]
fn a_hold_note_is_passed_through() {
    let mut notes = Notes::default();
    notes.hold(20_000);
    assert_eq!(notes.drain(), [Note::Hold { wait_ms: 20_000 }]);
}

fn ssh(host: &str, port: u16) -> gwz_transport::pool::Site {
    gwz_transport::pool::Site {
        scheme: gwz_transport::protocol::Scheme::Ssh,
        host: host.into(),
        port,
    }
}

#[test]
fn each_note_reads_as_section_9_words_it_with_the_host_and_port() {
    let site = ssh("github.com", 443);
    let line = |note: Note| note.render(&site);
    assert_eq!(
        line(Note::Overload { n: 8, ceiling: 32 }),
        "note: github.com:443 refused extra connections; using 8 at a time (ceiling 32 from --max-per-host); continuing, no action needed"
    );
    assert_eq!(
        line(Note::Changed { n: 12 }),
        "note: github.com:443: now using 12 connections"
    );
    assert_eq!(
        line(Note::Changed { n: 1 }),
        "note: github.com:443: now using 1 connection"
    );
    assert_eq!(
        line(Note::BackAtCeiling { ceiling: 32 }),
        "note: github.com:443: back at the ceiling of 32 connections"
    );
    assert_eq!(
        line(Note::Hold { wait_ms: 20_000 }),
        "note: github.com:443 asked gwz to wait 20 s; waiting"
    );
    assert_eq!(
        line(Note::Hold { wait_ms: 1_500 }),
        "note: github.com:443 asked gwz to wait 2 s; waiting",
        "a part of a second is a whole one: the wait is never understated"
    );
    assert_eq!(
        Note::Down.render(&ssh("github.com", 22)),
        "note: github.com:22 is not answering; waiting up to 30 s to retry"
    );
    assert_eq!(
        Note::Down.render(&ssh("::1", 22)),
        "note: [::1]:22 is not answering; waiting up to 30 s to retry",
        "an IPv6 literal is bracketed"
    );
}

#[test]
fn the_down_note_is_made_once_per_episode_until_the_key_is_healthy_again() {
    let mut notes = Notes::default();
    notes.down();
    notes.down();
    assert_eq!(notes.drain(), [Note::Down]);
    notes.down();
    assert_eq!(notes.drain(), [], "the same episode");
    notes.healthy();
    notes.down();
    assert_eq!(notes.drain(), [Note::Down], "a new episode");
}
