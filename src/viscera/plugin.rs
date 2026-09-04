//! The Bevy wiring: one resource, one system set, one system.

use bevy::app::{App, FixedUpdate, Plugin};
use bevy::prelude::{IntoScheduleConfigs, Query, Res, SystemSet};

use crate::viscera::settings::ViscSettings;
use crate::viscera::solver::step;
use crate::viscera::strand::{Mesentery, Strand};

/// **The set every [`Strand`] is advanced in, on `FixedUpdate`.**
///
/// Ordering hook for the caller: a system that rebuilds tube meshes goes `.after(VisceraSystems)`, and
/// a system that moves a mesentery's anchors goes `.before(VisceraSystems)`.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VisceraSystems;

/// **Adds [`ViscSettings`] and steps every [`Strand`] entity once per fixed tick.**
///
/// The plugin is a convenience, not the crate. [`crate::viscera::step`], [`crate::viscera::spill`] and
/// [`crate::viscera::tube_mesh`] are plain functions over plain data, so a headless search or an offline test
/// can drive the solver without an `App` at all — which is how `examples/rod_determinism.rs` runs.
///
/// It **never spawns**. Building an entity out of a strand is the caller's job, because a crate that
/// chose the material, the mesh handle and the parent would be unusable in any game that wanted
/// different ones.
pub struct VisceraPlugin;

impl Plugin for VisceraPlugin {
    fn build(&self, app: &mut App) {
        // `init_resource` here is what stops the reader below meeting Bevy 0.19's sharpest trap: a
        // missing `Res<T>` PANICS its system rather than skipping it.
        app.init_resource::<ViscSettings>()
            .configure_sets(FixedUpdate, VisceraSystems)
            .add_systems(FixedUpdate, step_viscera.in_set(VisceraSystems));
    }
}

/// Advance one strand per entity.
///
/// **ECS query order decides nothing here, and that is structural rather than sorted.** A strand reads
/// its own nodes, its own tether and the settings — never another strand — so there is no shared
/// accumulator, no budget to spend, and no last-writer-wins field. Two runs that visit the entities in
/// opposite orders produce identical digests, so the crate needs no `StrandOrder` component and no
/// canonical sort, and adding one would be a total order over nothing.
///
/// `Option<Res<_>>` rather than `Res<_>` because a resource this system panics without is a resource a
/// consumer could remove; the plugin above guarantees it exists, and this guarantees the guarantee.
fn step_viscera(
    settings: Option<Res<ViscSettings>>,
    mut strands: Query<(&mut Strand, Option<&mut Mesentery>)>,
) {
    let Some(settings) = settings else {
        return;
    };
    let settings = &*settings;
    for (mut strand, tether) in &mut strands {
        match tether {
            Some(mut tether) => step(
                core::slice::from_mut(&mut *strand),
                core::slice::from_mut(&mut *tether),
                settings,
            ),
            None => step(core::slice::from_mut(&mut *strand), &mut [], settings),
        }
    }
}
