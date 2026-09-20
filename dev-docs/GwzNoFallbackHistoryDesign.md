# No-fallback history characterization and design input

Accepted **L4-A characterization and design input only**, reviewed at core
`c63f497df29d51ad5d864738fa0056b513c3ab7d` after
[Code GO](../../dev-docs/GwzNoFallbackCharacterization-ReviewCode.md).
Replacement design/implementation and activation remain separate gates.

Date: 2026-09-20
Package: L4-A history characterization/design
Status: G0 focused characterization passed; the H1 extension is pending its coordinated run and the replacement design remains pending.

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

The recorded three-test pass below is the G0 baseline. The H1 extension is
intentionally not included in that result and remains pending the coordinated
run.

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

## H1 direct-member and bare-repository follow-up

The H1 extension remains test-only and is implemented in the existing child
module. It adds no wiring, visibility, production, dependency, protocol, or
CLI changes. Each new test re-executes the exact fully-qualified test in a
clean child with `env_clear`, isolated HOME/XDG/global/system Git config,
fixed identity and locale, and a unique temporary root. The root is removed
after child output is captured, and the parent asserts that exactly one test
reported `... ok`. Windows child fixtures use `D:/gwz-tests/<unique>`.

### H-MEMBER

The fixture creates an active `app` member with the existing committed
attribute history. Workspace-root pathspecs are routed directly to the member:

| Form | Repository pathspec | Native IDs | Current cursor IDs |
| --- | --- | --- | --- |
| `attr:gwz-path` | `:(attr:gwz-path)src` | `[head, base]` | `[]` |
| `attr:-gwz-path` | `:(attr:-gwz-path)src` | `[base]` | `[]` |
| `attr=blue` | `:(attr:gwz-path=blue)src` | `[base]` | `[]` |
| `attr:!gwz-path` | `:(attr:!gwz-path)src` | `[]` | `[head, base]` |

Each row asserts that only `mem_app` is selected, the full magic envelope is
preserved while `app/` is removed, and the current sequence is checked
separately from the native member oracle. Git directory bytes and the member
`.gitattributes` bytes are snapshotted after fixture construction and must be
unchanged after the complete row set.

Executed with C1 after the source drafts were stable. From core:
`cargo +1.95.0 test --locked --lib characterization -- --nocapture`.
All 17 selected tests passed (two H1, three existing L4-A, seven C1 and five
existing L3-A), including exact child runs; none ignored. Rustfmt and changed-
range whitespace checks pass. Current H1 code is 471 total lines, a 325-line
addition with two baseline lines replaced, within the 400-added-line ceiling.
Only macOS arm64 / Git 2.52.0 ran. Status: Code review pending.

### H-BARE and H-INFO

The same committed-attribute fixture is cloned into a bare `mem_bare` member.
The native oracle uses `git --git-dir <bare> rev-list`. Exact results:

| Row | Positive attribute | Unspecified attribute | Current cursor |
|---|---|---|---|
| H-BARE: committed attributes only | `[]` | `[head, base]` | Matches each native vector |
| H-INFO: info/attributes sets src/unset, unsets src/set and gives src/value a value | `[base]` | `[]` | Matches each native vector |

Committed `.gitattributes` supplies no worktree attributes in this bare fixture;
repository-local info attributes are consulted. The first draft expected the
head in H-INFO too, but that commit changes only src/set, now unset, so Git
correctly selects only base for the positive query. The executed test pins
those exact native/current vectors; a future refusal is an assertion failure,
not silently accepted as equivalent behavior. Bare bytes and info attributes
are unchanged across each read group; only fixture setup writes the override.

These rows characterize direct member routing only. They do not freeze positive
attribute fan-out from a workspace-root `.` pathspec, where current routing can
synthesize `.` and lose the magic envelope for member plans. That remains a
separate design row before a `gwz-git` path-history API is frozen.
