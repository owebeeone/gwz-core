# Splitter Tool — CST/Parser-Based God-File Splitter (Plan)

Status: draft plan — **Revised R1** after Review-55 (§6: corrected proof model +
import scope; §5's tiers and the `crate::<dest>` rule superseded). **§6 tiers ·
§7 test plan · §8 build brief** are the implementation-ready core — hand those to
an implementer.
Date: 2026-06-17
Context: we have 5 files over the 1000-LOC split trigger (`workspace_ops` 4175,
`main.rs` 2746, `operation` 1377, `status` 1054, `local_workflows` 1008) and more
to come. The O(n) split methodology (CLAUDE.md) currently runs on a line-based,
column-0 scanner. This plan proposes mechanizing it as a real parser-based tool
to cut churn and miscut risk, with two explicit additions: **symbol
cross-reference (adjacency) analysis** and **import pulling/merging**.

## 1. The methodology is not changing

The tool implements the existing O(n) technique verbatim — it only makes two of
its phases parser-accurate:

1. **Explode** — segment the file at top-level item boundaries, each item with
   its leading doc/attr/comment block → one chunk per item + a manifest
   (idx · name · kind · line-span · LOC · **adjacency** · **imports-used**).
2. **Classify** — fill a `dest` column per chunk. **Human-owned**, advisory
   grouping from the tool. (CLAUDE.md: *don't solve the clustering graph; lean to
   more files.* The tool computes adjacency and a suggested greedy grouping, but
   the human owns the final `dest` — the tool never force-clusters.)
3. **Reassemble** — place each item's body per `dest` (byte-sliced from the
   original, then rustfmt-settled) + a freshly merged `use` header; update `mod`
   declarations; compiler-drives the residual visibility (`pub(crate)`).
4. **Prove** — per-item **AST-canonical equivalence** (§3.1) + the carve-out gate
   stays green ⇒ behavior-preserving.

Invariants kept: **item semantics are preserved**, verified by **AST-canonical
equivalence** (rustfmt-canonical per-item equality, §3.1) — *not* byte-identity;
**compiler-drives residual imports/visibility**; **green gate is the final proof**.
Relaxing byte-identity → AST-identity is the upgrade: it lets the tool re-emit
clean merged imports and fmt-clean output, and retires the separate fmt sweep.

## 2. Why upgrade from the line scanner

The column-0 line scanner works but carries a long tail of special-cases, each a
churn/error source (all from CLAUDE.md's "learned the hard way" list):

- backing the boundary up over `///` / `//!` / `//` / `#[…]` blocks,
- nested registration blocks (`#[starlark_module] fn rules(b)`) needing inner
  sub-splitting and column-0 brace anchoring,
- struct fields read across the split needing `pub(crate)`,
- miscuts that only surface at the lossless-diff step.

A parser **knows** the item boundaries, attributes, and references, so the first
two vanish and the last is guaranteed by construction. It also unlocks the two
capabilities a regex can't do well: a real reference graph and structured imports.

## 3. CST / parser recommendation

**Recommendation: `syn` + `proc-macro2` (with `span-locations`) for analysis and
the proof; byte-slice item bodies + `rustfmt` for the move. Defer `tree-sitter`
to a documented pivot.**

### 3.1 Proof model: AST-canonical equivalence, not byte-identity

> **Corrected by §6 (Review-55).** rustfmt-canonical per-item equality is a
> *move-integrity* check (the tool didn't mangle the item), **not** a behavior
> proof — moving an item changes its name-resolution environment. The behavior
> proof is the compiler + tests over the cfg/target matrix. Read §6 for the two
> distinct gates. The paragraph below overstates "behavior-preserving."

The proof that a split is behavior-preserving is **not** that the bytes moved
unchanged — that is an overly strong proxy. The necessary property is that each
item's **meaning** is unchanged: its AST is identical before and after; only its
file and surrounding imports/visibility differ. Verify that directly with
**rustfmt-canonical per-item equality** — `rustfmt(item_before) ==
rustfmt(item_after)`. Because `rustfmt` normalizes whitespace **and preserves
comments**, this is a **CST-grade** check (it catches a dropped or moved comment)
with AST-level simplicity, and it is formatting-independent.

This is the load-bearing relaxation, and it changes the tool's freedoms:
- The tool may **re-emit and reformat** freely — clean **merged `use` headers**,
  fmt-clean output — because the proof tolerates any formatting that preserves the
  AST. The CLAUDE.md "no rustfmt / separate fmt sweep" rule is retired *for the
  splitter's own output*.
- Bodies are still **byte-sliced** from the original (then rustfmt-settled into
  their new module), so comments ride along verbatim; we never re-emit a body
  *from* the tree. This is exactly why a pure-AST tool (`syn`) suffices — the one
  thing an AST drops (comments) passes only through the text slice, never the
  tree. A pure CST would let us re-emit bodies from the tree too, but we don't
  want to reformat bodies, so we don't need it.
- AST-equality *alone* would miss comments — which is why the proof is
  rustfmt-canonical (comment-sensitive), not bare token-stream equality. (This is
  the "ast/**cst**" distinction made checkable.)

So we need: (a) exact item boundaries (byte offsets), (b) precise reference
analysis (adjacency), (c) structured `use` for import merging, (d) a
comment-preserving move + a comment-sensitive proof. `syn` (+ proc-macro2 spans)
for (a)–(c) and the proof, `rustfmt` for (d) — far better analysis ergonomics
than a generic CST, and a stronger proof than the old byte-diff.

### Why `syn`

- **Exact boundaries as bytes.** `syn::parse_file` → `file.items`; each item's
  `proc_macro2::Span` gives a byte range (`span-locations` feature exposes
  `start()/end()` LineColumn and `byte_range()` for string-parsed sources;
  fallback: a line-start index maps LineColumn → byte offset). Doc comments and
  attributes are part of the item (lexed as `#[doc]`/attrs), so the "back up over
  doc/attrs" refinement is automatic.
- **Precise references (adjacency).** `syn::visit::Visit` walks each item's body
  and collects every referenced path/ident; intersect with the sibling-item name
  set → adjacency edges. A typed AST distinguishes a path `foo::bar` from a field
  access — far cleaner than matching identifier nodes by text.
- **Structured imports.** `ItemUse` parses into a `UseTree`, so per-destination
  `use` blocks can be **merged and deduped precisely** (`use a::{B, C};` folded
  from two imports) instead of string-wrangled.
- **Fits the workspace.** Pure Rust, no C toolchain; ships as a small crate /
  xtask alongside the code it splits.
- **Validity is a non-issue.** `syn` requires the file to parse — but we only ever
  split committed, **green** files, so error-resilience (a CST selling point) buys
  nothing here.

### Trivia (ordinary `//` comments)

`syn` attaches doc comments + attributes to items but drops free-standing `//`
comments. Handled without a lossless CST by **gap-slicing**: chunks *tile* the
file — chunk_i = `src[cut_i .. end_i]`, where the gap `[end_{i-1} .. start_i]` is
assigned to the following item, cut at the last blank line (trailing blanks stay
with the previous item; a leading comment block travels with its item). This is
the line tool's heuristic, now with exact boundaries — strictly better. A cheap
explode-time tiling check (`concat(chunks) == original`) catches a segmenter
byte-drop; the *behavior* proof is the per-item rustfmt-canonical equivalence at
Prove (§3.1).

### Alternatives and the pivot criterion

| Option | Verdict |
| --- | --- |
| **`syn` + proc-macro2 spans** (recommended) | Best Rust analysis (typed refs, structured `use`), pure Rust, lowest effort. Rust-only. |
| **`tree-sitter` (+ grammars)** | True lossless CST, **multi-language** (Rust + TS `grip` + Python `taut` — the whole stack with one tool). But untyped nodes → adjacency/import analysis is name-matching + per-language queries (more work, fuzzier), and a C dependency. **Pivot here when the goal becomes one splitter for all three languages.** |
| **`ra_ap_syntax` / `rowan`** (rust-analyzer's CST) | Most faithful Rust CST + error-resilient, but unstable published API tracking RA internals, heavy, low-level. Overkill for green-file batch splitting. |

Honest framing: `syn` is AST+spans, not a literal CST. With the proof at the
AST/CST-canonical level (§3.1) and bodies byte-sliced + rustfmt'd, that's the
*right* tool on valid Rust, not a compromise — comments survive the slice, the
proof is comment-sensitive, and analysis stays typed. The only genuine reason to
choose the pure-CST route (`tree-sitter`) is **polyglot reach**
across Gianni's stack — a real but not-yet-urgent goal. Recommend shipping the
`syn` Rust splitter now and keeping the analysis layer backend-agnostic so a
`tree-sitter` backend can be added later without rewriting the clustering/import
logic.

## 4. Tool design (phases → implementation)

A standalone bin (or `cargo xtask`), working name `gwz-split`, manifest-driven so
the human keeps the Classify step:

```
gwz-split explode <file.rs> --out /tmp/split/<file>/
   → chunk-000..NNN (byte-identical slices) + manifest.toml
   → asserts: concat(chunks) == original (byte-for-byte)

# human edits the `dest` column in manifest.toml (advisory grouping pre-filled)

gwz-split reassemble --manifest /tmp/split/<file>/manifest.toml
   → writes dest module files (chunks per dest + merged `use` header)
   → updates the parent `mod` declarations
   → leaves residual `pub(crate)`/imports to `cargo build` + `cargo fix`
```

**Manifest row** (the analysis surface):

```
idx  name                kind   lines      loc  dest         refs(siblings)        uses
12   handle_materialize  fn     560-621    62   materialize  [materialized_response, par_map_per_host, read_lock_or_empty, ...]  [crate::artifact, par_map_per_host, ...]
```

- **Explode**: parse → byte-range items → gap-slice → lossless assert → emit
  manifest with name/kind/span/LOC + **adjacency** (refs to siblings) + **uses**
  (imports each item actually references).
- **Classify**: tool pre-fills `dest` with a greedy grouping (callers next to
  callees; widely-referenced helpers → `*_common`) under a `--max-loc 500`
  budget — **advisory only**; the human edits `dest`. No forced clustering.
- **Reassemble**: per `dest`, place the item bodies (byte-sliced from the
  original) and prepend a **merged import header** computed from the union of
  `uses` of that dest's items (deduped/folded via `UseTree`); cross-dest refs
  become `use crate::<dest>::Item;`. Update `mod` decls in the parent, then
  `rustfmt` each dest file.
- **Prove**: (1) **AST-canonical equivalence** — for every moved item,
  `rustfmt(before) == rustfmt(after)` (comment-sensitive, formatting-independent,
  §3.1); (2) the tiered carve-out gate (`cargo test -p <crate> --lib` + the
  carve-out binaries) for imports/visibility/resolution. Both green ⇒ done.

### The two explicitly requested capabilities

- **Adjacency (which symbols cross-reference which).** `syn::visit` builds the
  reference graph per item; the manifest exposes it and the greedy classifier uses
  it to *suggest* groupings (callers beside callees). This is the mechanized form
  of the cross-reference processing used during the manual splits.
- **Import pulling/merging.** Each item's referenced paths determine which `use`s
  it needs; per destination we emit the **union**, deduped and folded into clean
  grouped `use` blocks, dropping imports no item in that module uses. Over-
  inclusion (name-collision heuristic) is safe — it degrades to an unused-import
  warning that `cargo fix` prunes; under-inclusion is caught by the compiler.
  (Full name resolution is rust-analyzer territory and unnecessary: the compiler
  is the backstop, exactly as the methodology already assumes.)

## 5. Effort evaluation

Calibration up front: this is **commodity engineering**, no algorithmic novelty.
`syn`, `proc-macro2` spans, and `syn::visit` are well-trodden; the only fiddly
bits are byte-range mapping across proc-macro2 versions and the import-subset
heuristic (both de-risked by the rustfmt-canonical proof + `cargo fix`).

| Component | Effort |
| --- | --- |
| **Tier 1 — Explode + manifest** (parse, byte-range gap-slice, tiling check, name/kind/span/LOC) | ~2 days |
| Adjacency graph (`syn::visit` refs → manifest + advisory greedy grouping) | ~1 day |
| **Tier 2 — Import pull/merge** (referenced-path → per-dest `use` union/dedup) | ~2 days |
| Reassemble (per-dest body placement + merged header + `mod`-decl + rustfmt) | ~1 day |
| AST-canonical proof harness (`rustfmt(before)==rustfmt(after)` per item) | ~0.5 day |
| CLI/manifest UX, gate wiring, tests, dogfood on `workspace_ops` | ~1–2 days |
| **v1 total** (Rust/`syn`, human-classified, compiler-driven visibility) | **~6.5–7.5 days** |
| *Optional later:* auto `pub(crate)` visibility bump | +1–2 days |
| *Optional later:* `tree-sitter` backend for polyglot (per language) | +3–5 days |

### Build-vs-manual (the honest case)

Splitting the **5 files now**, manually with the existing line tool +
compiler-driven fixups, is roughly **1–2 days total**. So the tool is **not**
justified by these five files alone — v1 costs more than the work it replaces
this round. The case for building it rests on:

1. **Reuse** — every future split is near-free, and god files recur.
2. **Churn/error reduction** — the line scanner's special-case tail (§2) is a real
   defect source, worst on the 4175-LOC `workspace_ops` where a miscut is most
   likely and most expensive.
3. **Fit with the north star** — a splitter is the *generate-don't-hand-write*
   principle applied to refactoring mechanics; mechanical scaffolding is exactly
   what Gianni's methodology says to automate.

### Recommendation

- **Build Tier 1 first (~2 days)** — the `syn` explode + adjacency manifest +
  lossless assert. It immediately replaces the fragile scanner, gives accurate
  chunks + adjacency for all 5 splits, and de-risks the rest.
- **Dogfood Tier 1 on `workspace_ops`** (the 4175-LOC elephant) — highest accuracy
  payoff. Do `main.rs` either by hand or with Tier 1 in parallel.
- **Decide Tier 2 (import merge) after dogfooding** — build it only if import
  shuffling proves to be the dominant remaining churn (it likely is for the
  big-`use` files).
- **Keep classification human-owned** and **visibility compiler-driven** in v1.
- **Hold `tree-sitter`** until splitting the TS/Python codebases with one tool
  becomes a goal; keep the analysis layer backend-agnostic so that pivot is cheap.

### Net

A ~2-day Tier-1 MVP is a clear win (cheap, immediately useful, de-risks
`workspace_ops`); the full ~6–7-day v1 is justified by reuse + churn reduction,
not by this round's five files. Recommend Tier 1 now, Tier 2 on evidence.

## 6. Revision R1 — dispositions of Review-55 (GPT-5.5)

Review-55 approved the AST-normalizing direction and the parser-backed manifest,
and correctly flagged two over-claims: rustfmt-canonical per-item equality is
**not** a complete behavior proof, and `syn`-based import pulling/merging is the
risky part that should not be in the first splitter. Every point holds (no
push-back this round — `syn` is syntax, not name resolution, and I conflated
"the item text survived" with "the move preserved behavior"). Dispositions:

| # | Finding | Disposition | Change |
| --- | --- | --- | --- |
| P1 | Name the proof change as a **methodology revision** | Accept | §1's "not changing" is wrong; this retires byte-identity deliberately |
| P1 | rustfmt-canonical ≠ behavior proof | Accept | Two distinct gates (below) |
| P1 | `syn` refs are **hints**, not precise references | Accept | Manifest column `adjacency_hint`; advisory-only, never a correctness input |
| P1 | Import pull/merge under-specified & risky | Accept | Reassemble **over-includes**; minimization → deferred Tier 3 |
| P1 | `crate::<dest>::Item` rewriting oversimplified | Accept | Module **topology** explicit in the manifest |
| P2 | Span/boundary needs a proof spike | Accept | Tier 1A boundary-golden tests |
| P2 | Nested-item splits uncovered | Accept | v1 **refuses** nested splits; blocks stay intact |
| P2 | Gate too narrow for cfg/workspace | Accept | Broadened final gate matrix |
| P2 | Timing vs audit architecture | Accept | Tier 1 now; reassemble/minimize after AD1/AD2 |

### Corrected proof model (supersedes §1 step 4 and §3.1's "behavior-preserving")

Two **distinct** gates, named accurately:

- **Move-integrity check (per item):** `rustfmt(before) == rustfmt(after)`.
  Proves the *tool didn't alter the item* during the move (no dropped comment, no
  mangled body), modulo formatting. It is **not** a behavior proof: moving an item
  changes its name-resolution environment — unqualified sibling names, trait-method
  imports (which have *no* path reference for `syn` to see), macros, `super::`/
  `self::`, and cfg-gated/glob/aliased imports can all resolve differently, so
  identical text can mean something else or fail to compile in the new module.
- **Behavior proof:** the **compiler + tests** over the supported feature/target
  configs (gate matrix below). The compiler re-resolves every name in the new
  context — that is what proves the move is semantics-preserving.

So: rustfmt-canonical equality = "the item wasn't mangled"; compiler/tests = "it
still means the same thing." This is a deliberate **methodology revision** (byte
-identity retired, replaced by these two gates), not a mechanization of the old
byte-identical rule — §1's "the methodology is not changing" is corrected.

### Import handling (supersedes the Tier-2 union/dedup claim)

`syn` cannot reliably compute an item's needed imports (trait-method imports have
no path reference; `#[macro_use]`/macros, globs, aliases, cfg-gated, and
derive/inference-only imports are opaque). Under-inclusion breaks compilation —
and only for the *active* cfg. Therefore:

- **Reassemble (Tier 2) over-includes:** copy the original import header into each
  destination module (or keep it in the parent and re-export). No minimization.
- **Caveat:** over-inclusion yields unused-import warnings → under `clippy -D
  warnings` (which this repo uses) it *fails*. So a transitional module carries
  `#![allow(unused_imports)]`, or a single diff-reviewed prune follows. `cargo
  fix` is **not** a proof step (it changes more than imports) — optional and
  diff-reviewed only.
- **Minimization is Tier 3,** earning its way in after dogfooding, gated by the
  full cfg matrix + a diff review.

### Module topology in the manifest (supersedes `crate::<dest>::Item`)

The generic `crate::<dest>::Item` rule misgenerates for nested modules and
binaries. The manifest carries: source module path, destination module path,
parent module path, whether the parent re-exports moved items, and the import
style (`super::`, `crate::…`, or parent re-export). `workspace_ops/*` children
reference siblings via `super::common::X`; `main.rs` is a *bin* root;
`tests/local_workflows.rs` is a separate integration crate.

### Span/boundary proof spike (Tier 1A)

Before the tool depends on `proc_macro2` spans, prove + golden-test: outer
doc/attrs are inside the item span; inner attrs / module `//!` are not
misassigned; leading free `//` blocks attach to the intended *following* item;
the blank-line cut is stable at file start and around grouped comments;
`macro_rules!` / cfg-gated items give usable ranges; CRLF / non-ASCII / tabs map
line·col→byte correctly. (The tiling check catches byte *drops*, not a comment
attached to the *wrong* item.)

### Nested-item splits: out of scope for v1

The tool slices **top-level items only**. Splitting *inside* `impl` blocks,
`#[cfg(test)] mod tests`, macro-generated modules, `macro_rules!`, or large
inline modules is **not** supported — each stays intact as one chunk. (Fine for
the gwz files: `workspace_ops` is mostly top-level fns; a big test mod becomes one
chunk, split by hand if needed.) Nested extraction is a separate future design.

### Gate matrix (final acceptance; keep the fast carve-out for iteration)

`cargo fmt --check` · `cargo test --locked` for the split crate · `--all-targets
--locked` when feasible · feature gates (at least default + no-default-features
where relevant) · explicit binary tests for `gwz-cli` · integration tests when
splitting `tests/local_workflows.rs`. The fast `-p <crate> --lib` + carve-out
binaries stays the *per-iteration* gate; the broad matrix is the *accept* gate.

### Revised tiers + timing (supersedes §5's tier table)

- **Tier 1A — Safe Explode** (~2 days): parse, top-level items, gap-slice,
  manifest (name/kind/span/LOC + `adjacency_hint`), boundary-golden tests, tiling
  assert. **No** dest files, **no** import rewrite. *Build now.*
- **Tier 1B — Assisted Manual Split** (~0.5–1 day): the manifest drives the
  existing O(n) manual split — human assigns `dest`; tool prints a move plan,
  warns on high-adjacency helpers + large modules, emits a review checklist. No
  auto-reassemble. *Build now.*
- **Tier 2 — AST-Normalized Reassemble** (~2 days): body chunks moved from source
  slices then rustfmt-normalized; imports **over-included**/preserved; explicit
  parent `mod` topology; per-item move-integrity check before the compiler/test
  gate. *After the audit AD1/AD2 seams are decided* (they reshape `workspace_ops`).
- **Tier 3 — Import Minimize** (later): merge/dedup, optional diff-reviewed `cargo
  fix`, broad gate. *Earns in after dogfooding.*

Timing aligns with the audit plan: Tier 1A/1B now (useful for `main.rs` and as
*advisory* for `workspace_ops`); Tier 2 reassemble waits on the architecture
decisions that reshape the `workspace_ops` seams.

### Net

The high-value, low-risk deliverable is the parser-backed manifest (Tier 1A/1B).
Reassemble is fine in over-inclusion mode (Tier 2). Import minimization is the
risky part — deferred and broadly gated. The plan no longer claims a complete
behavior proof from `syn` + rustfmt.

## 7. Tier 1A Test Plan (TDD — write these RED first)

`explode` is not "ready to implement" until these exist and fail. Core surface:
`explode(src: &str) -> Result<Exploded>` where `Exploded { chunks: Vec<Chunk>,
manifest: Manifest }`, `Chunk { byte_range, text }`, manifest rows carry
`name · kind · span · loc · adjacency_hint`. Three test layers.

### 7.1 Invariant — lossless tiling (frozen real fixtures + optional live dogfood)

`rust-split` is a **separate repo** from glial-dev, so its tests cannot depend on
live gwz paths, and live files change (no stable golden). So freeze copies — but
**whole files, not excerpts**: the tiling check is a *property*, not a golden, and
more real code = more failure exposure for zero cost.

- **Frozen real-source corpus (in-repo) — whole files.** Copy real `.rs` from
  gwz-core/gwz-cli **in full** into `rust-split/tests/fixtures/real/` —
  the **whole god files** (`workspace_ops/mod.rs` 4175, `main.rs`, `status`,
  `operation`, `git`), the smaller files (`cbor`, `convert`, `workspace`), ideally
  the **entire set** of committed `.rs`. Don't trim to "representative" slices:
  trimming throws away the exact rare pattern (a `macro_rules!`, a weird attr, a
  deeply nested `impl`) that would expose a bug. The more, the better.
- **Tiling/structural invariants run on the WHOLE of every file — no golden
  needed.** For each: chunk ranges are contiguous, non-overlapping, cover
  `0..src.len()`, `concat(chunk.text) == src` **byte-for-byte**, no panic,
  deterministic. Size-independent and zero-maintenance — a 4175-LOC file asserts
  as trivially as a 40-LOC one. This is the headline regression; give it
  everything.
- **Golden manifest snapshots are the *secondary*, precision layer** — reserve
  `insta` goldens for the synthetic boundary cases (§7.2) and a few small real
  files where exact output is the point. Don't golden a 4175-LOC manifest (review
  churn, no extra signal over the property). For the god files, assert *targeted*
  facts instead (the `#[cfg(test)] mod` is one chunk; item count; a known item's
  span) — not a full snapshot.
- **Keep it fresh:** a `refresh-fixtures` script re-copies the gwz `.rs` set so the
  frozen corpus doesn't rot; the optional **live dogfood** (colocated glial-dev,
  tiling-only, no golden) catches drift between refreshes.
- License note: gwz is GPL-2.0-only, so copied fixtures carry it — fine for an
  internal/same-author tool; just set `rust-split`'s license deliberately.

### 7.2 Boundary-golden cases (Review-55's list + more)

Each is a small fixture asserting **which chunk owns a given line** (the tiling
check alone can't catch a comment attached to the *wrong* item). Pin:

1. `///` doc + `#[attr]` before a `fn` → owned by that fn. *(spike: confirmed)*
2. `//!` / `#![…]` inner attrs → file-level, owned by **no** item chunk. *(spike: confirmed)*
3. Free `//` block before an item → owned by the **following** item. *(spike: confirmed)*
4. Blank-line cut rule: `fn a` `\n\n` `// c` `\n` `fn b` → trailing blank stays
   with `a`, `// c` goes with `b`. **Pin the exact rule** (cut at the last blank
   line of the gap).
5. File start: leading license/`//` header before the first item → owned by the
   first chunk (or a synthetic preamble chunk — decide + pin).
6. File end: trailing comment / final newline after the last item → owned by the
   last chunk.
7. Comment touching the item vs separated by a blank line (grouping).
8. `macro_rules! m { … }` → one chunk, usable range.
9. `#[cfg(feature = "x")] fn` → attr inside the chunk.
10. `impl Trait for Type { fn … }` → **one** chunk (nested fns NOT split),
    synthetic name `impl Trait for Type`. *(spike: parses, no name — confirmed)*
11. `#[cfg(test)] mod tests { … }` → **one** chunk (not split).
12. Multi-line `/** */` doc and `/* */` block comment.
13. **CRLF** input → tiling holds, ranges correct.
14. **Non-ASCII** (unicode idents/strings/comments) → no panic; slices land on
    char boundaries.
15. Consecutive items with no blank line between them.
16. Degenerate: empty file · comments-only · `//!`-only · `use`-only.

### 7.3 Manifest content (unit)

- Name + kind for **every** `syn::Item` variant (fn, struct, enum, union, trait,
  trait-alias, type, const, static, mod, use, macro, `macro_rules`, impl, extern
  crate, foreign mod); synthetic names for impl/foreign-mod.
- `loc` per chunk; `&src[span]` matches the item body.
- `adjacency_hint`: item referencing a sibling name → edge; referencing nothing →
  none; **a local variable shadowing a sibling name STILL produces an edge** —
  this test locks the advisory contract (the graph is a *syntactic hint*, not
  precise; the greedy grouping must never depend on it for correctness).

### 7.4 Failure & determinism

- Unparseable input → `Err`, **no panic** (we only split green files, but fail
  cleanly).
- Same input → identical manifest + chunks (golden snapshots are stable; no
  clock/random in output).

### 7.5 Explicitly NOT tested at Tier 1A

No reassembly, no import rewriting, no compile/behavior proof — those are Tier 2
(the move-integrity check + the §6 gate matrix). Tier 1A proves exactly four
things: items are correctly identified, chunks tile losslessly, comments attach
to the right item, and the manifest is accurate.

Mechanics: golden manifests via `insta` snapshots (or checked-in `*.expected`);
the corpus + tiling properties are programmatic.

## 8. Implementation kickoff (Tier 1A)

This section makes the plan self-sufficient as the build brief: hand an
implementer this doc — §6 for the tiers, §7 for the tests, §8 for the rest.
Work TDD-first (write §7's tests RED, then implement to green).

### 8.1 Where

- **Tool:** `/Users/owebeeone/limbo/rust-split` — a standalone git repo with a
  clap skeleton already in place (`Cargo.toml`, `src/main.rs`; edition 2024,
  rust-version 1.95).
- **Corpus + eventual split target (read-only):**
  `/Users/owebeeone/limbo/glial-dev/gwz-core` and `/gwz-cli`. Never modify gwz
  source — copy files into `rust-split/tests/fixtures/real/`, don't edit in place.

### 8.2 Verified foundation (spike, Rust 1.95 stable — replicate as the first test)

The parser approach is proven; don't re-litigate it, lock it with a test:

- Deps: `syn = { version = "2", features = ["full","parsing","visit"] }`,
  `proc-macro2 = { version = "1", features = ["span-locations"] }`, dev-dep
  `insta`. (Likely cached; build `--offline` if needed.)
- `item.span().byte_range()` returns the item's exact byte `Range<usize>` on
  stable. **Doc comments + attributes are inside the item span**; **free `//`
  comments fall in the gaps** between items; `//!`/`#![…]` are `file.attrs`, not
  items. **Move text by byte-slicing the original — never re-emit from the tree**
  (that is why a pure-AST tool suffices; comments only travel through the slice).

### 8.3 Core surface

- `explode(src: &str) -> Result<Exploded>` → `Exploded { chunks: Vec<Chunk>,
  manifest: Manifest }`, `Chunk { byte_range, text }`, manifest rows
  `name · kind · span · loc · adjacency_hint`.
- CLI: `rust-split explode <file.rs> --out <dir>` → chunk files + `manifest.toml`,
  asserting `concat(chunks) == original`.

### 8.4 Scope guardrail (DO / DO NOT)

- **DO:** explode → tiling chunks + manifest (§6 Tier 1A, §7 tests). Top-level
  items **only**. `adjacency_hint` is an advisory syntactic hint, never a
  correctness input.
- **DO NOT** (blocked on the audit AD1/AD2 decisions — building now risks the
  wrong seams): reassembly, destination-module writing, import
  pulling/merging/minimizing, `mod`-decl rewriting, `rustfmt`/compile/behavior
  proof, name resolution. Nested splits (inside `impl` / `#[cfg(test)] mod` /
  `macro_rules!`) are out of scope — each stays **one chunk**.

### 8.5 Definition of done + gate

- Tiling invariant green on the **whole** gwz corpus (§7.1); boundary-goldens
  (§7.2); manifest units (§7.3); deterministic; no panic on unparseable input.
- `cargo test` + `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings`
  all green.
- **License:** `rust-split` has none set; copying GPL-2.0-only gwz source as
  fixtures requires choosing a GPL-compatible license first (§7.1).

### 8.6 Guardrails

- gwz source is **read-only** (corpus only).
- **Stop at Tier 1A** — do not build Tier 2/3 (they wait on AD1/AD2).
- A natural review checkpoint: land the tiling layer (§7.1 corpus green) before
  the boundary-goldens (§7.2).
- Commit/push per the user's direction.
