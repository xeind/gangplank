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

`cargo run --example showcase` shows all of them in one window.

Each is called during render, identified by its source location, and returns an entity the view reads. `use_keyed_*` variants take an explicit id for lists.

| Hook | Replaces | Read |
|---|---|---|
| `use_resource(window, cx, key, async fn)` | spawn + generation counter + stale-result check | `Loading` / `Ready(T)` |
| `use_debounce(window, cx, value, delay)` | timer you reset on every keystroke | settled `T` |
| `use_persisted(window, cx, path, default)` | read/write a JSON prefs file by hand | `T`, `.set(v, cx)` |
| `use_interval(window, cx, period)` | a spawn-loop with a timer | `.ticks()` |
| `use_keyboard(window, cx).bind("cmd-k", h)` | `actions!` + keymap JSON + focus handle | `.attach(div())` on the root |
| `use_clipboard(window, cx, show_for)` | write_to_clipboard + a "Copied!" timer | `.copy(text, cx)`, `.copied()`, `.read(cx)` |
| `use_previous(window, cx, value)` | a `last_value` field on the view | `Option<T>` from the last render |
| `use_file_watch(window, cx, path, period)` | a polling thread + channel | `.version()`, bumps on change |
| `use_command(window, cx, prog, args, period)` | a spawn-loop that shells out | `Output` of the last run |
| `use_selection(window, cx)` + `selectable_text(id, text, &sel)` | hand-rolled drag/double-click selection on read-only text | selected `&str`, copy via cmd-c; single line |
| `use_text_input(window, cx)` + `text_input(&state, &focus)` | cursor, selection, IME handler and blink timer for a one-line field | `.text()`, `.selection()`; `on_submit` / `on_cancel` for Enter / Esc |
| `use_open_files(window, cx, handler)` with `OpenFiles::install(&app)` / `.ready(cx)` | `on_open_urls` + argv + an inbox for pre-window arrivals | handler gets `&[PathBuf]` |
| `use_window_state(window, cx, file)` + `saved_window_bounds(file)` | saving bounds on move/resize by hand | window reopens where it closed |
| `pick_file` / `pick_files` / `pick_save_path` (not a hook) | `cx.prompt_for_paths` plumbing | callback with `PathBuf` |

```rust
let text = use_debounce(window, cx, self.query.clone(), Duration::from_millis(250));
let hits = use_resource(window, cx, text.read(cx).value().clone(), |q| search(q));
```

## Bundle

```toml
[package.metadata.gangplank]
name = "Notes"
identifier = "com.example.notes"
icon = "assets/icon.png"
file-types = ["public.plain-text"]
url-schemes = ["notes"]
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

Output lands in `target/<profile>/<Name>.app`, ad-hoc signed (or Developer ID when `sign` is set) and registered with Launch Services. Notarization is untested. Report what breaks.
