#![doc = include_str!("../../docs/wetmap.md")]

mod canvas;
mod digest;
mod settings;
mod uv;

pub use canvas::{UV_SPAN_M, WetCanvas};
pub use settings::WetSettings;

/// The stain silhouette [`WetCanvas::paint_uv`] and [`WetCanvas::paint_world`] take. Re-exported
/// because it appears in this crate's public signatures, and a caller should not have to name the
/// dependency to call the function.
pub use crate::bloodstain::stain::StainShape;

use bevy::app::{App, Plugin, Update};
use bevy::asset::Assets;
use bevy::ecs::entity::Entity;
use bevy::ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::image::Image;

/// **Everything this crate registers, in one set.**
///
/// Exactly one system lives here: the upload budget. Painting and [`WetCanvas::tick`] are the caller's,
/// because both need a tick counter and this crate has no clock — see the crate docs.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WetmapSystems;

/// **Adds [`WetSettings`] and the per-frame upload budget on `Update`.**
///
/// The plugin is deliberately thin. It does not tick canvases: a tick number is gameplay state, and a
/// crate that invented one would be reading a clock, which is the one thing a hashable wetmap cannot
/// do. Call [`WetCanvas::tick`] from your own fixed schedule *before* `Update`, and this plugin will
/// upload what changed.
pub struct WetmapPlugin;

impl Plugin for WetmapPlugin {
    fn build(&self, app: &mut App) {
        // `init_resource` so a consumer that never authors settings still gets the shipped dials, and
        // so the system below cannot be the reason a missing `Res<T>` takes the app down.
        app.init_resource::<WetSettings>()
            .add_systems(Update, upload_dirty_canvases.in_set(WetmapSystems));
    }
}

/// Upload at most [`WetSettings::max_canvas_updates_per_tick`] dirty canvases, oldest dirty first.
///
/// **Both resources are `Option`.** `WetmapPlugin` inits [`WetSettings`], but a missing `Res<T>` in
/// Bevy 0.19 *panics the system* rather than skipping it, and `Assets<Image>` belongs to a plugin this
/// crate does not add. Taking them as options means a consumer who wires the set into an app without an
/// asset arena gets nothing done rather than a crash.
///
/// Ordering is `(dirty_since, Entity)`. `dirty_since` is an integer tick, so it is reproducible;
/// `Entity` only ever breaks a tie between two canvases that went dirty on the same tick, and the only
/// thing that tie decides is which **write-only** image receives its bytes first. Query order decides
/// nothing here, which is the property this sort exists for.
fn upload_dirty_canvases(
    settings: Option<Res<WetSettings>>,
    images: Option<ResMut<Assets<Image>>>,
    mut canvases: Query<(Entity, &mut WetCanvas)>,
) {
    let (Some(settings), Some(mut images)) = (settings, images) else {
        return;
    };
    let budget = settings.max_canvas_updates_per_tick as usize;
    if budget == 0 {
        return;
    }

    let mut order: Vec<(u32, Entity)> = canvases
        .iter()
        .filter_map(|(entity, canvas)| canvas.dirty_since().map(|since| (since, entity)))
        .collect();
    if order.is_empty() {
        return;
    }
    order.sort_unstable();

    for &(_, entity) in order.iter().take(budget) {
        if let Ok((_, mut canvas)) = canvases.get_mut(entity) {
            canvas.flush(&mut images);
        }
    }
}
