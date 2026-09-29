# AGENTS

Follow the root `AGENTS.md` rules.

- Work TDD-first: failing test, implementation, green tests, then refactor.
- Keep `gwz-core` independent of `gwz-cli`.
- Do not add behavior beyond what `GWZDesign.md` (authoritative) and `GWZRequirements.md` (baseline) specify; update them before expanding scope.
- Protocol payloads are taut-defined; do not create a shadow protocol.
- Build each new library as a small crate with a narrow API that builds and tests alone, under
  `crates/` (or `candidate-crates/` for crates only the candidate build compiles). Follow the
  [library boundaries](../dev-docs/GwzLocalCloneLibraryBoundaries.md) policy (LBT-001 to LBT-012)
  and the rules of the [core session crate map](../dev-docs/GwzCoreSessionCrateMap.md) §1: GWZ
  protocol encoding stays in core, secrets stay in core, and no crate depends on gwz-core.
- No global mutable state, and a thread-local counts as global state. Pass an explicit context;
  unique numbers come from a context's `IdSource`. The only exceptions are immutable data, caches
  of immutable data, and state a named dependency imposes.
