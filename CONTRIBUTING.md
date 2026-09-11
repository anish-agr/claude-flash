# Contributing

## Layout

| Path | What it holds |
|---|---|
| `crates/flash-core` | The platform-independent core: policy engine, configuration, hook classification, flash rendering, the HTTP parser and the `settings.json` editor. No I/O and no `unsafe`. |
| `crates/claude-flash` | The `flash` CLI and the `flash-agent` background process: HTTP server, runtime, journal, and the Windows and macOS front ends. |
| `spec/scenarios` | The engine's behaviour, as JSON scenarios run by `crates/flash-core/tests/scenarios.rs`. |
| `docs` | Reference documentation. |

## Checks

Everything CI runs can be run locally:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

The integration tests in `crates/claude-flash/tests` start real agents in headless
mode on free ports, each with its own data directory, so they never touch an
installed copy.

The platform front ends only compile on their own platform. To type-check the
macOS code from Windows or Linux, add the target and run clippy against it:

```bash
rustup target add aarch64-apple-darwin
```

```bash
cargo clippy -p claude-flash --all-targets --target aarch64-apple-darwin -- -D warnings
```

## Changing behaviour

What the engine does with an event is specified by the scenarios in
`spec/scenarios`. A change in behaviour starts with a new or updated scenario that
fails, followed by the change that makes it pass. Each scenario lists hook events,
signals, controls and clock ticks, and the exact flashes, notifications and
suppressions each step must produce.

The test runner also checks that every outcome the engine can produce appears in
at least one scenario, so a new suppression reason or delivery channel needs a
scenario before the suite passes.

## Commits

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `ci:`, `chore:`.
