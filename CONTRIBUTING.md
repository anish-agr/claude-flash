# Contributing

## Layout

| Path | What it holds |
|---|---|
| `crates/flash-core` | The platform-independent core: policy engine, configuration, hook classification, flash rendering, the HTTP parser and the `settings.json` editor. No I/O and no `unsafe`. |
| `crates/claude-flash` | The `flash` CLI and the `flash-agent` background process: HTTP server, runtime, journal, and the Windows and macOS front ends. |
| `spec/scenarios` | The engine's behaviour, as JSON scenarios run by `crates/flash-core/tests/scenarios.rs`. |
| `docs` | Reference documentation. |
| `packaging` | The Homebrew formula and the Scoop manifest. |
| `scripts/update-packages.sh` | Points both at a published release. |

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

## Releasing

1. Move the changelog's Unreleased entries under the new version and date, set
   `version` in `Cargo.toml`, and commit.
2. Tag the commit and push the tag. The release workflow builds the three archives,
   writes `SHA256SUMS` and publishes the release, with the changelog entry as its
   notes.

   ```bash
   git tag -a v2.2.0 -m "Claude Flash 2.2.0" && git push origin v2.2.0
   ```

3. Once the release is published, point the packages at it and commit the result:

   ```bash
   scripts/update-packages.sh 2.2.0
   ```

4. Copy `packaging/homebrew/claude-flash.rb` to `Formula/claude-flash.rb` in
   [anish-agr/homebrew-tap](https://github.com/anish-agr/homebrew-tap). Scoop reads
   the manifest straight from this repository.
