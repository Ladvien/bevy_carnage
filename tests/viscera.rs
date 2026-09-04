//! **What this crate promises, checked.**
//!
//! Every test here defends a stated contract rather than an implementation detail: the digest is
//! reproducible, a tear never heals, the bowel does not stretch, the floor is solid, `spill` is a pure
//! function of its seed, the tube is the mesh it says it is, and the shipped defaults are the shipped
//! numbers.

use bevy::app::{App, FixedUpdate};
use bevy::math::Vec3;
use bevy::mesh::Mesh;
use bevy_carnage::viscera::{
    spill, step, tube_mesh, Mesentery, Strand, ViscSettings, VisceraPlugin, DEFAULT_COMPLIANCE_BEND,
    DEFAULT_COMPLIANCE_STRETCH, DEFAULT_DAMPING, DEFAULT_FLOOR_Y, DEFAULT_GRAVITY,
    DEFAULT_ITERATIONS, DEFAULT_MAX_STRANDS, DEFAULT_SUBSTEPS, DEFAULT_TEAR_STRAIN, MAX_NODES,
    SPILL_RADIUS, SPILL_REST_LEN, SPILL_SEGMENTS,
};

/// A wound above the floor, spilling forward and slightly up.
const WOUND: Vec3 = Vec3::new(0.1, 1.4, -0.2);
const EXIT: Vec3 = Vec3::new(0.35, 0.2, 1.0);
const SEED: u32 = 0x5EED_1234;

/// Tether the first few nodes of each strand back to where they left the body.
fn tether_all(strands: &[Strand], per_strand: usize) -> Vec<Mesentery> {
    strands
        .iter()
        .map(|s| {
            let anchors = s
                .nodes()
                .iter()
                .take(per_strand)
                .enumerate()
                .map(|(i, p)| (i as u32, *p))
                .collect::<Vec<_>>();
            let torn = vec![false; anchors.len()];
            Mesentery { anchors, tear_strain: DEFAULT_TEAR_STRAIN, torn }
        })
        .collect()
}

fn digests(strands: &[Strand]) -> Vec<u64> {
    strands.iter().map(Strand::digest).collect()
}

// ---------------------------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------------------------

#[test]
fn six_hundred_ticks_are_bit_identical_across_runs() {
    let s = ViscSettings::default();

    let run = || {
        let mut strands = spill(WOUND, EXIT, 6, SEED, &s);
        let mut mesentery = tether_all(&strands, 4);
        for _ in 0..600 {
            step(&mut strands, &mut mesentery, &s);
        }
        (digests(&strands), mesentery)
    };

    let (first, first_tears) = run();
    let (second, second_tears) = run();

    assert_eq!(first, second, "the same spill stepped the same number of ticks must digest the same");
    let tears_a: Vec<&Vec<bool>> = first_tears.iter().map(|m| &m.torn).collect();
    let tears_b: Vec<&Vec<bool>> = second_tears.iter().map(|m| &m.torn).collect();
    assert_eq!(tears_a, tears_b, "which mesenteric links tore must be reproducible too");
}

#[test]
fn the_slice_order_of_the_batch_changes_nothing() {
    // Strands never read each other, so reversing the batch must not move a single bit. This is the
    // property that lets the plugin's system ignore ECS query order entirely.
    let s = ViscSettings::default();
    let mut forward = spill(WOUND, EXIT, 5, SEED, &s);
    let mut forward_m = tether_all(&forward, 3);
    let mut reversed: Vec<Strand> = forward.iter().rev().cloned().collect();
    let mut reversed_m: Vec<Mesentery> = forward_m.iter().rev().cloned().collect();

    for _ in 0..240 {
        step(&mut forward, &mut forward_m, &s);
        step(&mut reversed, &mut reversed_m, &s);
    }

    let mut reversed_back = digests(&reversed);
    reversed_back.reverse();
    assert_eq!(digests(&forward), reversed_back);
}

#[test]
fn spill_is_a_pure_function_of_its_seed() {
    let s = ViscSettings::default();
    let a = spill(WOUND, EXIT, 6, SEED, &s);
    let b = spill(WOUND, EXIT, 6, SEED, &s);
    assert_eq!(digests(&a), digests(&b), "the same seed must lay the same strands");

    let c = spill(WOUND, EXIT, 6, SEED ^ 1, &s);
    assert_ne!(digests(&a), digests(&c), "a different seed must lay different strands");

    // Every strand is built with the documented shape, and the count is clamped, not honoured.
    let many = spill(WOUND, EXIT, 1_000, SEED, &s);
    assert_eq!(many.len() as u32, s.max_strands);
    for strand in &many {
        assert_eq!(strand.nodes().len(), SPILL_SEGMENTS as usize + 1);
        assert!((strand.radius() - SPILL_RADIUS).abs() < 1.0e-9);
        assert!(strand.nodes().len() <= MAX_NODES);
    }
}

// ---------------------------------------------------------------------------------------------
// The membrane
// ---------------------------------------------------------------------------------------------

/// One tether carrying a whole strand's weight over a bottomless drop: the load the membrane cannot
/// take up, which is the only way a monotone flag can be exercised at all.
fn hanging_by_one_thread() -> (Vec<Strand>, Vec<Mesentery>, ViscSettings) {
    let s = ViscSettings { floor_y: -1_000.0, ..Default::default() };
    let strands = vec![Strand::new(WOUND, Vec3::X, SPILL_SEGMENTS, SPILL_REST_LEN, SPILL_RADIUS)];
    let anchor = strands[0].nodes().first().copied().unwrap_or(Vec3::ZERO);
    let mesentery = vec![Mesentery { anchors: vec![(0, anchor)], ..Default::default() }];
    (strands, mesentery, s)
}

/// Tether one strand at every `stride`th node and fall for 600 ticks.
fn tethered_every(stride: usize) -> (usize, usize) {
    let s = ViscSettings::default();
    let mut strands =
        vec![Strand::new(Vec3::new(0.0, 1.4, 0.0), Vec3::X, SPILL_SEGMENTS, SPILL_REST_LEN, SPILL_RADIUS)];
    let anchors: Vec<(u32, Vec3)> = strands
        .iter()
        .flat_map(|st| st.nodes().iter().enumerate().collect::<Vec<_>>())
        .filter(|(n, _)| n % stride == 0)
        .map(|(n, p)| (n as u32, *p))
        .collect();
    let torn = vec![false; anchors.len()];
    let mut mesentery = vec![Mesentery { anchors, tear_strain: DEFAULT_TEAR_STRAIN, torn }];
    for _ in 0..600 {
        step(&mut strands, &mut mesentery, &s);
    }
    let m = mesentery.first().map(|m| (m.torn.iter().filter(|t| **t).count(), m.torn.len()));
    m.unwrap_or((0, 0))
}

#[test]
fn the_membrane_holds_a_dense_tether_and_parts_a_sparse_one() {
    // The whole point of `COMPLIANCE_MESENTERY`: a link carries about nine nodes of hanging weight.
    // If either half of this flips, the tear flag has stopped meaning anything — one way it never
    // fires, the other way it always does.
    let (dense_torn, dense_links) = tethered_every(4);
    assert!(dense_links >= 6);
    assert_eq!(dense_torn, 0, "a strand tethered every fourth node must hold");

    let (sparse_torn, sparse_links) = tethered_every(12);
    assert!(sparse_links >= 2);
    assert!(
        sparse_torn > 0,
        "a strand tethered every twelfth node hangs more weight off each link than a membrane takes"
    );
}

#[test]
fn a_bowel_segment_parts_too_when_the_load_beats_the_solver() {
    // `Strand::torn` is a live path, not decoration. Under the shipped near-inextensible compliance a
    // strand does not part under its own weight — which is correct — so this softens the stretch
    // constraint until the top segment cannot take the twenty-four nodes below it, and pins the head
    // with a tether strong enough not to go first.
    let s = ViscSettings { compliance_stretch: 1.0e-4, floor_y: -1_000.0, ..Default::default() };
    let mut strands =
        vec![Strand::new(WOUND, Vec3::X, SPILL_SEGMENTS, SPILL_REST_LEN, SPILL_RADIUS)];
    let head = strands[0].nodes().first().copied().unwrap_or(Vec3::ZERO);
    let mut mesentery =
        vec![Mesentery { anchors: vec![(0, head)], tear_strain: f32::MAX, torn: vec![false] }];

    let mut lengths = Vec::new();
    for _ in 0..600 {
        step(&mut strands, &mut mesentery, &s);
    }
    for pair in strands[0].nodes().windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        lengths.push((*b - *a).length());
    }
    let longest = lengths.iter().copied().fold(0.0f32, f32::max);
    assert!(
        longest > SPILL_REST_LEN * (1.0 + 0.6),
        "a parted segment must be free to run past its tear strain; longest was {longest}"
    );
    assert_eq!(mesentery[0].torn, vec![false], "the tether was meant to outlast the bowel here");
}

#[test]
fn an_overloaded_tether_tears() {
    let (mut strands, mut mesentery, s) = hanging_by_one_thread();
    for _ in 0..600 {
        step(&mut strands, &mut mesentery, &s);
    }
    assert_eq!(
        mesentery.first().map(|m| m.torn.as_slice()),
        Some([true].as_slice()),
        "a single tether holding a whole strand over a drop must part"
    );
}

#[test]
fn a_tear_never_heals() {
    let (mut strands, mut mesentery, s) = hanging_by_one_thread();
    let mut torn_count = 0usize;
    let mut ever_torn = vec![false; 1];
    let mut saw_a_tear = false;

    for _ in 0..900 {
        step(&mut strands, &mut mesentery, &s);
        let Some(m) = mesentery.first() else {
            panic!("the mesentery vanished");
        };
        // Monotone in two senses: no flag ever goes back to false, and the count never falls.
        for (i, torn) in m.torn.iter().enumerate() {
            if *torn {
                ever_torn[i] = true;
                saw_a_tear = true;
            }
            assert!(
                !(ever_torn[i] && !*torn),
                "link {i} healed — a tear must be monotone, like clotting"
            );
        }
        let now = m.torn.iter().filter(|t| **t).count();
        assert!(now >= torn_count, "the tear count fell from {torn_count} to {now}");
        torn_count = now;
    }
    assert!(saw_a_tear, "this fixture exists to produce a tear; it produced none");
}

#[test]
fn a_torn_tether_stops_holding() {
    // Once the thread parts the strand is in free fall, so its lowest node must keep descending
    // rather than settle at the tether's reach.
    let (mut strands, mut mesentery, s) = hanging_by_one_thread();
    for _ in 0..600 {
        step(&mut strands, &mut mesentery, &s);
    }
    let before = strands.iter().flat_map(|s| s.nodes()).map(|p| p.y).fold(f32::MAX, f32::min);
    for _ in 0..120 {
        step(&mut strands, &mut mesentery, &s);
    }
    let after = strands.iter().flat_map(|s| s.nodes()).map(|p| p.y).fold(f32::MAX, f32::min);
    assert!(after < before - 0.5, "a parted tether still held the strand: {before} → {after}");
}

// ---------------------------------------------------------------------------------------------
// The rod
// ---------------------------------------------------------------------------------------------

#[test]
fn segments_stay_within_a_few_percent_of_their_rest_length() {
    let s = ViscSettings::default();
    let mut strands = spill(WOUND, EXIT, 6, SEED, &s);
    let mut mesentery = tether_all(&strands, 4);
    for _ in 0..600 {
        step(&mut strands, &mut mesentery, &s);
    }

    let mut worst = 0.0f32;
    for strand in &strands {
        for pair in strand.nodes().windows(2) {
            let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
                continue;
            };
            let strain = ((*b - *a).length() - SPILL_REST_LEN).abs() / SPILL_REST_LEN;
            worst = worst.max(strain);
        }
    }
    assert!(
        worst < 0.05,
        "compliance_stretch = {DEFAULT_COMPLIANCE_STRETCH} is meant to be near-inextensible, but \
         the worst segment strain after 600 ticks was {worst}"
    );
}

#[test]
fn nothing_ever_ends_a_tick_below_the_floor() {
    let s = ViscSettings { floor_y: 0.25, ..Default::default() };
    let mut strands = spill(WOUND, EXIT, 6, SEED, &s);
    let mut mesentery = tether_all(&strands, 4);

    for tick in 0..600 {
        step(&mut strands, &mut mesentery, &s);
        for strand in &strands {
            let floor = s.floor_y + strand.radius();
            for node in strand.nodes() {
                assert!(
                    node.y >= floor - 1.0e-6,
                    "tick {tick}: a node at y = {} is below the floor at {floor}",
                    node.y
                );
            }
        }
    }
}

#[test]
fn a_strand_falls_and_then_settles_on_the_floor() {
    // The observable behaviour the crate is named for: guts land, and they stay landed.
    let s = ViscSettings::default();
    let mut strands = vec![Strand::new(
        Vec3::new(0.0, 2.0, 0.0),
        Vec3::Y,
        SPILL_SEGMENTS,
        SPILL_REST_LEN,
        SPILL_RADIUS,
    )];
    let mut mesentery: Vec<Mesentery> = Vec::new();
    for _ in 0..900 {
        step(&mut strands, &mut mesentery, &s);
    }
    let highest = strands
        .iter()
        .flat_map(|s| s.nodes())
        .map(|p| p.y)
        .fold(f32::MIN, f32::max);
    assert!(
        highest < 1.0,
        "an untethered strand dropped from 2 m should be coiled on the floor, not at {highest}"
    );
}

// ---------------------------------------------------------------------------------------------
// The mesh
// ---------------------------------------------------------------------------------------------

fn triangle_count(mesh: &Mesh) -> usize {
    mesh.indices().map(|i| i.len() / 3).unwrap_or(0)
}

#[test]
fn the_tube_has_the_counts_it_documents() {
    let strand = Strand::new(Vec3::ZERO, Vec3::Y, 4, 0.1, 0.05);
    let nodes = strand.nodes().len();
    assert_eq!(nodes, 5);

    for sides in [3u32, 8, 16] {
        let mesh = tube_mesh(&strand, sides);
        let ring = sides as usize + 1;
        assert_eq!(
            mesh.count_vertices(),
            (nodes + 2) * ring + 2,
            "{sides}-sided tube: one duplicated seam vertex per ring, plus a centre per cap"
        );
        assert_eq!(
            triangle_count(&mesh),
            (nodes - 1) * sides as usize * 2 + 2 * sides as usize,
            "{sides}-sided tube: two triangles per side per segment, plus two fans"
        );
    }
}

#[test]
fn the_side_count_is_clamped_rather_than_trusted() {
    let strand = Strand::new(Vec3::ZERO, Vec3::Y, 4, 0.1, 0.05);
    assert_eq!(triangle_count(&tube_mesh(&strand, 0)), triangle_count(&tube_mesh(&strand, 3)));
    assert_eq!(
        triangle_count(&tube_mesh(&strand, u32::MAX)),
        triangle_count(&tube_mesh(&strand, 32))
    );
}

#[test]
fn every_tube_normal_is_unit_length() {
    let s = ViscSettings::default();
    let mut strands = spill(WOUND, EXIT, 4, SEED, &s);
    let mut mesentery = tether_all(&strands, 4);
    for _ in 0..300 {
        step(&mut strands, &mut mesentery, &s);
    }

    for strand in &strands {
        let mesh = tube_mesh(strand, 8);
        let Some(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).and_then(|a| a.as_float3())
        else {
            panic!("the tube must carry ATTRIBUTE_NORMAL as three floats");
        };
        assert!(!normals.is_empty());
        for n in normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1.0e-4, "normal {n:?} has length {len}");
        }
        // Positions and UVs must be the same length or the mesh is not uploadable.
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).map(|a| a.len()).unwrap_or(0);
        let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).map(|a| a.len()).unwrap_or(0);
        assert_eq!(positions, normals.len());
        assert_eq!(uvs, normals.len());
    }
}

#[test]
fn every_tube_index_addresses_a_vertex_that_exists() {
    let strand = Strand::new(Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0), 6, 0.05, 0.02);
    let mesh = tube_mesh(&strand, 8);
    let count = mesh.count_vertices() as u32;
    let Some(indices) = mesh.indices() else {
        panic!("the tube must be indexed");
    };
    for i in indices.iter() {
        assert!((i as u32) < count, "index {i} addresses past {count} vertices");
    }
}

// ---------------------------------------------------------------------------------------------
// The dials
// ---------------------------------------------------------------------------------------------

#[test]
fn the_shipped_defaults_are_the_shipped_numbers() {
    let s = ViscSettings::default();
    assert_eq!(s.substeps, 4);
    assert_eq!(s.iterations, 8);
    assert_eq!(s.gravity, 18.0);
    assert_eq!(s.damping, 0.02);
    assert_eq!(s.compliance_stretch, 1.0e-6);
    assert_eq!(s.compliance_bend, 5.0e-4);
    assert_eq!(s.floor_y, 0.0);
    assert_eq!(s.max_strands, 8);

    // …and the named constants say the same thing, so there is one source for each number.
    assert_eq!(s.substeps, DEFAULT_SUBSTEPS);
    assert_eq!(s.iterations, DEFAULT_ITERATIONS);
    assert_eq!(s.gravity, DEFAULT_GRAVITY);
    assert_eq!(s.damping, DEFAULT_DAMPING);
    assert_eq!(s.compliance_stretch, DEFAULT_COMPLIANCE_STRETCH);
    assert_eq!(s.compliance_bend, DEFAULT_COMPLIANCE_BEND);
    assert_eq!(s.floor_y, DEFAULT_FLOOR_Y);
    assert_eq!(s.max_strands, DEFAULT_MAX_STRANDS);

    assert_eq!(Mesentery::default().tear_strain, 0.35);
    assert_eq!(SPILL_SEGMENTS, 24);
    assert_eq!(SPILL_REST_LEN, 0.035);
    assert_eq!(SPILL_RADIUS, 0.02);
}

#[test]
fn a_degenerate_strand_is_clamped_rather_than_built() {
    // Every one of these is a caller slip, and none of them may panic or produce a stuck strand.
    let too_long = Strand::new(Vec3::ZERO, Vec3::Y, 10_000, 0.05, 0.01);
    assert_eq!(too_long.nodes().len(), MAX_NODES);

    let zero_dir = Strand::new(Vec3::ZERO, Vec3::ZERO, 4, 0.05, 0.01);
    let spread = zero_dir
        .nodes()
        .last()
        .zip(zero_dir.nodes().first())
        .map(|(a, b)| (*a - *b).length())
        .unwrap_or(0.0);
    assert!(spread > 0.0, "coincident nodes have no stretch gradient and could never separate");

    let negative = Strand::new(Vec3::ZERO, Vec3::Y, 0, -1.0, -1.0);
    assert_eq!(negative.nodes().len(), 2);
    assert_eq!(negative.radius(), 0.0);

    // And a strand built out of nonsense still steps without producing NaN.
    let s = ViscSettings::default();
    let mut strands = vec![zero_dir, negative];
    let mut mesentery: Vec<Mesentery> = Vec::new();
    for _ in 0..60 {
        step(&mut strands, &mut mesentery, &s);
    }
    for strand in &strands {
        for node in strand.nodes() {
            assert!(node.is_finite(), "a clamped strand produced {node:?}");
        }
    }
}

#[test]
fn anchors_are_projected_in_ascending_node_order_however_they_were_pushed() {
    let s = ViscSettings { floor_y: -1_000.0, ..Default::default() };
    let build = |order: &[u32]| {
        let strands =
            vec![Strand::new(WOUND, Vec3::X, SPILL_SEGMENTS, SPILL_REST_LEN, SPILL_RADIUS)];
        let nodes = strands[0].nodes().to_vec();
        let anchors = order
            .iter()
            .filter_map(|i| nodes.get(*i as usize).map(|p| (*i, *p)))
            .collect::<Vec<_>>();
        let torn = vec![false; anchors.len()];
        (strands, vec![Mesentery { anchors, tear_strain: DEFAULT_TEAR_STRAIN, torn }])
    };

    let (mut a, mut am) = build(&[0, 6, 12, 18]);
    let (mut b, mut bm) = build(&[18, 0, 12, 6]);
    for _ in 0..300 {
        step(&mut a, &mut am, &s);
        step(&mut b, &mut bm, &s);
    }
    assert_eq!(
        digests(&a),
        digests(&b),
        "the projection order must come from the node index, not from the order a caller pushed"
    );
    // The flags travelled with their anchors rather than staying put.
    assert_eq!(am.first().map(|m| &m.torn), bm.first().map(|m| &m.torn));
    assert_eq!(am.first().map(|m| &m.anchors), bm.first().map(|m| &m.anchors));
}

// ---------------------------------------------------------------------------------------------
// The plugin
// ---------------------------------------------------------------------------------------------

#[test]
fn the_plugin_steps_entities_exactly_as_the_bare_function_does() {
    // The integration risk this crate actually carries: `VisceraPlugin` must not become a second
    // solver. One entity per strand through `FixedUpdate` has to produce the same bits as one call to
    // `step` over the whole slice — otherwise there are two paths and only one of them is tested.
    let settings = ViscSettings::default();
    let seeded = spill(WOUND, EXIT, 6, SEED, &settings);
    let tethers = tether_all(&seeded, 4);

    let mut app = App::new();
    app.add_plugins(VisceraPlugin);
    assert!(
        app.world().get_resource::<ViscSettings>().is_some(),
        "the plugin must init_resource ViscSettings — a missing Res panics its system in 0.19"
    );
    for (strand, tether) in seeded.iter().cloned().zip(tethers.iter().cloned()) {
        app.world_mut().spawn((strand, tether));
    }
    for _ in 0..600 {
        app.world_mut().run_schedule(FixedUpdate);
    }

    let mut direct = seeded;
    let mut direct_tethers = tethers;
    for _ in 0..600 {
        step(&mut direct, &mut direct_tethers, &settings);
    }

    let mut from_ecs: Vec<u64> =
        app.world_mut().query::<&Strand>().iter(app.world()).map(Strand::digest).collect();
    let mut expected = digests(&direct);
    // Query order is not stable across `App` instances, and the point here is that it cannot matter:
    // sorting both sides asserts the *set* of digests, which is the only thing the plugin promises.
    from_ecs.sort_unstable();
    expected.sort_unstable();
    assert_eq!(from_ecs, expected);
}
