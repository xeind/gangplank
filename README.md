# gangplank

The bridge from a GPUI `Render` impl to a shipped app. Two crates:

- `gangplank` — hooks. Pirates have hooks.
- `cargo-gangplank` — `cargo gangplank bundle [--release]` turns a GPUI binary into a signed macOS `.app` that Finder will hand files to.

macOS only for now.

```toml
gangplank = "0.1"                                              # gpui-ce (default)
gangplank = { version = "0.1", default-features = false, features = ["zed"] }  # Zed's gpui crate
```

Your app's `gpui` must be the same crate and version gangplank links, or entity types will not match. The library tracks crates.io releases (`gpui-ce` 0.3, `gpui` 0.2); a git-pinned gpui needs a path dependency on gangplank with the pin changed to match.

## Hooks

Each is called during render, identified by its source location, and returns an entity the view reads. `use_keyed_*` variants take an explicit id for lists.

| Hook | Replaces | Read |
|---|---|---|
| `use_resource(window, cx, key, async fn)` | spawn + generation counter + stale-result check | `Loading` / `Ready(T)` |
| `use_debounce(window, cx, value, delay)` | timer you reset on every keystroke | settled `T` |
| `use_persisted(window, cx, path, default)` | read/write a JSON prefs file by hand | `T`, `.set(v, cx)` |
| `use_interval(window, cx, period)` | a spawn-loop with a timer | `.ticks()` |
| `use_keyboard(window, cx).bind("cmd-k", h)` | `actions!` + keymap JSON + focus handle | `.attach(div())` on the root |
| `use_clipboard(window, cx)` | write_to_clipboard + a "Copied!" timer | `.copy(text, cx)`, `.copied()`, `.read(cx)` |
| `use_previous(window, cx, value)` | a `last_value` field on the view | `Option<T>` from the last render |
| `use_file_watch(window, cx, path, period)` | a polling thread + channel | `.version()`, bumps on change |

```rust
let text = use_debounce(window, cx, self.query.clone(), Duration::from_millis(250));
let hits = use_resource(window, cx, text.read(cx).value().clone(), |q| search(q));
```

## Bundle

```toml
[package.metadata.gangplank]
name = "CsvGrid"
identifier = "com.xein.csvgrid"
icon = "assets/icon.png"
file-types = ["public.comma-separated-values-text"]
url-schemes = ["csvgrid"]
agent = false   # true for LSUIElement panel apps
# For release builds. Make the profile once: xcrun notarytool store-credentials gangplank
sign = "Developer ID Application: Your Name (TEAMID)"
notarize-profile = "gangplank"
```

```sh
cargo install --path crates/cargo-gangplank
cargo gangplank bundle              # target/debug/<Name>.app
cargo gangplank run "my data.csv"   # bundle, then open it like Finder would
cargo gangplank dmg --release       # target/release/<Name>.dmg, drag-to-install; notarized if configured
```

Output lands in `target/<profile>/<Name>.app`, ad-hoc signed (or Developer ID when `sign` is set) and registered with Launch Services. Notarization is untested so far: no Developer ID on the machine it was written on. Report what breaks.
