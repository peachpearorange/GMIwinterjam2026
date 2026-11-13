//! The files in [`ASSETS_DIR`]. With the `editor` feature they're read from disk, so
//! edits show up without recompiling. Without it, `build.rs` bakes them into the binary
//! and they're served from there, to [`read`] and to Bevy's asset loading alike.

use {bevy::{asset::io::{AssetSourceBuilder, AssetSourceId, memory::MemoryAssetReader},
            prelude::*},
     std::{borrow::Cow, fs, path::Path}};

/// Directory assets live in, relative to the working directory.
pub const ASSETS_DIR: &str = "assets";

/// Every asset file as its path in [`ASSETS_DIR`] and its contents. Empty with the editor.
static EMBEDDED: &[(&str, &[u8])] =
  include!(concat!(env!("OUT_DIR"), "/embedded_assets.rs"));

/// The contents of the asset at `path`, relative to [`ASSETS_DIR`], like
/// `textures/snow.png`.
pub fn read(path: &str) -> Option<Cow<'static, [u8]>> {
  if cfg!(feature = "editor") {
    fs::read(Path::new(ASSETS_DIR).join(path)).ok().map(Cow::Owned)
  } else {
    EMBEDDED.iter().find(|&&(embedded, _)| embedded == path).map(|&(_, bytes)| bytes.into())
  }
}

/// Makes Bevy load assets from the baked in files when there's no editor. Add it before
/// `DefaultPlugins`.
pub fn plugin(app: &mut App) {
  if !cfg!(feature = "editor") {
    let reader = MemoryAssetReader::default();
    EMBEDDED.iter().for_each(|&(path, bytes)| reader.root.insert_asset(Path::new(path), bytes));
    app.register_asset_source(
      AssetSourceId::Default,
      AssetSourceBuilder::new(move || Box::new(reader.clone()))
    );
  }
}
