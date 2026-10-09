//! §4.1's table: the states, the counts the machine reads, `Ts`.
use super::states::*;
use std::collections::BTreeSet;

const A: ConnId = ConnId(1);
const B: ConnId = ConnId(2);
const CLOCKED: ConnEvent = ConnEvent::Started { clocked: true };

fn table_with(connected: u64) -> Table {
    let mut table = Table::new();
    for id in 1..=connected {
        table.apply(ConnId(id), CLOCKED, 0);
        table.apply(ConnId(id), ConnEvent::Connected, 0);
    }
    table
}

#[test]
fn a_connection_walks_setting_up_connected_closing_settling_gone() {
    let mut table = Table::new();
    let step = |table: &mut Table, event, now| table.apply(A, event, now);
    let change = step(&mut table, CLOCKED, 0).unwrap();
    assert_eq!((change.from, change.to), (None, Phase::SettingUp));
    assert_eq!(
        (table.connected(), table.possible(), table.held()),
        (0, 1, 1)
    );
    step(&mut table, ConnEvent::Connected, 10);
    assert_eq!(
        (table.connected(), table.possible(), table.held()),
        (1, 1, 1)
    );
    step(&mut table, ConnEvent::Closing, 20);
    assert_eq!(
        (table.connected(), table.possible(), table.held()),
        (0, 1, 1)
    );
    // Settling for Ts (250 ms with no connect measured): possible, not held.
    let change = step(&mut table, ConnEvent::Disposed, 30).unwrap();
    assert_eq!(
        (change.from, change.to),
        (Some(Phase::Closing), Phase::Settling)
    );
    assert_eq!(
        (table.connected(), table.possible(), table.held()),
        (0, 1, 0)
    );
    assert_eq!(table.next_settle_deadline(), Some(280));
    table.advance(279);
    assert_eq!(table.phase(A), Some(Phase::Settling));
    table.advance(280);
    assert_eq!(table.phase(A), None);
    assert_eq!((table.possible(), table.next_settle_deadline()), (0, None));
}

#[test]
fn a_setup_the_server_ended_is_gone_at_once_and_leaves_no_hold() {
    let mut table = Table::new();
    table.apply(A, CLOCKED, 0);
    let change = table.apply(A, ConnEvent::SetupEnded, 5).unwrap();
    assert_eq!(change.to, Phase::Gone);
    assert_eq!((table.possible(), table.next_settle_deadline()), (0, None));
    assert!(table.is_quiet());
}

#[test]
fn the_server_ending_a_connected_or_closing_connection_is_gone() {
    let mut table = table_with(2);
    table.apply(B, ConnEvent::Closing, 1);
    assert_eq!(
        table.apply(A, ConnEvent::ServerClosed, 2).unwrap().to,
        Phase::Gone
    );
    assert_eq!(
        table.apply(B, ConnEvent::ServerClosed, 2).unwrap().to,
        Phase::Gone
    );
    assert_eq!(table.possible(), 0);
}

#[test]
fn an_abandoned_setup_is_setting_up_until_its_job_retires_then_settles() {
    let mut table = Table::new();
    table.apply(A, CLOCKED, 0);
    table.apply(A, ConnEvent::Retired, 2_000);
    assert_eq!(table.phase(A), Some(Phase::Settling));
    assert_eq!(table.next_settle_deadline(), Some(2_250));
}

#[test]
fn quiet_needs_no_clocked_setup_nothing_closing_and_nothing_settling() {
    let mut table = table_with(2);
    assert!(table.is_quiet(), "idle Connected connections are quiet");
    table.apply(ConnId(3), CLOCKED, 0);
    assert!(!table.is_quiet(), "a clocked setup in flight");
    table.apply(ConnId(3), ConnEvent::Connected, 0);
    table.apply(A, ConnEvent::Closing, 0);
    assert!(!table.is_quiet(), "closing");
    table.apply(A, ConnEvent::Disposed, 0);
    assert!(!table.is_quiet(), "settling");
    table.advance(250);
    assert!(table.is_quiet());
}

#[test]
fn a_setup_with_no_clock_counts_but_does_not_keep_the_key_from_being_quiet() {
    // Case 47: --ssh-timeout 0 and a hung setup.
    let mut table = table_with(1);
    table.apply(B, ConnEvent::Started { clocked: false }, 0);
    assert!(table.is_quiet());
    assert_eq!(
        (table.possible(), table.held(), table.connected()),
        (2, 2, 1)
    );
}

#[test]
fn a_transition_the_table_does_not_have_changes_nothing() {
    let mut table = table_with(1);
    assert_eq!(table.apply(A, ConnEvent::Connected, 0), None, "twice");
    assert_eq!(table.apply(A, ConnEvent::Disposed, 0), None, "not closing");
    assert_eq!(table.apply(A, CLOCKED, 0), None, "already known");
    assert_eq!(table.apply(B, ConnEvent::Closing, 0), None, "unknown");
    assert_eq!(
        table.apply(A, ConnEvent::SetupEnded, 0),
        None,
        "set up already"
    );
    assert_eq!((table.connected(), table.possible()), (1, 1));
}

#[test]
fn the_id_sets_are_the_states_the_windows_read() {
    let mut table = table_with(3);
    table.apply(ConnId(4), CLOCKED, 0);
    table.apply(A, ConnEvent::Closing, 0);
    table.apply(B, ConnEvent::Closing, 0);
    table.apply(B, ConnEvent::Disposed, 0);
    let ids = |ids: &[u64]| ids.iter().map(|id| ConnId(*id)).collect::<BTreeSet<_>>();
    assert_eq!(table.connected_ids(), ids(&[3]));
    assert_eq!(table.possible_ids(), ids(&[1, 2, 3, 4]));
    assert_eq!(table.winding_down_ids(), ids(&[1, 2]));
    assert!(table.any_winding_down(&ids(&[2, 3])));
    assert!(!table.any_winding_down(&ids(&[3, 4])));
    table.advance(250);
    assert!(
        table.any_winding_down(&ids(&[1])),
        "closing has no deadline"
    );
    assert!(!table.any_winding_down(&ids(&[2])), "settled");
}

#[test]
fn ts_is_250_ms_until_a_connect_is_measured_then_twice_the_smoothed_connect_plus_100() {
    let mut settle = Settle::default();
    assert_eq!(settle.ts(), 250);
    settle.observe_connect(400);
    assert_eq!(settle.ts(), 900);
    // SRTT := 7/8 SRTT + 1/8 sample.
    settle.observe_connect(80);
    assert_eq!(settle.ts(), 2 * 360 + 100);
    let mut quick = Settle::default();
    quick.observe_connect(50);
    assert_eq!(quick.ts(), 250, "the floor");
}

#[test]
fn the_table_settles_with_the_ts_in_force_when_the_connection_is_disposed() {
    let mut table = table_with(1);
    table.settle_mut().observe_connect(400);
    table.apply(A, ConnEvent::Closing, 100);
    table.apply(A, ConnEvent::Disposed, 100);
    assert_eq!(table.next_settle_deadline(), Some(1_000));
}

#[test]
fn setups_in_flight_counts_clocked_setups_only() {
    let mut table = table_with(2);
    assert_eq!(table.setups_in_flight(), 0);
    table.apply(ConnId(3), CLOCKED, 0);
    table.apply(ConnId(4), ConnEvent::Started { clocked: false }, 0);
    assert_eq!(table.setups_in_flight(), 1);
    table.apply(ConnId(3), ConnEvent::Connected, 0);
    assert_eq!(table.setups_in_flight(), 0);
}
