# gwz-ids

An internal component crate of [GWZ](https://github.com/owebeeone/gwz-core):
unique numbers from a source each context owns. It is a pure crate with no
dependencies, and no stable API outside GWZ.

```rust
use gwz_ids::IdSource;

let ids = IdSource::new(0x0123_4567_89ab_cdef);
assert_eq!(ids.next(), 0);
assert_eq!(ids.unique().to_string(), "0123456789abcdef-1");
```

- `IdSource::new(prefix)` makes a source. The counter lives inside the
  instance, never in a static; the source is `Send + Sync`, so a context can
  share it across threads.
- `next()` returns 0, 1, 2 and so on: unique within the source. It panics
  rather than repeat a number, after `u64::MAX` draws.
- `unique()` pairs the prefix with the next number. It is unique across
  sources, contexts and processes unless two prefixes are equal.

## The representation

A `UniqueId` displays as `{prefix:016x}-{sequence:x}`: the prefix as 16
lowercase hexadecimal digits, a `-`, and the sequence number in lowercase
hexadecimal without padding. For example, prefix `0x2a` and sequence 31 display
as `000000000000002a-1f`. The characters are safe in a file name on every
platform, so callers embed it in temporary names between their own markers,
such as `.{name}.tmp.{unique}`.

## Where prefixes come from

gwz-core draws each context's 64-bit prefix from the operating system's random
source, so two contexts collide only if two random draws are equal. This crate
never draws one itself, and tests pass fixed prefixes.
