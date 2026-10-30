# Contributing

Thanks for helping Detour.

## Reporting a bug

Open an issue with the version, the Windows build, the game and the exact
steps. If the problem is in a measurement, say what the game itself showed.

## Code

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Keep user-facing text short and plain: one sentence per finding, no jargon.
