# Splitter Tool CST Plan Review 55

Status: review of `dev-docs/SplitterToolCstPlan.md`
Date: 2026-06-17
Reviewer: GPT-5.5

Verdict: approve the AST-normalizing direction, revise the proof and import
claims before building the full v1.

The plan has the right instinct: stop using a column-0 scanner for Rust item
boundaries, generate a manifest, keep classification human-owned, normalize via
rustfmt, and use the compiler as the final backstop. I am not treating
byte-identity as a requirement. The unsafe part is narrower: the plan overclaims
what `syn` can prove without name resolution and treats rustfmt-canonical item
equality as behavior-preservation proof. That makes the full import-pulling
scope riskier than the plan suggests.

## Findings

### P1: The plan should explicitly name the safety proof change

Reference: `dev-docs/SplitterToolCstPlan.md:12`,
`dev-docs/SplitterToolCstPlan.md:24-34`,
`dev-docs/SplitterToolCstPlan.md:57-72`

The local methodology in `CLAUDE.md` says moved code should remain byte-identical
and should not be rustfmt'd during the split. This plan intentionally retires
that rule and replaces it with rustfmt-canonical item equality. I agree that
bytes are not the important property; this is a valid direction.

The plan should still say plainly that this is a methodology revision, not that
the methodology is unchanged. It changes the review surface from "mechanical
move" to "tool normalizes formatting/imports and proves the normalized items
match." That can be fine, but the proof needs to be named accurately.

Required plan change: state this as a deliberate methodology revision, not a
mechanization of the old rule. Add an explicit gate:

- `rustfmt(before) == rustfmt(after)` is the item-normalization check.
- Compiler/tests are the behavior check.
- Import rewriting is separately gated because it changes the name-resolution
  environment.

### P1: Rustfmt-canonical per-item equality is not a behavior proof

Reference: `dev-docs/SplitterToolCstPlan.md:59-66`,
`dev-docs/SplitterToolCstPlan.md:176-179`

`rustfmt(item_before) == rustfmt(item_after)` proves that the moved item survived
modulo rustfmt. That is a useful AST/CST-normalization check. It does not prove
that the item means the same thing in the new module.

Moving an item changes lexical scope and name resolution:

- unqualified sibling names may now resolve through a different import
- trait methods may need trait imports in scope
- macro names may resolve differently
- `super::`, `self::`, and private module paths can change meaning
- cfg-gated imports can differ by target
- glob imports and aliases can mask or expose different symbols

The compiler/test gate catches most practical regressions and should be treated
as the actual behavior proof. The plan should avoid calling per-item
rustfmt-canonical equality a complete behavior proof by itself.

Recommended wording:

```text
Prove per-item normalized preservation with rustfmt-canonical equality. Prove
module behavior with compiler/tests over every supported feature/target
configuration.
```

### P1: `syn::visit` adjacency is useful but not "precise references"

Reference: `dev-docs/SplitterToolCstPlan.md:97-100`,
`dev-docs/SplitterToolCstPlan.md:183-186`

`syn` gives syntax, not name resolution. Intersecting paths/idents with sibling
item names will produce useful hints, but it is not precise:

- local variables, parameters, fields, enum variants, and type parameters can
  have the same names as sibling items
- method calls may depend on trait imports
- associated items may look like sibling refs
- macro invocations and generated code are mostly opaque
- `Self::x`, `super::x`, reexports, and glob imports need resolution context
- cfg-gated items produce different graphs per configuration

This is still valuable for advisory grouping, but the manifest should label the
graph `refs_syntax_hint` or `adjacency_hint`, not "precise references." The
greedy grouping must not depend on it as a correctness input.

### P1: Import pulling/merging is under-specified and likely the risky part

Reference: `dev-docs/SplitterToolCstPlan.md:101-103`,
`dev-docs/SplitterToolCstPlan.md:171-174`,
`dev-docs/SplitterToolCstPlan.md:187-193`

The plan says each item's referenced paths determine which `use`s it needs and
that per-destination imports can be unioned/deduped/folded. That is not reliable
with `syn` alone.

Hard cases:

- trait imports used only for method resolution
- macro imports and `#[macro_use]`
- glob imports
- aliases such as `use foo::Bar as Baz`
- cfg-gated imports
- imports used only in type inference or derive/helper attributes
- imports intentionally kept for public API/reexport behavior
- module-local prelude patterns

Over-inclusion is usually acceptable; under-inclusion is compiler-visible only
for the active configuration. `cargo fix` is also too broad as a plan step: it
can modify more than import pruning and is not a proof mechanism.

Recommended scope change:

- Tier 1 should not pull/merge imports.
- Tier 1 should preserve the original import header in the parent or copy an
  over-inclusive header into each destination module.
- Tier 2 can add import minimization after dogfooding, and only with
  cfg-preserving tests.
- Any automatic `cargo fix` should be optional and diff-reviewed, not part of
  the proof.

### P1: Parent/child module path rewriting is oversimplified

Reference: `dev-docs/SplitterToolCstPlan.md:171-175`

The plan says cross-destination references become `use crate::<dest>::Item;`.
That is wrong for common split shapes. If `workspace_ops/mod.rs` is split into
child modules, the paths are likely:

- `super::common::helper`
- `crate::workspace_ops::common::helper`
- `pub(crate) use common::helper` from the parent

For `main.rs`, the module root is different again. For integration tests, the
crate/module shape differs from library modules. A generic `crate::<dest>::Item`
rule will misgenerate imports for nested modules and binary crates.

Required plan change: make the module topology explicit in the manifest:

- source module path
- destination module path
- parent module path
- whether the parent reexports moved items
- import style: `super::`, `crate::`, or parent reexport

### P2: Span and chunk boundary assumptions need a proof spike

Reference: `dev-docs/SplitterToolCstPlan.md:91-96`,
`dev-docs/SplitterToolCstPlan.md:110-120`

The plan relies on `proc_macro2` spans to give exact item byte ranges. That
needs a small proof spike before the tool design depends on it.

Cases to verify:

- outer doc comments and ordinary attributes are included in the item span
- inner attributes and module-level `//!` docs are not misassigned
- leading free `//` blocks attach to the intended following item
- blank-line splitting is stable at file start and around grouped comments
- macro items, `macro_rules!`, and cfg-gated items produce usable ranges
- CRLF input, non-ASCII input, and tabs map correctly from line/column to bytes

The tiling check catches byte drops, but it does not catch comments attached to
the wrong item. Add golden tests for these boundary cases.

### P2: Macro-heavy and nested-item cases are not covered

Reference: `dev-docs/SplitterToolCstPlan.md:41-44`,
`dev-docs/SplitterToolCstPlan.md:97-100`

The plan mentions nested registration blocks from the old methodology, but the
new design only promises top-level item chunks. That does not solve cases where
the split target is inside:

- `impl` blocks
- `#[cfg(test)] mod tests`
- macro-generated registration modules
- `macro_rules!` definitions
- large inline modules

That is acceptable for a v1 if the tool explicitly refuses nested splits and
keeps nested blocks intact. The plan should say so. If nested splitting remains a
goal, `syn` item-level slicing is only Tier 1; nested item extraction is a
separate design.

### P2: The gate is too narrow for cfg and workspace realities

Reference: `dev-docs/SplitterToolCstPlan.md:176-179`

`cargo test -p <crate> --lib` plus carve-out binaries is a reasonable fast gate,
but it is not enough for a splitter proof. Moving imports and visibility often
breaks only under a feature, target, integration test, or binary path.

Recommended gate matrix:

- `cargo fmt --check`
- `cargo test --locked` for the split crate
- `cargo test --all-targets --locked` when feasible
- feature gates used by the crate, at least default and no-default where relevant
- explicit binary tests for `gwz-cli`
- integration tests when splitting `tests/local_workflows.rs`

The plan can still keep a fast carve-out gate, but the final proof should be the
broader crate gate before accepting the split.

### P2: Building this before the audit architecture decisions may be badly timed

Reference: `dev-docs/SplitterToolCstPlan.md:229-247`,
`dev-docs/GwzAuditResolutionPlan-Review55.md`

Tier 1 is cheap enough to justify if it is strictly explode/manifest/advisory.
The full v1 is not obviously the right next move while the audit resolution is
still deciding backend architecture and the root/member boundary. Those
decisions can change the module seams, especially around `workspace_ops`.

Recommended sequencing:

- Build only Tier 1 now if the team wants splitter assistance immediately.
- Do not build reassemble/import merge until the audit architecture seams are
  decided.
- Do not combine import rewriting with the first safety-critical split unless
  the compiler/test gate and review diff are scoped tightly enough to isolate
  name-resolution changes.

## Recommended Revised Scope

### Tier 1A: Safe Explode

Build this first:

- parse with `syn`
- identify top-level items
- gap-slice into chunks
- emit manifest with item name/kind/span/LOC
- emit syntax-only adjacency hints
- emit boundary-golden tests
- assert `concat(chunks) == original`

Do not write destination files or rewrite imports in this tier.

### Tier 1B: Assisted Manual Split

Use the manifest to drive the existing manual/O(n) process:

- human assigns `dest`
- tool can print a move plan
- tool can warn about high-adjacency helpers and large modules
- tool can generate review checklists

Still no automatic reassemble by default.

### Tier 2: AST-Normalized Reassemble

Add automatic reassemble in a conservative normalized mode:

- body chunks are moved from source slices, then rustfmt-normalized
- imports are over-included or preserved, not minimized
- parent `mod` declarations are explicit
- per-item rustfmt-canonical equality is checked before the compiler/test gate

### Tier 3: Import Minimize / Rustfmt Mode

Only after dogfooding:

- merge/dedup imports
- optionally run `cargo fix`
- require a broader test gate and a diff review

## Bottom Line

The plan should ship the parser-backed manifest. That is the high-value,
low-risk part.

The plan should not claim complete behavior proof from `syn` plus rustfmt, and
it should not automate import pulling/merging as part of the first splitter. Use
`syn` for better chunks and advisory graphs, allow AST-normalized reassembly,
and let import minimization earn its way in after dogfooding.
