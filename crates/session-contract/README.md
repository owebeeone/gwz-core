# gwz-session-contract

The channel contract of GWZ's core session: what a frame is, which tags each
carrier carries, the two lanes and their limits, and the `FrameSink` and
`FrameSource` traits that every channel adapter implements. It carries bytes
only. It holds no GWZ or Taut protocol type, decodes no CBOR, does no I/O and
keeps no global state. Its authority is the core session contract's §1 and §3
(gwz-dev `dev-docs/GwzCoreSessionDesign.md`), with the server design's
byte-stream tags, and the core session crate map's §2 and §7.

## API

| Item | What it is |
| --- | --- |
| `Frame { tag: u8, body: Vec<u8> }` | One frame. `Frame::new(Tag, body)`; `size()`, its tag byte plus its body; `check(Carrier) -> Result<Tag, FrameError>`; `Frame::check_size(usize)`. `Debug` never shows a body, which can carry the environment snapshot. |
| `Tag` | The registry: `SessionCall` 1, `SessionReply` 2, `SessionError` 3, and, for byte streams only, `SessionOpen` 4, `SessionOpened` 5, `SessionHello` 6, `ServerControl` 7 and `ServerState` 8. `from_byte`, `byte`, `byte_stream_only`, `ALL`. |
| `Carrier` | `InProcess` (tags 1 to 3) or `ByteStream` (tags 1 to 8). `tag(byte)` is the carrier's answer for a tag byte. |
| `MAX_FRAME_BYTES`, `RESERVED_TAGS` | 64 MiB, counting the tag byte and the body; tags 16 to 31, reserved for a transport lane. |
| `FrameError` | The protocol errors a frame can carry: `UnknownTag`, `ReservedTag`, `ByteStreamOnly`, `TooLarge { size }` and `Empty`. |
| `Lane` | `Call` or `Control`. A reply travels on the lane of the call it answers. |
| `Limits`, `LimitsError` | The outstanding-call limit, the call lane's room, and the control reserve, the control lane's: 1024 and 64 by default. `Limits::new` refuses zero and overflow, so a `Limits` is always valid. |
| `FrameSink` | `send(&self, Frame, Lane) -> Result<(), SendError>` and `close(&self)`. |
| `FrameSource` | `recv(&self) -> Result<Frame, Closed>`. |
| `SendError` | `Full(Frame)` or `Closed(Frame, Closed)`: the frame always comes back. |
| `Closed` | Why a channel ended, as one end sees it: `Local`, `Peer`, `Protocol(FrameError)` or `Stream(io::ErrorKind)`. |
| `contract_tests` | With the `contract-tests` feature: `ChannelFixture` and `run_all`, the conformance suite each adapter runs against itself. |

## What every adapter guarantees

- Frames arrive in the order they were sent, whatever their lanes, each once
  and whole.
- A frame whose tag its carrier does not carry, or over 64 MiB, is a protocol
  error. It ends the session at both ends, and it comes back to its sender.
- Where an adapter bounds its lanes, as the in-process one does, a full lane
  refuses a frame and gives it back, and the channel stays open.
- Closing an end, or dropping it, ends the session. The closing end sends and
  receives nothing more, and its peer receives what was sent before the close,
  then `Closed::Peer`. An end keeps reporting the first reason it met.

The session's own rules are gwz-core's: which party sends which tag, the
handshake's order, call-ID order, decoding bodies, and classifying each
frame's lane.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
