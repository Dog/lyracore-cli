# Core contract ownership

The CLI should own checkout selection, local processes, command sequencing, confirmation,
and actionable failure reports. Core should own game-content and artifact format rules.

Two contracts still need a Core interface before their CLI copies can be removed:

- `src/cmd/packages/artifact.rs` and `script.rs` reproduce canonical artifact bytes and digests.
  Core's `lyracore-delta-check` validates and traces artifacts and exposes `--print-events`,
  but its current output is a human report, not a versioned metadata protocol. A replacement
  needs machine-readable canonical bytes, digests, package identities, counts and conflicts
  from the selected checkout. Only then can the CLI remove its parser/canonicalizer without
  parsing presentation text or compiling against an unrelated Core revision.
- `src/cmd/import.rs` owns detailed post-import SQL assertions for profiles, trainers, spells,
  quests and geometry. Core needs a profile-aware verification command that accepts an explicit
  server, database and credential source and returns a structured report with a failing exit
  status. `check_world_content` checks readiness; it does not replace those detailed import
  assertions. A full import script also cannot be substituted for a read-only verification step.

Keep these checks until replacements exist in the supported Core checkout. Do not maintain
both paths indefinitely: migrate their fixtures with the Core interface, switch the CLI to it,
and delete the duplicated rules. Keep credentials out of arguments and reports throughout.

The Loot Roll upgrade guard in `publish` also remains necessary while supported destinations
can carry the old schema. Remove it only when the supported upgrade floor excludes that state.

Skipping publication solely because all Rust Packages are disabled is also unsafe: an offline
disable creates no remote teardown record, so a target may still run that Package's code.
Keep the inventory-wide publication trigger until per-target compiled-module identity can prove
that disabled Rust has already been removed.
