//! **The plugin's contract**, in a headless `App`: the strips bake on `Startup` into whatever asset
//! stores exist, and an app that has none of them gets an empty atlas rather than a crash.
//!
//! Bevy 0.19's rule is that a missing `Res<T>` panics the system rather than skipping it, so the
//! "works without a PBR plugin" claim is only true if every store the bake touches is an `Option`.
//! The smallest app that can hold an `Image` is `TaskPoolPlugin` + `AssetPlugin` + `init_asset::<Image>`
//! — no render device, no window, no material — which is the first case; the second drops even that.

use bevy::app::{App, TaskPoolPlugin};
use bevy::asset::{AssetApp, AssetPlugin, Assets};
use bevy::image::Image;
use bevy_carnage::cross_section::{CrossSectionAtlas, CrossSectionPlugin, Region, strip, Layers, CrossSectionSettings};

#[test]
fn the_plugin_bakes_one_strip_per_region_without_a_renderer() {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default(), CrossSectionPlugin));
    app.init_asset::<Image>();
    app.update();

    let atlas = app.world().resource::<CrossSectionAtlas>();
    assert_eq!(atlas.regions().count(), Region::ALL.len(), "one strip per region");
    let images = app.world().resource::<Assets<Image>>();
    let settings = app.world().resource::<CrossSectionSettings>();
    let tile_mm = settings.scale.tile_units * settings.scale.mm_per_unit;
    for region in Region::ALL {
        let Some(s) = atlas.get(region) else { panic!("{region:?} has no strip") };
        let region = &region;
        assert!(images.get(&s.albedo).is_some(), "{region:?} albedo was not added");
        assert!(images.get(&s.rough).is_some(), "{region:?} roughness was not added");
        assert!(s.material.is_none(), "no PBR plugin, so no material");
        // The digest the atlas carries is the strip at the settings' tile — the one `annotate_cap`
        // maps `v` onto — not at the depth axis' own resolution.
        let want = strip(&Layers::for_region(*region), settings.width, settings.height, tile_mm, settings.seed).digest();
        assert_eq!(s.digest, want, "{region:?} baked at a different tile than the scale names");
    }
}

#[test]
fn an_app_with_no_image_store_gets_an_empty_atlas_not_a_panic() {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default(), CrossSectionPlugin));
    app.update();
    assert_eq!(app.world().resource::<CrossSectionAtlas>().regions().count(), 0);
}
