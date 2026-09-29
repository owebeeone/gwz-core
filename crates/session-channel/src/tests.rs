#![cfg(test)]
//! Both adapters meet the channel contract: each runs gwz-session-contract's
//! shared conformance suite against itself (LBT-009).

use std::io::{self, PipeReader, PipeWriter};

use crate::{ByteStreamEnd, InProcessEnd, byte_stream, pair};
use gwz_session_contract::contract_tests::{ChannelFixture, run_all};
use gwz_session_contract::{Carrier, Limits};

/// The in-process pair, with the contract's default limits.
struct InProcess;

impl ChannelFixture for InProcess {
    type End = InProcessEnd;

    fn carrier(&self) -> Carrier {
        Carrier::InProcess
    }

    fn pair(&mut self) -> (InProcessEnd, InProcessEnd) {
        pair(Limits::default())
    }
}

/// Two byte-stream ends over two OS pipes, one for each direction: real
/// streams, with no process.
struct Pipes;

impl ChannelFixture for Pipes {
    type End = ByteStreamEnd<PipeReader, PipeWriter>;

    fn carrier(&self) -> Carrier {
        Carrier::ByteStream
    }

    fn pair(&mut self) -> (Self::End, Self::End) {
        let (a_reads, b_writes) = io::pipe().expect("an OS pipe");
        let (b_reads, a_writes) = io::pipe().expect("an OS pipe");
        (
            byte_stream(a_reads, a_writes),
            byte_stream(b_reads, b_writes),
        )
    }
}

#[test]
fn the_in_process_pair_meets_the_channel_contract() {
    run_all(&mut InProcess);
}

#[test]
fn a_byte_stream_over_pipes_meets_the_channel_contract() {
    run_all(&mut Pipes);
}
