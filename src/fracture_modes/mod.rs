#![doc = include_str!("../../docs/fracture_modes.md")]

mod graph;
mod impact;
mod linalg;
mod modes;

pub use graph::{CellGraph, Face, GraphError};
pub use impact::{Impact, Partition, SIGMA};
pub use modes::{BakeError, Mode, ModeSet, ModeSettings};

use bevy::app::{App, Plugin};
use bevy::ecs::resource::Resource;
use std::collections::HashMap;

/// **Baked mode sets, keyed by whatever the caller keys its subjects by.**
///
/// A `u64` rather than an asset id because this crate does not know what a subject is: a fracture
/// crate keys by the scene asset, an editor by a document id. The caller bakes with
/// [`ModeSet::bake`] once it has a [`CellGraph`] and stores the result here; nothing in this crate
/// decides *when* — the graph is the caller's, and so is the moment it is complete.
#[derive(Resource, Default, Debug)]
pub struct FractureModeCache {
    sets: HashMap<u64, ModeSet>,
}

impl FractureModeCache {
    /// Store a baked set under `key`, replacing any earlier one.
    pub fn insert(&mut self, key: u64, set: ModeSet) {
        self.sets.insert(key, set);
    }

    /// The set for `key`, if baked.
    pub fn get(&self, key: u64) -> Option<&ModeSet> {
        self.sets.get(&key)
    }

    /// Whether `key` has been baked.
    pub fn contains(&self, key: u64) -> bool {
        self.sets.contains_key(&key)
    }

    /// Forget `key`.
    pub fn remove(&mut self, key: u64) -> Option<ModeSet> {
        self.sets.remove(&key)
    }

    /// Bake `graph` with `settings` and store it under `key`. The error is the bake's.
    pub fn bake(&mut self, key: u64, graph: &CellGraph, settings: &ModeSettings) -> Result<&ModeSet, BakeError> {
        let set = ModeSet::bake(graph, settings)?;
        self.sets.insert(key, set);
        self.sets.get(&key).ok_or(BakeError::Singular)
    }

    /// How many sets are held.
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    /// True when nothing has been baked.
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

/// **Adds [`ModeSettings`] and [`FractureModeCache`]. Registers no systems.**
///
/// Deliberately: a bake needs a cell graph, and only the crate that owns a decomposition knows
/// when one is complete. A system here would either poll for a graph type this crate cannot name
/// or run on a schedule it does not own. The plugin's job is the two resources, so a consumer
/// takes them as `Res`/`ResMut` and never has to construct them.
pub struct FractureModesPlugin;

impl Plugin for FractureModesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ModeSettings>().init_resource::<FractureModeCache>();
    }
}
