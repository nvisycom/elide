# Contributing

Thank you for your interest in contributing to Elide.

## Requirements

- Rust 1.95+ (stable)
- Cargo

## Setup

```bash
git clone https://github.com/nvisycom/elide.git
cd elide
cargo build
```

## Development

Run all CI checks locally before submitting a pull request:

```bash
cargo fmt --check     # formatting
cargo clippy          # lints
cargo test            # tests
```

To auto-fix formatting:

```bash
cargo fmt
```

### Benchmarks

The hot paths carry [Criterion](https://bheisler.github.io/criterion.rs/book/)
benchmarks: the end-to-end pipeline (`elide`, needs `engine,codec-txt`),
pattern detection (`elide-pattern`), and redaction selection/apply
(`elide-redaction`).

```bash
cargo bench --features engine,codec-txt         # run all
cargo bench -p elide-pattern --bench scan       # one suite

# Track regressions: save a baseline, then compare against it. Pass the pipeline
# features so the e2e suite is included in the baseline.
cargo bench --features engine,codec-txt -- --save-baseline main
cargo bench --features engine,codec-txt -- --baseline main   # % change vs `main`
```

## Pull Request Process

1. Fork the repository
2. Create a feature branch
3. Make changes with tests
4. Run `cargo fmt --check && cargo clippy && cargo test` to verify all checks
   pass
5. Submit a pull request

## Project Structure

The workspace uses Cargo workspaces. All crates live under `crates/`. See the
[CHANGELOG](CHANGELOG.md) for a description of each crate.

## Security

- Never commit secrets or API keys
- Use environment variables for configuration
- Validate all external inputs

## License

By contributing, you agree your contributions will be licensed under the Apache
License 2.0.
