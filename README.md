# Order Quote CLI (rust-app)

Practical app repo for the Rust polyrepo setup.

Uses `env-rust` and `materializer`.

The app is a CLI used by commerce teams to generate shipping quotes.

## Commands

```bash
# health payload
cargo run -- health

# quote payload
cargo run -- quote 12500 15 express --fragile
```

## Devenv helpers

```bash
devenv shell
quote-example
health
packaged-health
```

## Output for other repos

This repo exports:

- `outputs.order-quote-cli`
- `outputs.materialized_text` (merged instructions from import chain)

Other repos can run:

```bash
${config.outputs.order-quote-cli}/bin/order-quote-cli health
```

## Validate

```bash
devenv test
```
