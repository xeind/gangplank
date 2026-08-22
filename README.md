# gangplank

The bridge from a GPUI `Render` impl to a shipped app. Two crates:

- `gangplank` — hooks. `use_resource` (async state keyed by its input), more to come. Pirates have hooks.
- `cargo-gangplank` — `cargo gangplank bundle [--release]` turns a GPUI binary into a signed macOS `.app` that Finder will hand files to.

macOS only for now. Targets gpui-ce.

## Bundle

```toml
[package.metadata.gangplank]
name = "CsvGrid"
identifier = "com.xein.csvgrid"
file-types = ["public.comma-separated-values-text"]
url-schemes = ["csvgrid"]
agent = false   # true for LSUIElement panel apps
```

```sh
cargo install --path crates/cargo-gangplank
cargo gangplank bundle
```

Output lands in `target/<profile>/<Name>.app`, ad-hoc signed and registered with Launch Services.
