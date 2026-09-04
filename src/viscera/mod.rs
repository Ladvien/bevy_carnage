#![doc = include_str!("../../docs/viscera.md")]

mod frame;
mod hash;
mod plugin;
mod settings;
mod solver;
mod spill;
mod strand;
mod tube;

pub use plugin::{VisceraPlugin, VisceraSystems};
pub use settings::{
    ViscSettings, DEFAULT_COMPLIANCE_BEND, DEFAULT_COMPLIANCE_STRETCH, DEFAULT_DAMPING,
    DEFAULT_FLOOR_Y, DEFAULT_GRAVITY, DEFAULT_ITERATIONS, DEFAULT_MAX_STRANDS, DEFAULT_SUBSTEPS,
};
pub use solver::{step, COMPLIANCE_MESENTERY, FIXED_DT, FIXED_HZ, MAX_ANCHORS};
pub use spill::{spill, SPILL_CONE, SPILL_RADIUS, SPILL_REST_LEN, SPILL_SEGMENTS};
pub use strand::{
    Mesentery, Strand, DEFAULT_TEAR_STRAIN, MAX_NODES, MAX_SEGMENTS, MIN_REST_LEN,
    STRAND_TEAR_STRAIN,
};
pub use tube::{tube_mesh, MAX_SIDES, MIN_SIDES};
