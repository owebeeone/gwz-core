# gwz-session-channel

The two channel adapters of GWZ's core session, which carry frames between a
client and its session host. Both implement `gwz-session-contract`'s
`FrameSink` and `FrameSource`, and both pass its conformance suite. They carry
bytes only: nothing here decodes a body, and the crate keeps no global state.
Its authority is the core session contract's §3 (gwz-dev
`dev-docs/GwzCoreSessionDesign.md`, with the server design's byte-stream tags)
and the core session crate map's §2.

## `pair(capacity: Limits) -> (InProcessEnd, InProcessEnd)`

A session's in-process channel, as its client end and its host end.

- Each direction has its own queue. Each queue holds the outstanding-call
  limit on the call lane plus the control reserve on the control lane, so a
  control frame goes through while the call lane is full. The queues are
  bounded by counters and start empty.
- `send` never waits. A full lane refuses the frame with `SendError::Full` and
  gives it back; the channel stays open. A frame holds its lane's room until
  the receiver takes it. Core reports a full lane as
  `transport_session_full`.
- `recv` waits until a frame arrives or the channel has ended. A close at
  either end wakes every waiting `recv`.
- It carries tags 1 to 3. Any other tag, or a frame over 64 MiB, ends the
  session at both ends, and both report the protocol error.
- Closing an end, or dropping it, ends the session. The peer still receives
  what was sent before, then `Closed::Peer`.

## `byte_stream(read: impl Read, write: impl Write) -> ByteStreamEnd<R, W>`

One end of a channel over a byte stream, such as standard streams, a socket or
SSH.

- Each frame is its size as a little-endian `u32`, then its tag byte, then its
  body, exactly as taut-shape's interop tool frames it. The size counts the
  tag byte and the body. The tests check this against vectors that the tool's
  own framing produced.
- `recv` checks the length prefix before it reads on: a prefix of zero, or
  over 64 MiB, ends the session before anything is allocated. It checks the
  tag before it reads the body, and it carries tags 1 to 8.
- End of stream between frames is `Closed::Peer`. Inside a frame, or on a
  failed stream, the session ends with `Closed::Stream`. A stream that
  panicked in the middle of a frame ends the session too.
- `send` writes and flushes whole frames, one at a time, and waits while the
  writer does. It does not use the lane.
- When the session ends at this end, the end drops its writer, so the peer
  reads end of stream after the frames sent before. A call already waiting
  inside the stream stays there until the stream returns. Only the stream's
  owner can force it, by shutting a socket down or killing a child. A half
  whose drop does not end the peer's stream, such as one clone of a socket,
  must be wrapped by its owner so that it does.

Both ends are `Send` and `Sync`, so one thread can wait in `recv` while others
send.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
