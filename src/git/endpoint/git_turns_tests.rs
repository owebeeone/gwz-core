//! Whose turn it is, read from protocol v0 transcripts of upload-pack and
//! receive-pack as libgit2 and git's upload-pack.c exchange them.
use super::{git_turns::GitTurns, ssh_channel::GitService};

const A: &str = "1111111111111111111111111111111111111111";
const B: &str = "2222222222222222222222222222222222222222";
const FLUSH: &[u8] = b"0000";

fn pkt(line: &str) -> Vec<u8> {
    let mut bytes = format!("{:04x}", line.len() + 4).into_bytes();
    bytes.extend_from_slice(line.as_bytes());
    bytes
}

fn advertisement() -> Vec<u8> {
    [
        pkt(&format!(
            "{A} HEAD\0multi_ack_detailed side-band-64k thin-pack ofs-delta shallow no-done\n"
        )),
        pkt(&format!("{A} refs/heads/main\n")),
        FLUSH.to_vec(),
    ]
    .concat()
}

fn wants(capabilities: &str) -> Vec<u8> {
    [pkt(&format!("want {A} {capabilities}\n")), FLUSH.to_vec()].concat()
}

fn haves(count: usize) -> Vec<u8> {
    let mut bytes: Vec<u8> = (0..count)
        .flat_map(|_| pkt(&format!("have {B}\n")))
        .collect();
    bytes.extend_from_slice(FLUSH);
    bytes
}

struct Exchange(GitTurns);

impl Exchange {
    fn fetch() -> Self {
        Self(GitTurns::new(GitService::UploadPack))
    }
    fn push() -> Self {
        Self(GitTurns::new(GitService::ReceivePack))
    }
    fn advertised(mut self) -> Self {
        assert!(self.server(&advertisement()));
        self
    }
    fn server(&mut self, bytes: &[u8]) -> bool {
        self.0.server_bytes(bytes);
        self.0.clients_turn()
    }
    fn client(&mut self, bytes: &[u8]) -> bool {
        self.0.client_bytes(bytes);
        self.0.clients_turn()
    }
}

#[test]
fn the_advertisement_is_the_servers_turn_until_its_flush() {
    for mut exchange in [Exchange::fetch(), Exchange::push()] {
        let advertisement = advertisement();
        let (refs, flush) = advertisement.split_at(advertisement.len() - FLUSH.len());
        assert!(!exchange.0.clients_turn());
        assert!(!exchange.server(refs));
        assert!(
            exchange.server(flush),
            "choosing wants, or building a pack, is think time"
        );
    }
    let mut v1 = Exchange::fetch();
    assert!(!v1.server(&pkt("version 1\n")));
    assert!(v1.server(&advertisement()));
    let mut v2 = Exchange::fetch();
    assert!(!v2.server(&[pkt("version 2\n"), advertisement()].concat()));
    assert!(
        !v2.client(&wants("multi_ack_detailed")),
        "protocol v2 is not read"
    );
}

#[test]
fn each_negotiation_round_is_the_servers_until_its_nak() {
    let mut exchange = Exchange::fetch().advertised();
    let round = [
        wants("multi_ack_detailed side-band-64k ofs-delta"),
        haves(2),
    ]
    .concat();
    assert!(!exchange.client(&round));
    assert!(!exchange.server(&pkt(&format!("ACK {B} common\n"))));
    assert!(
        exchange.server(&pkt("NAK\n")),
        "the client chooses its next haves"
    );
    assert!(
        !exchange.client(&pkt(&format!("have {B}\n"))),
        "an unflushed round is the client's still, and counts"
    );
    assert!(!exchange.client(FLUSH));
    assert!(!exchange.server(&pkt(&format!("ACK {B} ready\n"))));
    assert!(exchange.server(&pkt("NAK\n")));
    assert!(!exchange.client(&pkt("done\n")));
    assert!(!exchange.server(&pkt(&format!("ACK {B}\n"))));
}

#[test]
fn the_want_section_alone_owes_no_reply() {
    // `receive_needs` answers only a deepen request; otherwise the server
    // waits for haves.
    let mut exchange = Exchange::fetch().advertised();
    assert!(exchange.client(&wants("multi_ack_detailed")));
    assert!(!exchange.client(&haves(1)));
    assert!(exchange.server(&pkt("NAK\n")));
}

#[test]
fn done_hands_the_rest_to_the_server_through_the_sideband_pack() {
    let mut exchange = Exchange::fetch().advertised();
    assert!(!exchange.client(&[wants("multi_ack_detailed side-band-64k"), pkt("done\n")].concat()));
    for packet in [
        pkt("NAK\n"),
        pkt("\x02Enumerating objects: 3, done.\n"),
        pkt("\x01PACK\0\0\0\x02\0\0\0\x03"),
        FLUSH.to_vec(),
    ] {
        assert!(!exchange.server(&packet));
    }
    assert!(!exchange.0.clients_turn());
}

#[test]
fn a_push_is_the_servers_from_the_clients_first_byte() {
    let mut exchange = Exchange::push().advertised();
    let commands = [
        pkt(&format!(
            "{A} {B} refs/heads/main\0report-status side-band-64k\n"
        )),
        FLUSH.to_vec(),
    ]
    .concat();
    assert!(!exchange.client(&commands[..1]));
    assert!(!exchange.client(&commands[1..]));
    assert!(!exchange.client(b"PACK\0\0\0\x02\0\0\0\x01raw pack bytes"));
    for packet in [
        pkt("unpack ok\n"),
        pkt("ok refs/heads/main\n"),
        FLUSH.to_vec(),
    ] {
        assert!(!exchange.server(&packet));
    }
}

#[test]
fn every_chunk_split_reads_the_same_turns() {
    // (client?, bytes, the client's turn once they have all arrived)
    let events: Vec<(bool, Vec<u8>, bool)> = vec![
        (false, advertisement(), true),
        (true, wants("multi_ack_detailed side-band-64k"), true),
        (true, haves(3), false),
        (
            false,
            [pkt(&format!("ACK {B} common\n")), pkt("NAK\n")].concat(),
            true,
        ),
        (true, haves(2), false),
        (
            false,
            [pkt(&format!("ACK {B} ready\n")), pkt("NAK\n")].concat(),
            true,
        ),
        (true, pkt("done\n"), false),
        (
            false,
            [pkt(&format!("ACK {B}\n")), pkt("\x01PACK"), FLUSH.to_vec()].concat(),
            false,
        ),
    ];
    for chunk in 1..=9 {
        let mut exchange = Exchange::fetch();
        for (client, bytes, expected) in &events {
            let pieces: Vec<&[u8]> = bytes.chunks(chunk).collect();
            for (index, piece) in pieces.iter().enumerate() {
                let turn = if *client {
                    exchange.client(piece)
                } else {
                    exchange.server(piece)
                };
                if index + 1 < pieces.len() {
                    assert!(
                        !turn,
                        "a partial event is never the client's turn (chunk {chunk})"
                    );
                } else {
                    assert_eq!(turn, *expected, "chunk {chunk}");
                }
            }
        }
    }
}

#[test]
fn malformed_lengths_hand_the_exchange_to_the_server() {
    // Non-hex digits; protocol v2's delim and response-end; a length below the
    // header; one above LARGE_PACKET_MAX.
    for bad in [&b"00zz"[..], b"0001", b"0002", b"0003", b"fff1", b"ffff"] {
        let mut client = Exchange::fetch().advertised();
        assert!(!client.client(bad));
        assert!(!client.client(&wants("multi_ack_detailed")), "{bad:?}");

        let mut server = Exchange::fetch();
        assert!(!server.server(bad));
        assert!(!server.server(&advertisement()), "{bad:?}");
    }
}

#[test]
fn lines_are_read_whole_or_not_at_all() {
    let long_ref = format!("{A} refs/heads/{}\n", "x".repeat(4000));
    let mut exchange = Exchange::fetch();
    assert!(exchange.server(&[pkt(&long_ref), advertisement()].concat()));
    let long_want = format!("want {A} multi_ack_detailed agent={}\n", "y".repeat(2000));
    assert!(!exchange.client(&[pkt(&long_want), FLUSH.to_vec()].concat()));
    assert!(!exchange.client(&haves(1)));
    assert!(
        !exchange.server(&pkt("NAK\n")),
        "an unread capability could be no-done"
    );
}

#[test]
fn deepen_waits_for_the_shallow_list() {
    let mut exchange = Exchange::fetch().advertised();
    let request = [
        pkt(&format!("want {A} multi_ack_detailed shallow\n")),
        pkt("deepen 1\n"),
        FLUSH.to_vec(),
    ]
    .concat();
    assert!(!exchange.client(&request));
    assert!(!exchange.server(&pkt(&format!("shallow {A}\n"))));
    assert!(exchange.server(FLUSH));
    assert!(!exchange.client(&haves(1)));
    assert!(exchange.server(&pkt("NAK\n")));
}

#[test]
fn no_done_or_a_bare_ack_ends_the_count() {
    // no-done: after ACK ready the server sends the pack without waiting.
    let mut no_done = Exchange::fetch().advertised();
    assert!(!no_done.client(&[wants("multi_ack_detailed no-done"), haves(1)].concat()));
    assert!(!no_done.server(&pkt("NAK\n")));
    // Without multi_ack, no NAK follows the first common commit's ACK.
    let mut plain = Exchange::fetch().advertised();
    assert!(!plain.client(&[wants("side-band-64k"), haves(1)].concat()));
    assert!(!plain.server(&pkt(&format!("ACK {B}\n"))));
    assert!(!plain.client(&haves(1)));
    assert!(!plain.server(&pkt("NAK\n")));
}

#[test]
fn stderr_counts_only_while_the_server_has_the_turn() {
    let mut exchange = Exchange::fetch();
    let advertisement = advertisement();
    let (refs, flush) = advertisement.split_at(advertisement.len() - FLUSH.len());
    assert!(!exchange.server(refs));
    exchange.0.server_stderr();
    assert!(exchange.server(flush));
    exchange.0.server_stderr();
    assert!(!exchange.0.clients_turn());
    assert!(!exchange.client(&wants("multi_ack_detailed")));
}

#[test]
fn nothing_wanted_a_half_close_or_output_out_of_turn_end_the_tracking() {
    let mut nothing = Exchange::fetch().advertised();
    assert!(!nothing.client(FLUSH));
    let mut closed = Exchange::fetch().advertised();
    closed.0.client_end();
    assert!(!closed.0.clients_turn());
    let mut chatty = Exchange::fetch().advertised();
    assert!(!chatty.server(&pkt("NAK\n")));
    assert!(!chatty.client(&wants("multi_ack_detailed")));
    let mut refused = Exchange::fetch();
    assert!(!refused.server(&[pkt("ERR access denied\n"), FLUSH.to_vec()].concat()));
    assert!(!GitTurns::untracked().clients_turn());
}
