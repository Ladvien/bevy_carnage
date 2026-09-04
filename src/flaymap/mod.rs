#![doc = include_str!("../../docs/flaymap.md")]

mod canvas;
mod digest;
mod settings;
mod uv;

pub use canvas::{FlayCanvas, Handoff, UV_SPAN_M};
pub use settings::FlaySettings;

/// The types that appear in this crate's public signatures. Re-exported because a caller should not
/// have to name the dependency to call [`FlayCanvas::new`] or read a [`Handoff`].
pub use crate::cross_section::{Layer, Layers, Region, Scale};

use bevy::app::{App, Plugin, Update};
use bevy::asset::Assets;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::Message;
use bevy::ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy::ecs::system::{Query, Res, ResMut};
use bevy::image::Image;
use bevy::math::{Vec2, Vec3};

/// **Bone has come through, once, on this entity.** The message form of [`Handoff::bone_reached`].
///
/// The crate owns the *type* and registers it, but writes it **never**: only the caller knows whether
/// the thing it just peeled is an actor whose skeleton something else owns, and a system in here that
/// wrote a message per canvas would be a schedule this crate has no business having. Forward a
/// [`Handoff`] whose `bone_reached` is set and the reader on the other side — a fracture spawner, a
/// bone-scrape sound, a wound-tier bump — gets it without either half naming the other.
///
/// It fires **once per canvas**, because bone is exposed once. See [`Handoff::bone_reached`] for why a
/// flag that stayed true would spawn a fracture proxy per shot for the rest of the fight.
#[derive(Message, Clone, Debug)]
pub struct BoneExposed {
    /// The entity whose [`FlayCanvas`] reached the cortex.
    pub entity: Entity,
    /// Where on that canvas, in its own UVs.
    pub uv: Vec2,
    /// Where in the mesh's own space, when the hit came from [`FlayCanvas::paint_world`] and so knows
    /// a point. `None` when it came from [`FlayCanvas::paint_uv`], which was handed a texture
    /// coordinate — a UV names a point on an atlas, and a seam maps one UV to several places on a
    /// body, so guessing would be worse than saying nothing.
    pub at: Option<Vec3>,
    /// The hit triangle's geometric normal, mesh-local, on the same terms as [`at`](Self::at).
    pub normal: Option<Vec3>,
}

impl BoneExposed {
    /// **The handoff, addressed to an entity.** `None` when this call was not the one that reached
    /// bone, so a caller writes `if let Some(msg) = BoneExposed::from_handoff(entity, &handoff)` and
    /// never has to remember which field gates which.
    pub fn from_handoff(entity: Entity, handoff: &Handoff) -> Option<Self> {
        let uv = handoff.first_bone_uv?;
        handoff.bone_reached.then_some(Self {
            entity,
            uv,
            at: handoff.at,
            normal: handoff.normal,
        })
    }
}

/// **Everything this crate registers, in one set.**
///
/// Exactly one system lives here: the upload budget. Painting and [`FlayCanvas::shade`] are the
/// caller's, because both need to know when a tick's last hit landed and this crate has no clock —
/// see the crate docs.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlaymapSystems;

/// **Adds [`FlaySettings`], registers [`BoneExposed`], and runs the per-frame upload budget on
/// `Update`.**
///
/// The plugin is deliberately thin. It does not paint, and it does not shade: a hit is gameplay state
/// and a tick number is gameplay state, and a crate that invented either would be reading a clock,
/// which is the one thing a hashable flaymap cannot do.
///
/// **The order the caller owns:** peel with [`FlayCanvas::paint_uv`] or
/// [`FlayCanvas::paint_world`], then call [`FlayCanvas::shade`] once after the last paint of the tick,
/// and this plugin uploads what changed on the next `Update`.
pub struct FlaymapPlugin;

impl Plugin for FlaymapPlugin {
    fn build(&self, app: &mut App) {
        // `init_resource` so a consumer that never authors settings still gets the shipped dials, and
        // so the system below cannot be the reason a missing `Res<T>` takes the app down.
        app.init_resource::<FlaySettings>()
            .add_message::<BoneExposed>()
            .add_systems(Update, upload_dirty_canvases.in_set(FlaymapSystems));
    }
}

/// Upload at most [`FlaySettings::max_canvas_updates_per_tick`] dirty canvases, oldest dirty first.
///
/// **Both resources are `Option`.** `FlaymapPlugin` inits [`FlaySettings`], but a missing `Res<T>` in
/// Bevy 0.19 *panics the system* rather than skipping it, and `Assets<Image>` belongs to a plugin this
/// crate does not add. Taking them as options means a consumer who wires the set into an app without
/// an asset arena gets nothing done rather than a crash.
///
/// Ordering is `(dirty_since, Entity)`. `dirty_since` is an integer tick, so it is reproducible;
/// `Entity` only ever breaks a tie between two canvases that went dirty on the same tick, and the only
/// thing that tie decides is which **write-only** image receives its bytes first. Query order decides
/// nothing here, which is the property this sort exists for.
fn upload_dirty_canvases(
    settings: Option<Res<FlaySettings>>,
    images: Option<ResMut<Assets<Image>>>,
    mut canvases: Query<(Entity, &mut FlayCanvas)>,
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
    // SORT-OK: `(dirty_since, Entity)` — the tick is total; the `Entity` tiebreak decides only which
    // write-only image is flushed first, and `digest()` folds the depth buffer, never the images.
    order.sort_unstable();

    for &(_, entity) in order.iter().take(budget) {
        if let Ok((_, mut canvas)) = canvases.get_mut(entity) {
            canvas.flush(&mut images);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::TaskPoolPlugin;
    use bevy::asset::AssetPlugin;
    use bevy::ecs::component::Component;
    use bevy::ecs::message::{MessageReader, MessageWriter};
    use bevy::ecs::resource::Resource;
    use bevy::image::ImagePlugin;
    use bevy::math::Vec2;

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            AssetPlugin::default(),
            ImagePlugin::default(),
            FlaymapPlugin,
        ));
        app
    }

    /// Which canvas is which, for the budget test.
    #[derive(Component)]
    struct Slot(u32);

    /// What the reader saw, so the assertion is about delivery rather than about a log line.
    #[derive(Resource, Default)]
    struct Heard(Vec<BoneExposed>);

    fn peel_to_bone(mut canvases: Query<(Entity, &mut FlayCanvas)>, mut out: MessageWriter<BoneExposed>) {
        for (entity, mut canvas) in &mut canvases {
            let handoff = canvas.paint_uv(Vec2::new(0.5, 0.5), 0.2, 40.0, 1);
            if let Some(msg) = BoneExposed::from_handoff(entity, &handoff) {
                out.write(msg);
            }
        }
    }

    fn hear(mut inbox: MessageReader<BoneExposed>, mut heard: ResMut<Heard>) {
        for msg in inbox.read() {
            heard.0.push(msg.clone());
        }
    }

    #[test]
    fn bone_exposed_message_reaches_a_reader() {
        let mut app = headless_app();
        app.init_resource::<Heard>();
        app.add_systems(Update, (peel_to_bone, hear.after(peel_to_bone)));

        let entity = app.world_mut().resource_scope(
            |world, mut images: bevy::ecs::change_detection::Mut<Assets<Image>>| {
                let canvas = FlayCanvas::new(
                    &mut images,
                    32,
                    Region::Limb,
                    Layers::for_region(Region::Limb),
                    [0.78, 0.66, 0.60],
                    0.55,
                );
                world.spawn(canvas).id()
            },
        );

        app.update();
        {
            let heard = app.world().get_resource::<Heard>().map(|h| h.0.clone()).unwrap_or_default();
            assert_eq!(heard.len(), 1, "the reader must have been handed exactly one exposure");
            let first = heard.first().expect("one message");
            assert_eq!(first.entity, entity);
            assert_eq!(first.uv, Vec2::new(0.5, 0.5));
            assert_eq!((first.at, first.normal), (None, None), "a UV paint knows no point");
        }

        // A second frame peels the same canvas deeper and says nothing: bone is exposed once.
        app.update();
        let heard = app.world().get_resource::<Heard>().map(|h| h.0.len()).unwrap_or(0);
        assert_eq!(heard, 1, "the handoff must not fire twice");
    }

    #[test]
    fn the_plugin_uploads_at_most_the_budget_oldest_dirty_first() {
        let mut app = headless_app();
        let s = FlaySettings::default();
        let budget = s.max_canvas_updates_per_tick;
        let total = budget + 2;

        app.world_mut().resource_scope(
            |world, mut images: bevy::ecs::change_detection::Mut<Assets<Image>>| {
                for slot in 0..total {
                    let mut canvas = FlayCanvas::new(
                        &mut images,
                        16,
                        Region::Torso,
                        Layers::for_region(Region::Torso),
                        [0.8, 0.7, 0.6],
                        0.5,
                    );
                    // Painted on tick `slot`, so `dirty_since` orders them and the sort has work.
                    canvas.paint_uv(Vec2::new(0.5, 0.5), 0.3, 4.0, slot);
                    canvas.shade(&s);
                    assert_eq!(canvas.dirty_since(), Some(slot));
                    world.spawn((canvas, Slot(slot)));
                }
            },
        );

        app.update();

        let mut still_dirty: Vec<u32> = app
            .world_mut()
            .query::<(&FlayCanvas, &Slot)>()
            .iter(app.world())
            .filter(|(canvas, _)| canvas.is_dirty())
            .map(|(_, slot)| slot.0)
            .collect();
        still_dirty.sort_unstable();
        assert_eq!(
            still_dirty,
            (budget..total).collect::<Vec<_>>(),
            "the budget uploaded the wrong canvases — oldest dirty must go first"
        );

        // A second frame clears the remainder.
        app.update();
        let dirty_after = app
            .world_mut()
            .query::<&FlayCanvas>()
            .iter(app.world())
            .filter(|canvas| canvas.is_dirty())
            .count();
        assert_eq!(dirty_after, 0, "the backlog never drained");
    }

    #[test]
    fn the_upload_system_survives_a_missing_settings_resource() {
        // Bevy 0.19 PANICS a system with a missing `Res<T>` rather than skipping it, so the system
        // takes an `Option` even though the plugin inits the resource. This is that guard, checked.
        let mut app = headless_app();
        assert!(
            app.world().get_resource::<FlaySettings>().is_some(),
            "the plugin did not init its dials"
        );
        app.world_mut().remove_resource::<FlaySettings>();
        app.update();
        app.update();
    }
}
