# gwz-session-channel

The two channel adapters of GWZ's core session, which carry frames between a
client and its session host. The in-process pair implements `gwz-session-contract`'s `FrameSink` and
`FrameSource`; a byte-stream end implements `FrameSink` and delivers to a
`FrameRouter`. Both pass its conformance suite (a byte stream through a test
router that queues what it is given). They carry
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
- `send(frame, lane, key)` never waits and ignores the key. A full lane refuses the frame with `SendError::Full` and
  gives it back; the channel stays open. A frame holds its lane's room until
  the receiver takes it. Core reports a full lane as
  `transport_session_full`.
- `recv` waits until a frame arrives or the channel has ended. A close at
  either end wakes every waiting `recv`.
- It carries tags 1 to 3, 16 and 17. Any other tag, or a frame over 64 MiB, ends the
  session at both ends, and both report the protocol error.
- Closing an end, or dropping it, ends the session. The peer still receives
  what was sent before, then `Closed::Peer`.

## `byte_stream(read, write, router) -> io::Result<ByteStreamEnd<W>>`

One end of a channel over a byte stream, such as standard streams, a socket or
SSH. A byte-stream end is a `FrameSink` only; what it reads goes to the
`FrameRouter` that core supplies (operator decision D-N,
`GwzSplitCommandChannel.md` §3.3).

- Each frame is its size as a little-endian `u32`, then its tag byte, then its
  body, exactly as taut-shape's interop tool frames it. The size counts the
  tag byte and the body. The tests check this against vectors that the tool's
  own framing produced.
- **Reading.** The end reads in a thread of its own. It checks the length
  prefix before it reads on: a prefix of zero, or over 64 MiB, ends the
  session before anything is allocated. It checks the tag before it reads the
  body (tags 1 to 8, 16 and 17), then hands each frame, as a tag and a body, to
  `router.route`. `route` never blocks, so the reader never waits for a
  consumer; core decodes the body there and owns every per-class and
  per-binding queue. An error from `route` ends the session with that reason.
  The router is told once, by `ended`, with the reason that stands.
- End of stream between frames is `Closed::Peer`. Inside a frame, or on a
  failed stream, the session ends with `Closed::Stream`. A stream that
  panicked in the middle of a frame ends the session too.
- **Writing.** `send(frame, lane, key)` writes and flushes whole frames, one
  at a time, and waits while the writer does. Senders that wait together take
  turns by the writer's rule: at each frame boundary the oldest frame whose
  key is control is written ahead of the ordinary frames before it, except
  ordinary frames of its own target (core derives it from the call, or from the
  request, session and stream) and any `SessionCall` (tag 1), which it never
  overtakes; so the order of one target's frames and the increasing call-ID
  order survive. The channel decodes no body: it orders by the key and the tag
  byte. A sender that never waits with another is written in call order. The
  lane bounds nothing on a byte stream. The rule is `WriteQueue`, which a
  carrier that queues frames ahead of its own writer thread uses too.
- When the session ends at this end, the end drops its writer, so the peer
  reads end of stream after the frames sent before. A call or the reader
  already waiting inside the stream stays there until the stream returns.
  Only the stream's owner can force it, by shutting a socket down or killing a
  child. A half whose drop does not end the peer's stream, such as one clone
  of a socket, must be wrapped by its owner so that it does.

The in-process pair takes the key and ignores it. Both ends are `Send` and
`Sync`, so one thread can wait in `recv` while others send.

This crate is an internal component of GWZ, published so that `gwz-core` can be built from crates.io. It is versioned in lockstep with the other internal crates on a `0.0.N` line and makes no compatibility promise of its own; depend on `gwz-core` instead.

Source, issues and documentation: https://github.com/owebeeone/gwz-core
