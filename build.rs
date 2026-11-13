//! Without the `editor` feature, bakes every file in `assets/` into the binary for
//! `src/assets.rs` to serve, and fails the build when a RON file there names an asset
//! file that doesn't exist. Editor builds skip both and read assets from disk, so saving
//! an asset never makes the next build recompile.

use std::{env, fs,
          path::{Path, PathBuf}};

const ASSETS_DIR: &str = "assets";
/// Extensions of asset files that RON files may name, by their path in `assets/`.
const NAMEABLE: [&str; 4] = ["png", "glb", "gltf", "ron"];

/// Every file in `dir` and its subfolders, leaving out hidden ones like `.gitkeep`.
fn files(dir: &Path) -> Vec<PathBuf> {
  fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .filter(|path| !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')))
    .flat_map(|path| if path.is_dir() { files(&path) } else { vec![path] })
    .collect()
}

/// Every string anywhere in `value`.
fn strings(value: &ron::Value) -> Vec<&str> {
  match value {
    ron::Value::String(string) => vec![string],
    ron::Value::Seq(values) => values.iter().flat_map(strings).collect(),
    ron::Value::Map(map) => {
      map.iter().flat_map(|(key, value)| [strings(key), strings(value)].concat()).collect()
    }
    ron::Value::Option(Some(value)) => strings(value),
    _ => vec![]
  }
}

/// The baked in files as Rust: a slice of (path in `assets/`, contents).
fn embedded() -> String {
  println!("cargo::rerun-if-changed={ASSETS_DIR}");
  let files = files(Path::new(ASSETS_DIR));
  let relative = |path: &Path| {
    path.strip_prefix(ASSETS_DIR).map_or_else(|_| path.into(), Path::to_path_buf)
  };

  let missing = files
    .iter()
    .filter(|path| path.extension().is_some_and(|extension| extension == "ron"))
    .flat_map(|path| {
      let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
      let value = ron::from_str::<ron::Value>(&text)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
      strings(&value)
        .into_iter()
        .filter(|name| {
          Path::new(name)
            .extension()
            .is_some_and(|extension| NAMEABLE.iter().any(|nameable| extension == *nameable))
        })
        .filter(|name| !Path::new(ASSETS_DIR).join(name).is_file())
        .map(|name| format!("{} names {name}, which isn't in {ASSETS_DIR}/", path.display()))
        .collect::<Vec<_>>()
    })
    .collect::<Vec<_>>();
  if !missing.is_empty() {
    panic!("Missing assets:\n{}", missing.join("\n"))
  }

  let root = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
  files
    .iter()
    .map(|path| {
      format!(
        "({:?}, include_bytes!({:?})),\n",
        relative(path).to_string_lossy().replace('\\', "/"),
        Path::new(&root).join(path)
      )
    })
    .collect()
}

fn main() {
  println!("cargo::rerun-if-changed=build.rs");
  let embedded =
    if env::var_os("CARGO_FEATURE_EDITOR").is_some() { String::new() } else { embedded() };
  let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
  fs::write(out.join("embedded_assets.rs"), format!("&[\n{embedded}]\n"))
    .expect("OUT_DIR is writable");
}
