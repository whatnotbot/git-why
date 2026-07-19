# Contributing

## Before starting

- Use Git 2.15 or newer.
- Use Rust 1.85 or newer.
- Search existing issues before opening a new one.
- For behavior changes, describe the user problem and the smallest CLI contract that solves it.
- Report security issues privately as described in [SECURITY.md](SECURITY.md).

## Development workflow

Create a branch in your local checkout, make a focused change, and run:

```console
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
git diff --check
```

Integration tests should use temporary, real Git repositories. Keep fixtures small and deterministic; do not depend on a network connection, GitHub credentials, global Git identity, or the contributor's existing repositories.

## Pull requests

A useful pull request includes:

- the behavior and motivation;
- tests that fail without the change when behavior is affected;
- updated public documentation when the CLI or JSON schema changes;
- no unrelated formatting, dependency, or refactoring changes.

Compatibility matters. Treat human output, exit statuses, and `schema_version` as public interfaces. A schema-breaking JSON change requires an explicit schema-version decision.

By contributing, you agree that your contribution is licensed under the project's MIT License.
