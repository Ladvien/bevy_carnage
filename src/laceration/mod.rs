#![doc = include_str!("../../docs/laceration.md")]

mod curve;
mod tear;

pub use curve::{ALONG_LANGER_FACTOR, Gape, Tension, anisotropy, gape};
pub use tear::{RAIL_WANDER, TearShape, Torn, WANDER_MM, digest, skin_patch, tear, tear_direction};

/// The three types that appear in this crate's own signatures — [`tear`] takes all three and
/// [`Laceration`] carries a [`Region`]. Re-exported so calling this crate does not oblige a caller to
/// name the crate underneath it.
pub use crate::cross_section::{Layers, Region, Scale};

use bevy::app::{App, Plugin, Update};
use bevy::asset::{Assets, Handle};
use bevy::ecs::component::Component;
use bevy::ecs::entity::Entity;
use bevy::ecs::hierarchy::ChildOf;
use bevy::ecs::query::With;
use bevy::ecs::resource::Resource;
use bevy::ecs::schedule::{IntoScheduleConfigs, SystemSet};
use bevy::ecs::system::{Commands, Query, Res, ResMut};
use bevy::log::warn_once;
use bevy::math::Vec3;
use bevy::mesh::{Mesh, Mesh3d};
use bevy::pbr::MeshMaterial3d;
use crate::cross_section::{CrossSectionAtlas, CrossSectionSettings};

/// **How much the gape has to move before the geometry is rebuilt**, in mesh units.
///
/// A tenth of a millimetre at metre scale. Below it a retear would spend a full mesh rebuild moving
/// vertices by less than a pixel, so [`LacerationPlugin`] skips it — which is why a wound that has
/// finished opening costs nothing at all per frame, forever.
pub const GAPE_EPSILON: f32 = 1.0e-4;

/// **A cut in a surface that is in the process of opening.**
///
/// Put it on the entity whose `Mesh3d` should show the wound, and give it `source`: the **intact**
/// mesh. Every retear starts from `source`, never from the last result, so the gape is a function of
/// the clock and nothing else — an accumulating edit would drift, and a drifting wound cannot be
/// hashed or rewound. `source` and the entity's own `Mesh3d` **must be different handles**; the
/// plugin refuses (once, loudly) rather than overwriting the intact copy.
///
/// [`Default`] exists for struct-update syntax — `Laceration { path, source, ..default() }` — and
/// authors nothing useful on its own: an empty path is refused by [`tear`].
#[derive(Component, Clone, Debug)]
pub struct Laceration {
    /// The cut, as a polyline in the mesh's own space. Two points is a straight slash; more points
    /// follow a curve. Sample it densely enough that the surface does not turn much between points.
    pub path: Vec<Vec3>,
    /// The surface normal along the cut. Defines "sideways": the lips part along
    /// `normal × segment_direction`, never along the normal.
    pub normal: Vec3,
    /// Final width and how long it takes to get there.
    pub gape: Gape,
    /// The skin's resting tension here, and which way its Langer lines run.
    pub tension: Tension,
    /// How far from the cut the displacement still reaches, in mesh units.
    pub influence: f32,
    /// How deep the wound bed's floor sits, in millimetres.
    pub bed_depth_mm: f32,
    /// Which anatomical thickness row the bed's bands come from.
    pub region: Region,
    /// The [`LacerationClock`] tick the wound was made on. Ticks before it read as `0`, so a
    /// laceration can be authored in advance and open on cue.
    pub opened_at: u32,
    /// The **intact** mesh every retear cuts from. Never written.
    pub source: Handle<Mesh>,
    /// The child entity carrying the wound bed. `None` until the plugin spawns it; the plugin
    /// respawns it if the entity goes away.
    pub bed: Option<Entity>,
    /// The gape the current geometry was built at. **Owned by the plugin** — the field is public only
    /// so the struct can be built with a literal. A negative value means "never torn", which is why
    /// [`Default`] sets `-1.0`: any real gape is at least [`GAPE_EPSILON`] away from it, so the first
    /// update always cuts.
    pub last_gape: f32,
}

impl Default for Laceration {
    fn default() -> Self {
        Self {
            path: Vec::new(),
            normal: Vec3::Y,
            gape: Gape::default(),
            tension: Tension::default(),
            influence: 0.02,
            bed_depth_mm: 6.0,
            region: Region::Limb,
            opened_at: 0,
            source: Handle::default(),
            bed: None,
            last_gape: -1.0,
        }
    }
}

/// **The child entity carrying a wound's bed**, so the plugin can find the trough it spawned without
/// storing a mesh handle in [`Laceration`] — and so a consumer can query, hide or restyle every bed
/// in the world with one filter.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LacerationBed;

/// **The tick count every gape is measured against.**
///
/// An integer tick, not a duration, and that is the whole point: a wound's width is then a pure
/// function of two `u32`s, so two machines at different frame rates reach the same geometry and a
/// test can freeze it. [`LacerationPlugin`] increments it once per `Update`; a caller who wants the
/// wound tied to a fixed simulation step should skip that plugin's schedule and drive this resource
/// itself.
///
/// Saturating, not wrapping: at 60 Hz `u32` is 828 days, and a wound that has been open that long
/// should stay open rather than snap shut.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LacerationClock(pub u32);

/// **Everything this crate registers, in one set**: the clock tick, then the retear.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LacerationSystems;

/// **Adds [`LacerationClock`] and opens every [`Laceration`] on `Update`.**
///
/// Two systems, chained, both in [`LacerationSystems`]: the clock advances, then wounds whose gape
/// moved by more than [`GAPE_EPSILON`] are re-cut from their intact source. Nothing else — no
/// materials are created, no meshes are loaded, and the bed's material is whatever
/// [`CrossSectionAtlas`] already baked.
pub struct LacerationPlugin;

impl Plugin for LacerationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LacerationClock>()
            .add_systems(Update, (advance_clock, retear_lacerations).chain().in_set(LacerationSystems));
    }
}

/// One tick per `Update`.
///
/// `Option<ResMut<..>>` even though this plugin inits the resource: in Bevy 0.19 a missing `Res<T>`
/// *panics* the system rather than skipping it, and a caller is free to wire
/// [`LacerationSystems`] into an app that never added this plugin.
fn advance_clock(clock: Option<ResMut<LacerationClock>>) {
    if let Some(mut clock) = clock {
        clock.0 = clock.0.saturating_add(1);
    }
}

/// **Re-cut every wound whose gape has moved.**
///
/// Always from [`Laceration::source`], never from the last result — see [`Laceration`] for why. The
/// skin goes back into the entity's own mesh handle; the bed goes into a child entity's, spawned on
/// the first tear and reused after.
///
/// Every resource is optional. `LacerationClock` this plugin owns, but the other three belong to
/// plugins this crate does not add — and `CrossSectionSettings` being absent is a *supported*
/// configuration, not an error: the bed then uses [`Layers::for_region`] and [`Scale::default`],
/// which are the same numbers `bevy_cross_section` would have inited.
///
/// **Query order decides nothing here.** Each entity writes only its own two mesh assets and its own
/// component, so the order the query hands them over cannot change what any of them ends up as.
fn retear_lacerations(
    clock: Option<Res<LacerationClock>>,
    settings: Option<Res<CrossSectionSettings>>,
    atlas: Option<Res<CrossSectionAtlas>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    mut lacerations: Query<(Entity, &mut Laceration, &Mesh3d)>,
    beds: Query<&Mesh3d, With<LacerationBed>>,
    mut commands: Commands,
) {
    let (Some(clock), Some(mut meshes)) = (clock, meshes) else {
        return;
    };
    for (entity, mut lac, drawn) in &mut lacerations {
        let ticks = clock.0.saturating_sub(lac.opened_at);
        let width = gape(ticks, &lac.gape, &lac.tension, tear_direction(&lac.path));
        if (width - lac.last_gape).abs() <= GAPE_EPSILON {
            continue;
        }
        if drawn.id() == lac.source.id() {
            warn_once!(
                "bevy_laceration: {entity} draws its own source mesh — retearing would destroy the intact copy. \
                 Give the entity `Mesh3d(meshes.add(intact.clone()))` and the component `source: meshes.add(intact)`."
            );
            continue;
        }

        let layers = settings.as_ref().map_or_else(|| Layers::for_region(lac.region), |s| *s.layers(lac.region));
        let scale = settings.as_ref().map_or_else(Scale::default, |s| s.scale);
        let shape = TearShape {
            // The gape is the full mouth; the kernel works from the distance to each lip.
            half_width: width * 0.5,
            influence: lac.influence,
            bed_depth_mm: lac.bed_depth_mm,
        };
        // The immutable borrow of the arena ends with the match, which is why the source is not
        // cloned: `Torn` owns its two meshes outright.
        let torn = match meshes.get(&lac.source) {
            Some(source) => tear(source, &lac.path, lac.normal, &shape, lac.region, &layers, &scale),
            None => {
                // Still loading, or the handle outlived its asset. Nothing said: this is the ordinary
                // state of a wound authored before its mesh finished loading, and it resolves itself.
                continue;
            }
        };
        let Some(torn) = torn else {
            // `tear` has already warned once about whatever it refused. `last_gape` is deliberately
            // left alone so a wound that becomes tearable later still gets cut.
            continue;
        };

        if meshes.insert(drawn.id(), torn.skin).is_err() {
            warn_once!("bevy_laceration: {entity}'s mesh handle is stale; the wound cannot be drawn");
            continue;
        }

        match lac.bed.and_then(|bed| beds.get(bed).ok()).map(|mesh| mesh.id()) {
            Some(id) => {
                if meshes.insert(id, torn.bed).is_err() {
                    warn_once!("bevy_laceration: {entity}'s wound bed handle is stale");
                }
            }
            None => {
                let handle = meshes.add(torn.bed);
                let mut bed = commands.spawn((Mesh3d(handle), LacerationBed, ChildOf(entity)));
                if let Some(material) = atlas.as_ref().and_then(|a| a.material(lac.region)) {
                    bed.insert(MeshMaterial3d(material));
                }
                lac.bed = Some(bed.id());
            }
        }
        lac.last_gape = width;
    }
}
