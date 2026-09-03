//! **Loading mode changes the silhouette, and only when asked.**
//!
//! Sellán et al., *Breaking Good* (`doi:10.1145/3549540`, §6), state the limitation this closes: their
//! fault is *the same regardless of the directionality of the impact*, and they name uniaxial tension,
//! pure shear and torsion as the missing cases. So the first thing to prove is that the four modes
//! actually produce four different breaks — a `LoadingMode` that did not reach the geometry would
//! compile, pass every existing test, and be decoration.
//!
//! The second thing to prove is the opposite: that [`FaultPolicy::WeakAxis`] is **unmoved**. Every
//! frozen bake in this crate was taken under it, and `CutSettings::new` still selects it, so the
//! existence of the morphology arm must be invisible to a caller who does not ask for it.

use bevy::math::{Mat4, Vec3};
use bevy::mesh::Mesh;
use bevy::prelude::Cuboid;
use bevy_carnage::{
    CutSettings, FaultPolicy, FractureSettings, LoadingMode, ProxyCell, TissueClass,
    fracture_mesh, grady_mott_target,
};

/// A limb: twice as long as it is wide, so a long axis exists to twist and bend about.
///
/// Deliberately not a cube. Torsion and bending are *directional*, and a subject with no long axis
/// would make three of the four modes degenerate into each other — which would let the test pass for
/// the wrong reason.
fn limb() -> (Mesh, Vec<ProxyCell>) {
    (
        Mesh::from(Cuboid::new(0.16, 0.60, 0.16)),
        vec![ProxyCell::from_box(Vec3::ZERO, Vec3::new(0.08, 0.30, 0.08))],
    )
}

/// The subject's long axis. Both morphology modes measure against it.
const LONG_AXIS: Vec3 = Vec3::Y;

/// One seed for every bake below, so **the loading mode is the only variable**.
const SEED: u32 = 0x0BAD_F00D;

/// Fragment count and the quantised centroid of every leaf, folded into one number.
///
/// Quantised to a tenth of a millimetre before folding, because the claim being tested is that the
/// *silhouettes* differ — not that two float sums differ, which any change at all would produce.
fn digest(cut: &CutSettings) -> u64 {
    let (mesh, proxy) = limb();
    let bake = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, cut);
    let leaves = bake.into_leaves();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let fold = |h: &mut u64, v: u64| {
        *h ^= v;
        *h = h.wrapping_mul(0x0000_0100_0000_01B3);
    };
    fold(&mut h, leaves.len() as u64);
    for f in &leaves {
        // `center()` is the public accessor; the area-weighted `centroid()` is crate-private, and a
        // test reaching past the public surface would be testing a different crate than a consumer
        // gets.
        let c = f.cell.center();
        for x in [c.x, c.y, c.z] {
            fold(&mut h, ((x / 1.0e-4).round() as i64) as u64);
        }
    }
    h
}

/// A morphology policy for one mode, at an impulse well past the greenstick threshold.
fn morphology(mode: LoadingMode, tissue: TissueClass) -> CutSettings {
    CutSettings {
        fault: FaultPolicy::Morphology {
            mode,
            tissue,
            axis: LONG_AXIS,
            torque: 2.0,
            impulse: 60.0,
        },
        tissue,
        ..CutSettings::new(12, 0.05, SEED)
    }
}

/// **Four modes, four breaks.** If any two digests matched, the mode would not be reaching the
/// geometry.
#[test]
fn every_loading_mode_produces_a_different_break() {
    let modes = [
        ("Torsion", LoadingMode::Torsion),
        ("Bending", LoadingMode::Bending),
        ("Axial", LoadingMode::Axial),
        ("DirectHighEnergy", LoadingMode::DirectHighEnergy),
    ];
    let digests: Vec<(&str, u64)> = modes
        .iter()
        .map(|(name, mode)| (*name, digest(&morphology(*mode, TissueClass::Cortical))))
        .collect();

    for (i, (na, a)) in digests.iter().enumerate() {
        for (nb, b) in digests.iter().skip(i + 1) {
            assert_ne!(
                a, b,
                "{na} and {nb} produced the same fragment layout from the same seed — the loading \
                 mode is not reaching the cut planes"
            );
        }
    }
}

/// **`WeakAxis` is unmoved.** The digest is frozen here, and the reason it can be trusted as
/// "unchanged rather than merely recorded" is beside it.
///
/// This value was taken **after** the morphology arm landed, so on its own it would only pin the
/// present. What makes it evidence is that the three tests which *predate* the change — the bake's own
/// `fracture_output_is_bit_identical_across_runs`, the locked topology counts, and both `== 12`
/// fragment-count assertions — all passed **unchanged** across it. `choose_plane`'s weak-axis
/// arithmetic was lifted into `weak_axis_normal` byte for byte: same draws, same order, same tie rule.
///
/// If this moves, the direction-blind path moved, and that is a regression rather than a re-blessing.
#[test]
fn the_direction_blind_policy_is_frozen() {
    let cut = CutSettings::new(12, 0.05, SEED);
    assert_eq!(
        cut.fault,
        FaultPolicy::WeakAxis,
        "`CutSettings::new` must keep selecting the direction-blind policy, or every frozen bake in \
         this crate silently changed shape"
    );
    let got = digest(&cut);
    assert_eq!(
        got, 0x63c3_877f_9ce5_7f1f,
        "the weak-axis bake moved. Every frozen output in this crate was taken under this policy, \
         so this is a regression and not a table to update. The digest is 0x{got:016x}."
    );
}

/// **Greenstick: the tension cortex opens, the far cortex holds, and the bone stays bent.**
///
/// Not a fifth mode — an *outcome* of a bend below `greenstick_impulse`
/// (`doi:10.3390/jimaging11060187`). So the bake produces **one** fragment and reports a direction,
/// which is a thing no fragment count can express.
#[test]
fn a_gentle_bend_bends_rather_than_breaks() {
    let (mesh, proxy) = limb();
    let d = FractureSettings::default();
    let gentle = CutSettings {
        fault: FaultPolicy::Morphology {
            mode: LoadingMode::Bending,
            tissue: TissueClass::Cortical,
            axis: LONG_AXIS,
            torque: 0.0,
            // Half the threshold: firmly a greenstick.
            impulse: d.greenstick_impulse * 0.5,
        },
        ..CutSettings::new(12, 0.05, SEED)
    };
    let bake = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &gentle);
    assert_eq!(
        bake.into_leaves().len(),
        1,
        "a sub-threshold bend must leave the subject in one piece — that is what greenstick means"
    );

    let bake = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &gentle);
    assert!(
        bake.bent.length() > 0.0,
        "and it must report the residual bend, or a caller cannot tell a greenstick from a subject \
         that simply refused to break"
    );
    assert!(
        bake.bent.dot(LONG_AXIS).abs() < 1.0e-5,
        "the bend is across the long axis, not along it: {:?}",
        bake.bent
    );

    // And the same bend past the threshold does break, or the threshold is doing nothing.
    let hard = CutSettings {
        fault: FaultPolicy::Morphology {
            mode: LoadingMode::Bending,
            tissue: TissueClass::Cortical,
            axis: LONG_AXIS,
            torque: 0.0,
            impulse: d.greenstick_impulse * 4.0,
        },
        ..CutSettings::new(12, 0.05, SEED)
    };
    let broken = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &hard);
    assert!(
        broken.into_leaves().len() > 1,
        "a bend past the threshold must actually fault the bone"
    );
    let broken = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &hard);
    assert_eq!(
        broken.bent,
        Vec3::ZERO,
        "a subject that parted has no residual bend — reporting one would be two answers to one \
         question"
    );
}

/// **Cortical bone splinters.** At least one fragment must be long and thin, because cortical bone
/// fails at ~2 % strain and comes apart in shards rather than in lumps.
#[test]
fn cortical_bone_produces_a_splinter() {
    let cut = morphology(LoadingMode::Torsion, TissueClass::Cortical);
    let (mesh, proxy) = limb();
    let leaves = fracture_mesh(&[(&mesh, Mat4::IDENTITY)], &proxy, &cut).into_leaves();
    assert!(leaves.len() > 2, "precondition: the limb actually broke, got {}", leaves.len());

    let worst = leaves
        .iter()
        .map(|f| {
            // Extent along each axis of the cell's own bounds — a shard is long on one and short on
            // another, and the ratio is what "splinter" means.
            // Bounds from the cell's own points: `ProxyCell` exposes them, and a shard is long on
            // one axis and short on another.
            let pts = f.cell.points();
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for p in pts {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            let e = hi - lo;
            let long = e.x.max(e.y).max(e.z);
            let short = e.x.min(e.y).min(e.z);
            if short > 0.0 { long / short } else { f32::INFINITY }
        })
        .fold(0.0f32, f32::max);
    assert!(
        worst >= 4.0,
        "the longest fragment was only {worst:.2}x its own thickness — cortical bone must splinter, \
         not crumble"
    );
}

/// **Trabecular bone crushes rather than shattering.** It tolerates ~30 % strain, so the honest
/// failure is a shorter, denser bone: never more than [`bevy_carnage::TRABECULAR_MAX_PIECES`] pieces,
/// however hard it is hit.
#[test]
fn trabecular_bone_never_shatters() {
    let s = FractureSettings::default();
    // A volume, an energy and a strain rate spanning a stumble to a rifle round.
    for (energy, rate) in [(5.0f32, 20.0f32), (500.0, 400.0), (5000.0, 5000.0), (1.0e6, 1.0e5)] {
        let n = grady_mott_target(2.0e-4, energy, rate, TissueClass::Trabecular, &s);
        assert!(
            n <= bevy_carnage::TRABECULAR_MAX_PIECES,
            "trabecular bone produced {n} pieces at {energy} J and {rate} 1/s — it compacts, it does \
             not shatter"
        );
    }
}

/// **Fragment count follows the energy, not an artist constant.**
///
/// Grady's characteristic fragment size goes as `ε̇^(-2/3)` (`doi:10.1063/1.329934`), so a faster load
/// makes more pieces — and the energy delivered is a hard ceiling on how many surfaces can be
/// created at all. A pistol wedges; a rifle comminutes.
#[test]
fn a_faster_harder_load_makes_more_fragments() {
    let s = FractureSettings::default();
    let vol = 2.0e-4; // m³, about a humerus shaft
    let pistol = grady_mott_target(vol, 500.0, 300.0, TissueClass::Cortical, &s);
    let rifle = grady_mott_target(vol, 3500.0, 3000.0, TissueClass::Cortical, &s);
    assert!(
        rifle > pistol,
        "a rifle round must comminute where a pistol round wedges: {pistol} then {rifle}"
    );

    // The energy ceiling is real: the same strain rate with almost no energy behind it cannot create
    // the surface, so the count collapses to the floor.
    let starved = grady_mott_target(vol, 0.5, 3000.0, TissueClass::Cortical, &s);
    assert!(
        starved < rifle,
        "an energy-starved load must not comminute: {starved} against {rifle}"
    );
    assert!(starved >= s.min_pieces as usize, "and it must still stay inside the authored clamp");

    // Nonsense in, the floor out — never an invented count.
    for bad in [(0.0f32, 100.0f32), (100.0, 0.0), (f32::NAN, 100.0), (100.0, f32::INFINITY)] {
        let n = grady_mott_target(vol, bad.0, bad.1, TissueClass::Cortical, &s);
        assert_eq!(
            n, s.min_pieces as usize,
            "a blow nobody described must give the floor, not a guess: {bad:?} gave {n}"
        );
    }

    // Soft tissue tears: at the same blow it yields far fewer pieces than cortical bone.
    let flesh = grady_mott_target(vol, 3500.0, 3000.0, TissueClass::Soft, &s);
    assert!(flesh < rifle, "flesh must tear rather than comminute: {flesh} against {rifle}");
}

/// Two bakes of one policy agree, bit for bit. The morphology arm must not have introduced anything
/// order-dependent — the whole crate's contract, applied to the new path.
#[test]
fn every_policy_is_reproducible() {
    for mode in
        [LoadingMode::Torsion, LoadingMode::Bending, LoadingMode::Axial, LoadingMode::DirectHighEnergy]
    {
        for tissue in [TissueClass::Cortical, TissueClass::Trabecular, TissueClass::Soft] {
            let cut = morphology(mode, tissue);
            assert_eq!(
                digest(&cut),
                digest(&cut),
                "{mode:?} on {tissue:?} did not reproduce"
            );
        }
    }
}
