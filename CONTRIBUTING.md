# Contributing to rgrip

Thanks for your interest! Bug reports, ideas, docs fixes and code are all welcome.

## Getting set up

Requires Rust 1.88+.

```sh
git clone https://github.com/yoouTaro/rgrip
cd rgrip
cargo run -p rgrip
```

The workspace has two crates: `crates/core` (driver, SSH tunnel, SQL splitting, autocomplete, export, PostGIS decoding, edit SQL) and `crates/app` (egui UI). Keep logic that does not need a window in `core` so it can be tested.

## Tests

```sh
cargo test
```

- Unit tests run anywhere.
- `crates/core/tests/pg.rs` talks to a local Postgres on `localhost:5432` and is skipped if it is unreachable.
- `crates/core/tests/remote.rs` needs the dev environment (PostGIS with SSL and an SSH bastion):

```sh
./dev/gen-certs.sh
docker compose -f dev/docker-compose.yml up -d --build
cargo test
```

CI (`.github/workflows/ci.yml`) runs clippy and the tests on macOS for every push to `main` and every pull request, and runs the core tests against the dev environment on Linux.

## Code style

- Match the surrounding style. The code isn't rustfmt-formatted (it uses longer lines), so please don't reformat files you aren't otherwise changing.
- Don't add new `cargo clippy --workspace --all-targets` warnings.
- Match the surrounding code: naming, error handling, module layout.
- Comments explain why, not what.
- Add tests for behavior in `core`; for UI changes, include a screenshot or short recording in the PR.

## Pull requests

- Keep them focused; one change per PR is easier to review.
- Describe what and why, and how you tested it.
- Add a line under "Unreleased" in `CHANGELOG.md` for user-visible changes.
- For large changes, please open an issue first so we can agree on direction.

## AI-assisted contributions

rgrip is itself built with an AI coding agent, so AI-assisted PRs are welcome. The rule is simple: you are the author. Read the diff, run it, and test it before you open the PR, and be ready to discuss it in review.

## License

By contributing, you agree that your contributions are dual licensed under MIT OR Apache-2.0, without any additional terms or conditions.
