//! The solver's dials, and the shipped value of every one of them.

use bevy::prelude::Resource;

/// Substeps per fixed tick. **Fixed, because a variable substep count is a variable result.**
pub const DEFAULT_SUBSTEPS: u32 = 4;
/// Constraint-projection passes per substep. Fixed for the same reason.
pub const DEFAULT_ITERATIONS: u32 = 8;
/// Downward acceleration, m/s². The value `bevy_carnage`'s examples already use, so guts and gibs
/// fall at one rate.
pub const DEFAULT_GRAVITY: f32 = 18.0;
/// Fraction of velocity shed per substep. Viscera are wet and heavy; they do not float.
pub const DEFAULT_DAMPING: f32 = 0.02;
/// XPBD compliance of a bowel segment, m/N. Near-inextensible.
pub const DEFAULT_COMPLIANCE_STRETCH: f32 = 1.0e-6;
/// XPBD compliance of the bending surrogate, m/N. Limp, not springy.
pub const DEFAULT_COMPLIANCE_BEND: f32 = 5.0e-4;
/// World height of the floor plane, metres.
pub const DEFAULT_FLOOR_Y: f32 = 0.0;
/// How many strands one [`crate::viscera::spill`] may produce.
pub const DEFAULT_MAX_STRANDS: u32 = 8;

/// **Every dial the solver reads.**
///
/// Registered by [`crate::viscera::VisceraPlugin`] with `init_resource`, so a consumer that adds the plugin
/// never meets the 0.19 trap where a missing `Res<T>` panics its system rather than skipping it.
///
/// The two counts are deliberately *not* tuned per call. A solver that spent more iterations on a
/// close-up strand than on a distant one would give a different answer for the same input, which is
/// the one thing this crate is for.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ViscSettings {
    /// Substeps per fixed tick. Clamped to at least 1 at use.
    pub substeps: u32,
    /// Constraint passes per substep. Clamped to at least 1 at use.
    pub iterations: u32,
    /// Downward acceleration, m/s².
    pub gravity: f32,
    /// Fraction of velocity shed per substep, `0.0..=1.0`.
    pub damping: f32,
    /// XPBD compliance of the stretch constraint, m/N. Zero is a rigid segment.
    pub compliance_stretch: f32,
    /// XPBD compliance of the bend constraint, m/N.
    pub compliance_bend: f32,
    /// World height of the floor plane. Nodes are clamped to `floor_y + radius`.
    pub floor_y: f32,
    /// Ceiling on [`crate::viscera::spill`]'s strand count.
    pub max_strands: u32,
}

impl Default for ViscSettings {
    fn default() -> Self {
        Self {
            substeps: DEFAULT_SUBSTEPS,
            iterations: DEFAULT_ITERATIONS,
            gravity: DEFAULT_GRAVITY,
            damping: DEFAULT_DAMPING,
            compliance_stretch: DEFAULT_COMPLIANCE_STRETCH,
            compliance_bend: DEFAULT_COMPLIANCE_BEND,
            floor_y: DEFAULT_FLOOR_Y,
            max_strands: DEFAULT_MAX_STRANDS,
        }
    }
}
