# No-fallback history characterization and design input

Accepted **L4-A characterization and design input only**, reviewed at core
`c63f497df29d51ad5d864738fa0056b513c3ab7d` after
[Code GO](../../dev-docs/GwzNoFallbackCharacterization-ReviewCode.md).
Replacement design/implementation and activation remain separate gates.

Date: 2026-09-20
Package: L4-A history characterization/design
Status: focused characterization tests pass; the replacement design remains pending.

## Scope and ownership

This package owns only the focused test child
`src/operation/commit_log/path_characterization.rs` and this report. The test
child is declared by the integrator in `commit_log/tests.rs` and uses
`super::*` to reuse the existing private fixtures, request builder, and native
oracle helpers. No helper visibility, production module, protocol type, CLI
flag, dependency, or runtime behavior changed.

The characterization uses Git only as the existing fixture/oracle boundary.
It does not propose retaining a product subprocess route and does not claim a
replacement walk.

## Observed contract

| Row | Case | Evidence | Design consequence |
| --- | --- | --- | --- |
| H-ATTR-1 | `attr:`, `attr:-`, `attr=value`, `attr=value` mismatch, and `attr:!` | With fixture refs `head,base`, native/current are respectively `[head,base]/[]`, `[base]/[]`, `[base]/[]`, `[]/[]`, and `[]/[head,base]` | Preserve native attribute matching and unspecified semantics before selecting a pathspec implementation |
| H-ATTR-2 | Long `top` plus `attr:` envelope from workspace root | Envelope is retained and routed to the root target; native is `[head,base]`, current is `[]` for the same worktree-context reason | Reroot only the payload; retain every magic word and record the worktree-context requirement |
| H-ORD-1 | Path-limited merge history | Focused test compares exact sequence | Preserve native default order; do not timestamp-sort within a repository |
| H-RANGE-1 | `HEAD~3..HEAD` with path `p` | Focused test compares exact sequence | Apply pushes/hides before path simplification; retain range boundaries |
| H-FP-1 | Same path/range with `--first-parent` | Focused test compares exact sequence | First-parent is traversal semantics, not a post-filter |
| H-NM-1 | Same path/range with `--no-merges` | Focused test compares exact sequence | Keep the current post-read filter contract until a replacement proves parity |

The existing tests remain authoritative for root/member `.`, short and long
exclusions, shallow boundaries, promisor/no-lazy-fetch behavior, octopus and
three-dot merge-base handling where already covered. This package adds no new
ordering mode, rename-follow behavior, or full-history/simplify-merges option.

## Design input and pending rows

The replacement must model Git's path-sensitive TREESAME walk, including
parent rewriting and merge selection. Filtering ordinary revwalk output after
the fact cannot reproduce the observed cases. The implementation must accept
the existing ordered push/hide vectors, all three-dot merge bases, pathspec
envelopes, and first-parent mode without changing the request or output APIs.

Attribute matching is the remaining pathspec-specific design question. Git
evaluates attributes from the working tree for this command, while historical
tree contents determine commit/path changes. Before selecting `gix-pathspec`
or another implementation, characterize set, unset, value, and unspecified
attributes under member routing, bare repositories, and shallow/promisor
repositories. Preserve the accepted behavior or amend the authoritative design
documents before narrowing it.

This package deliberately characterizes root-only attribute routing. Member
fan-out and member-local attribute worktrees are pending rows for a later
bounded design package; they are not implied by the root result.

The current test matrix does not establish rename-follow behavior, topo/date
ordering, reflogs, notes/mailmap, `--all`, `--follow`, or other unexposed Git
options. They remain out of scope. A randomized generator is not justified by
this package: deterministic cases now cover the identified gap. If later
octopus/criss-cross, deletion/rename, shallow-boundary, or magic combinations
expose a gap, add a fixed seed/version, exact request/pathspec replay data, and
one deterministic regression through the existing test target before adding a
generator.

## Verification and budget

Focused command:

```text
cargo +1.95.0 test --locked -p gwz-core --lib operation::commit_log::tests::path_characterization
```

The first attempt stopped before tests because the integrator-wired sibling
module `src/git/gitbackend/commit_tag_characterization.rs` was not yet present.
After that handoff appeared, the same command passed: 3 focused tests passed,
2150 other tests were filtered, and the run took 0.26 seconds. Rustfmt 1.95.0
and `git diff --check` pass for both owned files.

Actual package budget is bounded to:

```text
production additions/changes: 0
production moves:             0
test lines:                   148 physical lines (conservative upper bound)
tool lines:                   0
documentation lines:          92 physical lines
files:                        2 owned lane files
protocol/dependency/flag deltas: 0
```

No runtime replacement, dependency activation, Git mutation, or shared-file
edit is part of this package.
