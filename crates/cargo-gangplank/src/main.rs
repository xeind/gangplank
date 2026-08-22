//! `cargo gangplank bundle` — build a GPUI binary into a macOS `.app`.
//!
//! gpui-ce ships no packaging story. Finder will not hand a file to a bare
//! binary; it needs a bundle with `CFBundleDocumentTypes`, and Gatekeeper
//! needs a signature even for a local ad-hoc one. This does the four steps
//! every GPUI app otherwise scripts by hand: build, lay out the bundle, write
//! the plist, sign, and tell Launch Services about it.
//!
//! Configure in the app's Cargo.toml:
//!
//! ```toml
//! [package.metadata.gangplank]
//! name = "CsvGrid"                     # bundle display name (default: package name)
//! identifier = "com.xein.csvgrid"      # required
//! file-types = ["public.comma-separated-values-text"]  # UTIs this app opens
//! url-schemes = ["csvgrid"]            # csvgrid://… links
//! agent = true                         # LSUIElement: no Dock icon (panel apps)
//! minimum-system-version = "11.0"
//! ```

use anyhow::{Context, Result, bail};
use cargo_metadata::{MetadataCommand, Package};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct Config {
    name: Option<String>,
    identifier: Option<String>,
    #[serde(default)]
    file_types: Vec<String>,
    #[serde(default)]
    url_schemes: Vec<String>,
    #[serde(default)]
    agent: bool,
    minimum_system_version: Option<String>,
}

fn main() -> Result<()> {
    // Invoked as `cargo gangplank bundle`: argv is [bin, "gangplank", "bundle", ...].
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("gangplank") {
        args.remove(0);
    }
    let release = args.iter().any(|a| a == "--release");
    match args.first().map(String::as_str) {
        Some("bundle") => bundle(release),
        _ => bail!("usage: cargo gangplank bundle [--release]"),
    }
}

fn bundle(release: bool) -> Result<()> {
    let metadata = MetadataCommand::new().exec().context("cargo metadata")?;
    let package = metadata
        .root_package()
        .context("run inside a package, not a bare workspace")?;
    let config: Config = match package.metadata.get("gangplank") {
        Some(value) => serde_json::from_value(value.clone())
            .context("[package.metadata.gangplank] is malformed")?,
        None => bail!("no [package.metadata.gangplank] table in Cargo.toml"),
    };
    let identifier = config
        .identifier
        .clone()
        .context("[package.metadata.gangplank] needs `identifier`")?;
    let bin = bin_name(package)?;
    let name = config.name.clone().unwrap_or_else(|| package.name.to_string());

    let profile = if release { "release" } else { "debug" };
    run(Command::new("cargo")
        .arg("build")
        .args(release.then_some("--release")))?;

    let app = metadata.target_directory.join(profile).join(format!("{name}.app"));
    let app = Path::new(app.as_str());
    let macos = app.join("Contents/MacOS");
    if app.exists() {
        fs::remove_dir_all(app)?;
    }
    fs::create_dir_all(&macos)?;
    fs::copy(
        metadata.target_directory.join(profile).join(&bin),
        macos.join(&bin),
    )
    .context("copy binary")?;
    fs::write(
        app.join("Contents/Info.plist"),
        plist(&config, &name, &identifier, &bin, &package.version.to_string()),
    )?;

    // Ad-hoc signature. Without it macOS kills the app on launch on Apple silicon.
    run(Command::new("codesign").args(["--force", "--sign", "-"]).arg(app))?;
    // Register now, not whenever Launch Services next rescans; otherwise the
    // document types can take minutes to appear.
    run(Command::new(LSREGISTER).arg("-f").arg(app))?;

    println!("built {}", app.display());
    Ok(())
}

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

fn bin_name(package: &Package) -> Result<String> {
    let mut bins = package.targets.iter().filter(|t| t.is_bin());
    let first = bins.next().context("package has no [[bin]] target")?;
    if bins.next().is_some() {
        bail!("package has more than one binary; not supported yet");
    }
    Ok(first.name.clone())
}

fn plist(config: &Config, name: &str, identifier: &str, bin: &str, version: &str) -> String {
    let mut out = String::new();
    out.push_str(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
"#,
    );
    let min = config.minimum_system_version.as_deref().unwrap_or("11.0");
    for (key, value) in [
        ("CFBundleName", name),
        ("CFBundleDisplayName", name),
        ("CFBundleIdentifier", identifier),
        ("CFBundleExecutable", bin),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleInfoDictionaryVersion", "6.0"),
        ("CFBundleVersion", "1"),
        ("CFBundleShortVersionString", version),
        ("LSMinimumSystemVersion", min),
    ] {
        string(&mut out, key, value);
    }
    out.push_str("\t<key>NSHighResolutionCapable</key>\n\t<true/>\n");
    if config.agent {
        out.push_str("\t<key>LSUIElement</key>\n\t<true/>\n");
    }
    if !config.file_types.is_empty() {
        // Alternate, not Owner: most of these are system UTIs owned by Apple,
        // and claiming Owner picks a fight with the default app for no gain.
        out.push_str("\t<key>CFBundleDocumentTypes</key>\n\t<array>\n\t\t<dict>\n");
        string_in(&mut out, 3, "CFBundleTypeName", &format!("{name} Document"));
        string_in(&mut out, 3, "CFBundleTypeRole", "Editor");
        string_in(&mut out, 3, "LSHandlerRank", "Alternate");
        out.push_str("\t\t\t<key>LSItemContentTypes</key>\n\t\t\t<array>\n");
        for uti in &config.file_types {
            out.push_str(&format!("\t\t\t\t<string>{}</string>\n", escape(uti)));
        }
        out.push_str("\t\t\t</array>\n\t\t</dict>\n\t</array>\n");
    }
    if !config.url_schemes.is_empty() {
        out.push_str("\t<key>CFBundleURLTypes</key>\n\t<array>\n\t\t<dict>\n");
        string_in(&mut out, 3, "CFBundleURLName", identifier);
        out.push_str("\t\t\t<key>CFBundleURLSchemes</key>\n\t\t\t<array>\n");
        for scheme in &config.url_schemes {
            out.push_str(&format!("\t\t\t\t<string>{}</string>\n", escape(scheme)));
        }
        out.push_str("\t\t\t</array>\n\t\t</dict>\n\t</array>\n");
    }
    out.push_str("</dict>\n</plist>\n");
    out
}

fn string(out: &mut String, key: &str, value: &str) {
    string_in(out, 1, key, value);
}

fn string_in(out: &mut String, depth: usize, key: &str, value: &str) {
    let tab = "\t".repeat(depth);
    out.push_str(&format!(
        "{tab}<key>{}</key>\n{tab}<string>{}</string>\n",
        escape(key),
        escape(value)
    ));
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("spawn {:?}", cmd.get_program()))?;
    if !status.success() {
        bail!("{:?} failed with {status}", cmd.get_program());
    }
    Ok(())
}

