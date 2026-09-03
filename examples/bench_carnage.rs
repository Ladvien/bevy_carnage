//! **What a massacre costs per frame, in a terminal.**
//!
//! No window, no GPU, no `App`, no clock — the whole deterministic half of this crate is reachable as
//! plain functions, so a carnage load is a script rather than a play session. That is what makes this
//! a benchmark instead of a vibe: the same sixteen bodies die on the same ticks, open the same wounds,
//! and bleed for the same 360 ticks every single run.
//!
//! Run: `cargo run --release -p bevy_carnage --example bench_carnage`
//!
//! # The number this exists to produce
//!
//! `carnage_frame_ms` — mean milliseconds of carnage work per simulated tick, taken from the fastest
//! of nine timed reps.
//! **Lower is better, and the point of lowering it is to afford more gore, not less.** Which is
//! exactly why the script's *output* is pinned:
//!
//! # A perf metric with nothing holding it down measures how much work you deleted
//!
//! Every cost here can be driven to zero by throwing fewer droplets, and a benchmark that rewarded
//! that would be an engine for making the game less bloody. So this example folds every emitted value
//! — every fragment centre, every wound, every stain, every droplet — into one FNV-1a digest, and
//! **refuses to report a timing at all if that digest moved.** The timing is only meaningful as
//! "the same carnage, cheaper".
//!
//! The digest is taken in a separate, untimed pass. Folding a million floats into a hash inside the
//! timed region would make the harness measure itself.
//!
//! # What the script actually exercises
//!
//! Both halves of a carnage frame, because they have very different shapes:
//!
//! - **The spike.** A body dies: one [`fracture_mesh`] bake of a two-shell subject with two bullet
//!   channels through the torso, then the channel wounds off [`Fracture::ejecta`], then a radial
//!   [`radial`] blast query over the bond graph turned into severance wounds by [`wounds_from_reach`].
//!   Reported separately as `bake_ms`, because a bake is what hitches a frame.
//! - **The tail.** Every open wound pulses on its own heartbeat for the next 360 ticks:
//!   [`pulse_wound`] scales its severity by [`flow`](bevy_carnage::flow), and the pulse throws
//!   [`droplets`] and lands [`stains`]. Reported as `sim_ms`.
//!
//! `TICKS` is 600 and the last body dies at tick 180, so every wound reaches its clot inside the
//! window. The script measures a complete lifecycle, never a truncated one.
//!
//! # Determinism
//!
//! Nothing here reads a clock, a thread id, or the environment. Every number is a function of the
//! subject index through [`hash_f32`], which is frozen by this crate's own `hash_f32_is_frozen`. The
//! only wall-clock reads are the timing instruments themselves, and they feed no decision.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use bevy::math::{Mat4, Vec3, primitives::Cuboid};
use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy_carnage::{
    Bleed, BloodSettings, Bore, CarnageSettings, CutSettings, Droplet, Ejecta, FragmentGeometry,
    ProxyCell, Stain, Wound, blood, droplets, flows, fracture_mesh, hash_f32 as unit,
    hitstop_ticks, pulse_wound, radial, shake_offset, stains, trauma_for, wound_of_channel,
    wounds_from_reach, yield_stress,
};

// ---------------------------------------------------------------------------------------------
// The workload. Every constant here is part of the contract: changing one is changing the
// benchmark, which means re-blessing the goldens at the bottom of this file.
// ---------------------------------------------------------------------------------------------

/// Fixed tick rate the bleed schedule is derived against. This crate counts ticks, never seconds.
const HZ: u32 = 60;
/// Length of the script. 600 ticks is 10 s at [`HZ`] — long enough that the last wound clots inside
/// the window, which is what makes the tail cost a complete measurement rather than a truncated one.
const TICKS: u32 = 600;
/// Bodies in the massacre.
const SUBJECTS: u32 = 16;
/// Ticks between deaths. `16 × 12` puts the last death at tick 180, leaving 420 ticks — more than the
/// shipped 360-tick clot — for every wound to run its full schedule.
const DEATH_STRIDE: u32 = 12;
/// Timed repetitions. Nine rather than five because the reported figure is the **fastest** of them,
/// and the fastest of nine is a better estimate of the unthrottled cost than the fastest of five. Nine
/// reps of a ~25 ms script is a quarter-second — the accuracy is free.
const REPS: usize = 9;
/// Discarded passes before the timed ones. See the comment at the warm-up loop for why it is two.
const WARMUPS: usize = 2;
/// How many of the fastest reps must agree for the minimum to count as a plateau.
const BEST_K: usize = 3;
/// How far the [`BEST_K`]th-fastest rep may sit above the fastest, as a percentage, before this
/// benchmark refuses to report at all.
///
/// Measured on an idle host: all nine reps inside 0.3 %. Measured forty seconds after a compile on
/// this fanless host: the fast end had not converged. 5 % is well clear of the first and firmly
/// excludes the second, and it is tight enough that a genuine 1 % regression is still visible.
const TIGHT_LIMIT_PCT: f64 = 5.0;
/// The floor blood lands on, subject-local. Wounds sit above it, so the ballistic solve in
/// [`bevy_carnage::landing`] has a plane to reach and every droplet resolves to a stain.
/// **Below the subject's lowest point, which is the soles at `y = -0.92`.** It was `-0.5`, which sat
/// *inside* the legs once the subject became a humanoid — a wound below the plane has no landing, so
/// half the spray would have silently stopped producing stains.
const PLANE_Y: f32 = -0.95;

/// Target fragment count per body — the finest granularity the bake cuts to.
///
/// **34, and the previous 12 was a bug in this script rather than a taste choice.** `bore::apply`
/// subtracts the two channels below from the proxy and hands back roughly sixteen root cells, so a
/// target of 12 was already exceeded before the recursive cut loop was entered and **it performed zero
/// cuts**. Instrumented, on the old settings: `nodes=16 leaves=16 interior=0`. The benchmark was
/// therefore measuring bore subtraction plus the emit of sixteen root pieces, and never touching the
/// plane cutter, the [`bevy_carnage::FragmentTree`], or any frontier query — which is most of what the
/// crate is. 34 is what this crate's own demo subject uses (`examples/common/body.rs`), and it is
/// comfortably above the bored root count, so the hierarchy is real.
const TARGET_PIECES: usize = 34;
/// Stop cutting a piece once its extent drops below this fraction of the whole.
///
/// 0.08 rather than 0.15, matching the demo subject: at 0.15 the volume floor stops the cut loop early
/// on limb cells, which would re-introduce the same "target never binds" problem from the other side.
const MIN_FRACTION: f32 = 0.08;
/// Slack, so `TARGET_PIECES` is what binds.
const MAX_DEPTH: u16 = 64;
/// Tier-B rounding. Non-zero because the shipped look is non-zero, and softening costs real time in
/// the bake that a benchmark running at 0.0 would never see.
const SOFTEN: f32 = 0.5;
/// Bullet channels per body.
const BORES_PER_SUBJECT: u32 = 2;
/// Channel radius, metres — a rifle calibre against a 0.6 m torso.
const BORE_RADIUS: f32 = 0.045;

/// Inner and outer radius of the blast that severs bonds. Inside `min` a bond is fully severed;
/// outside `max` it is untouched.
const BLAST_MIN_R: f32 = 0.10;
const BLAST_MAX_R: f32 = 0.55;
/// The caller's "this gives way" line for [`wounds_from_reach`]. A game rule, not a dial of the crate.
const SEVER_THRESHOLD: f32 = 0.35;

/// **Matches both of this crate's own demos, and the reason is arithmetic rather than taste.** At the
/// shipped `1.0` a droplet leaving straight up at 40 m/s under 18 m/s² gravity rises 44 metres — correct
/// for a real gunshot, absurd on a 1.8 m body, and it would throw every stain outside any floor.
const SPATTER_SPEED_SCALE: f32 = 0.25;

/// **This script's `Vec3` → `[f32; 3]` boundary**, and it has exactly one crossing.
///
/// `bloodstain` names no math library, so the blood model's public API is `[f32; 3]` while this
/// crate's fracture and wound types are `glam`. The crate converts in one private module,
/// `bevy_carnage::v3` — private on purpose, because it is an implementation detail of the facade
/// rather than a promise — so a caller converts at its own boundary. This is that boundary.
fn blood_wound(w: &Wound) -> blood::Wound {
    blood::Wound {
        at: [w.at.x, w.at.y, w.at.z],
        normal: [w.normal.x, w.normal.y, w.normal.z],
        area: w.area,
        severity: w.severity,
        kind: w.kind,
    }
}


// ---------------------------------------------------------------------------------------------
// Observation. Two sinks over one script, so the timed pass and the hashed pass cannot diverge.
// ---------------------------------------------------------------------------------------------

/// Everything the script emits, routed somewhere that cannot be optimised away.
///
/// **A trait rather than a flag** so it monomorphises: the timed pass compiles with no digest code in
/// it at all, and the hashed pass with no timing significance. A `bool` parameter would put a branch
/// on the inner droplet loop, which is the one loop that must stay honest.
trait Sink {
    /// A spawnable piece, cell **and** drawn surface.
    ///
    /// **Takes the whole fragment, not just its centre, and that was a hole worth closing.** The first
    /// version of this trait folded `center_local` alone — which comes from the convex cell — so the
    /// digest never observed `outer` or `cap` at all. A change that stopped building the drawn meshes
    /// entirely would have passed the gate and reported a large, meaningless win, and deferring mesh
    /// construction is the single most obvious thing to try against a bake that is 88 % of the cost.
    /// A benchmark whose gate cannot see the first optimisation anyone would attempt is not a gate.
    fn fragment(&mut self, f: &FragmentGeometry);
    /// A plug a bore pushed out: debris, with its own drawn surface.
    fn plug(&mut self, p: &Ejecta);
    fn wound(&mut self, w: &Wound);
    fn stain(&mut self, s: &Stain);
    fn droplet(&mut self, d: &Droplet);
    fn shake(&mut self, offset: Vec3);
}

/// Triangles a mesh draws, or zero if it has no indices.
fn tris(mesh: Option<&Mesh>) -> u64 {
    mesh.and_then(Mesh::indices).map_or(0, |i| i.len() as u64 / 3)
}

/// FNV-1a over the raw bits of every emitted value, in emission order.
///
/// **Raw bits, never a tolerance.** "Did the carnage move" is a question about exact floats; comparing
/// them approximately answers a different and easier question.
struct Digest {
    h: u64,
}

impl Digest {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self { h: Self::BASIS }
    }

    fn u32(&mut self, v: u32) {
        self.h ^= u64::from(v);
        self.h = self.h.wrapping_mul(Self::PRIME);
    }

    fn u64(&mut self, v: u64) {
        self.u32((v & 0xffff_ffff) as u32);
        self.u32((v >> 32) as u32);
    }

    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }

    /// **`bloodstain`'s vector, folded byte-for-byte the way [`Self::vec3`] folds `glam`'s.**
    ///
    /// The blood model's public API is `[f32; 3]` rather than `Vec3`, so a stain position and a
    /// droplet direction arrive as arrays. Three `f32`s in `x, y, z` order is exactly what `vec3`
    /// eats, so the digest below is the same number it was before the move — which is the only
    /// acceptable outcome, since a printed benchmark digest that shifted on a refactor would say the
    /// carnage changed when nothing did.
    fn arr3(&mut self, v: [f32; 3]) {
        self.f32(v[0]);
        self.f32(v[1]);
        self.f32(v[2]);
    }

    /// Every vertex position a mesh carries, in buffer order, plus its triangle count.
    ///
    /// **Only the untimed pass does this.** Folding a few hundred thousand floats into a hash is real
    /// work, and doing it inside the timed region would make the harness measure itself.
    fn mesh(&mut self, mesh: Option<&Mesh>) {
        self.u64(tris(mesh));
        let Some(m) = mesh else { return };
        let Some(VertexAttributeValues::Float32x3(p)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
            return;
        };
        for q in p {
            self.f32(q[0]);
            self.f32(q[1]);
            self.f32(q[2]);
        }
    }
}

impl Sink for Digest {
    fn fragment(&mut self, f: &FragmentGeometry) {
        self.vec3(f.center_local);
        self.vec3(f.half_extents);
        self.mesh(f.outer.as_ref());
        self.mesh(f.cap.as_ref());
    }
    fn plug(&mut self, p: &Ejecta) {
        self.vec3(p.center_local);
        self.vec3(p.half_extents);
        self.vec3(p.exit);
        self.vec3(p.direction);
        self.mesh(p.outer.as_ref());
        self.mesh(p.cap.as_ref());
    }
    fn wound(&mut self, w: &Wound) {
        self.vec3(w.at);
        self.vec3(w.normal);
        self.f32(w.area);
        self.f32(w.severity);
        self.u32(w.kind as u32);
    }
    fn stain(&mut self, s: &Stain) {
        self.arr3(s.at);
        self.f32(s.radius);
        self.u32(s.seed);
    }
    fn droplet(&mut self, d: &Droplet) {
        self.arr3(d.dir);
        self.f32(d.speed);
        self.f32(d.diameter);
    }
    fn shake(&mut self, offset: Vec3) {
        self.vec3(offset);
    }
}

/// The timed pass's sink: consumes every value cheaply so the optimiser cannot delete the work that
/// produced it, and hashes nothing.
///
/// One `f32` add per value. It is printed as `ASI sink=` for exactly one reason — a value that is
/// never observed is a value LLVM is entitled to stop computing.
#[derive(Default)]
struct Blackhole {
    acc: f32,
}

impl Sink for Blackhole {
    fn fragment(&mut self, f: &FragmentGeometry) {
        self.acc += f.center_local.x + f.half_extents.x + tris(f.outer.as_ref()) as f32;
    }
    fn plug(&mut self, p: &Ejecta) {
        self.acc += p.exit.x + tris(p.cap.as_ref()) as f32;
    }
    fn wound(&mut self, w: &Wound) {
        self.acc += w.area + w.severity;
    }
    fn stain(&mut self, s: &Stain) {
        self.acc += s.radius;
    }
    fn droplet(&mut self, d: &Droplet) {
        self.acc += d.speed + d.diameter;
    }
    fn shake(&mut self, offset: Vec3) {
        self.acc += offset.x;
    }
}

/// How much carnage the script produced. **Pinned, because the timing means nothing without it.**
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Counts {
    fragments: u64,
    ejecta: u64,
    bonds: u64,
    wounds: u64,
    pulses: u64,
    droplets: u64,
    stains: u64,
    /// Triangles of the subject's own skin, summed over every spawned piece and plug.
    skin_tris: u64,
    /// Triangles of newly-created cut surface — the raw interior, the thing that reads as severed.
    ///
    /// **Counted separately from the skin because they are the two halves of the crate's premise**, and
    /// because a change that quietly stopped capping would be invisible in a combined total that the
    /// skin dominates.
    cap_tris: u64,
    /// Summed [`hitstop_ticks`], so the game-feel curves are inside the golden too.
    hitstop: u64,
}

/// One pass of the script.
struct Run {
    counts: Counts,
    /// Milliseconds spent inside each simulated tick, in order.
    tick_ms: Vec<f64>,
    /// Of that, milliseconds spent in [`fracture_mesh`] and the wound extraction that follows it.
    bake_ms: f64,
}

impl Run {
    fn total_ms(&self) -> f64 {
        self.tick_ms.iter().sum()
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

// ---------------------------------------------------------------------------------------------
// The scene, and the script over it.
// ---------------------------------------------------------------------------------------------

/// The subject every body in the massacre is a copy of: **a blocked-out humanoid, one convex cell per
/// body part**, exactly the decomposition this crate's own demo subject uses
/// (`examples/common/body.rs`). Six shells, never unioned — which is what keeps a head separable from
/// a torso, and what makes the bond graph a graph rather than a line.
///
/// **It was two boxes, and that was too easy a subject to measure against.** Six cells give the cut
/// loop somewhere to go, the tree real depth, and the bond graph real branching; two gave a target of
/// twelve nothing to bind against once the bores had already produced sixteen roots.
///
/// **One geometry, sixteen seeds.** Varying the mesh per body would make the benchmark's variance a
/// property of the fixture rather than of the code under test; varying the seed varies the cut pattern,
/// which is the thing worth varying.
struct Scene {
    /// One box mesh per part, parallel to `proxy`.
    meshes: Vec<Mesh>,
    /// Where each part sits, parallel to `meshes`.
    offsets: Vec<Vec3>,
    /// One convex cell per shell, never unioned.
    proxy: Vec<ProxyCell>,
    settings: CarnageSettings,
}

/// `(centre, half-extent)` per body part, subject-local. Copied from the demo subject so the benchmark
/// and the recordings describe the same shape.
const PARTS: [(Vec3, Vec3); 6] = [
    // torso
    (Vec3::new(0.00, 0.00, 0.0), Vec3::new(0.22, 0.32, 0.14)),
    // head
    (Vec3::new(0.00, 0.46, 0.0), Vec3::new(0.13, 0.14, 0.13)),
    // arm.L / arm.R
    (Vec3::new(-0.32, 0.06, 0.0), Vec3::new(0.10, 0.26, 0.10)),
    (Vec3::new(0.32, 0.06, 0.0), Vec3::new(0.10, 0.26, 0.10)),
    // leg.L / leg.R
    (Vec3::new(-0.13, -0.62, 0.0), Vec3::new(0.11, 0.30, 0.12)),
    (Vec3::new(0.13, -0.62, 0.0), Vec3::new(0.11, 0.30, 0.12)),
];

impl Scene {
    fn new() -> Self {
        let mut meshes = Vec::with_capacity(PARTS.len());
        let mut offsets = Vec::with_capacity(PARTS.len());
        let mut proxy = Vec::with_capacity(PARTS.len());
        for (centre, half) in PARTS {
            meshes.push(Mesh::from(Cuboid::new(
                half.x * 2.0,
                half.y * 2.0,
                half.z * 2.0,
            )));
            offsets.push(centre);
            proxy.push(ProxyCell::from_box(centre, half));
        }
        Self {
            meshes,
            offsets,
            proxy,
            settings: CarnageSettings {
                blood: BloodSettings {
                    spatter_speed_scale: SPATTER_SPEED_SCALE,
                    ..BloodSettings::default()
                },
                ..CarnageSettings::default()
            },
        }
    }

    /// The same `(&Mesh, Mat4)` pairs the ECS bake assembles by walking a scene's children.
    ///
    /// **Order is load-bearing** — [`fracture_mesh`] says so: cut planes are placed relative to cell
    /// centroids and the payload's vertex order decides float sums downstream. `PARTS` is a `const`
    /// array, so the order is authored and fixed.
    fn parts(&self) -> Vec<(&Mesh, Mat4)> {
        self.meshes
            .iter()
            .zip(&self.offsets)
            .map(|(m, c)| (m, Mat4::from_translation(*c)))
            .collect()
    }

    /// Two channels front-to-back through the torso, placed by the subject's own hash.
    fn bores(&self, subject: u32) -> Vec<Bore> {
        let mut out = Vec::with_capacity(BORES_PER_SUBJECT as usize);
        for k in 0..BORES_PER_SUBJECT {
            let seed = subject.wrapping_mul(977).wrapping_add(k.wrapping_mul(31));
            let y = -0.32 + unit(seed) * 0.64;
            let x = -0.16 + unit(seed ^ 0x5bf0_3635) * 0.32;
            out.push(Bore::new(
                Vec3::new(x, y, -0.40),
                Vec3::new(x, y, 0.40),
                BORE_RADIUS,
            ));
        }
        out
    }

    fn cut(&self, subject: u32) -> CutSettings {
        let seed = 0x00c0_ffee ^ subject.wrapping_mul(0x9e37_79b9);
        CutSettings {
            max_depth: MAX_DEPTH,
            soften: SOFTEN,
            bores: self.bores(subject),
            ..CutSettings::new(TARGET_PIECES, MIN_FRACTION, seed)
        }
    }

    /// **A body dies.** The bake, the channel wounds it pushed out, and the blast that took bonds off
    /// it — everything that happens on the one frame a subject comes apart.
    fn open<S: Sink>(
        &self,
        subject: u32,
        tick: u32,
        sink: &mut S,
        counts: &mut Counts,
        live: &mut Vec<(Wound, Bleed)>,
    ) {
        let mut baked = fracture_mesh(&self.parts(), &self.proxy, &self.cut(subject));

        counts.bonds += baked.bonds.len() as u64;
        for frag in baked.leaves() {
            counts.fragments += 1;
            counts.skin_tris += tris(frag.outer.as_ref());
            counts.cap_tris += tris(frag.cap.as_ref());
            sink.fragment(frag);
        }

        // Two sources of wound, and they are genuinely different events: a channel left an interior
        // wall open to the air, a severance stopped two fragments sharing a face.
        let mut fresh: Vec<Wound> = Vec::with_capacity(baked.ejecta.len() + 8);
        for plug in &baked.ejecta {
            counts.ejecta += 1;
            counts.skin_tris += tris(plug.outer.as_ref());
            counts.cap_tris += tris(plug.cap.as_ref());
            sink.plug(plug);
            fresh.push(wound_of_channel(&plug.cell, plug.exit, plug.direction));
        }

        let hit = Vec3::new(
            -0.20 + unit(subject ^ 0x1234_5678) * 0.40,
            -0.40 + unit(subject ^ 0x2468_ace0) * 0.80,
            0.0,
        );
        let reach = radial(&baked.bonds, hit, BLAST_MIN_R, BLAST_MAX_R);
        fresh.extend(wounds_from_reach(&baked.bonds, &reach, SEVER_THRESHOLD));

        for w in fresh {
            counts.wounds += 1;
            sink.wound(&w);
            // The impact spatter: where this wound's first throw lands, solved in closed form.
            for st in stains(&blood_wound(&w), &self.settings.blood, PLANE_Y) {
                counts.stains += 1;
                sink.stain(&st);
            }
            counts.hitstop += u64::from(hitstop_ticks(&w, HZ, &self.settings));
            live.push((w, Bleed::new(tick, &blood_wound(&w))));
        }
    }

    /// The whole script: 600 ticks, sixteen deaths, and every wound bleeding on its own heartbeat
    /// until it clots.
    fn run<S: Sink>(&self, sink: &mut S) -> Run {
        let mut counts = Counts::default();
        let mut tick_ms = Vec::with_capacity(TICKS as usize);
        let mut bake_ms = 0.0f64;
        let mut live: Vec<(Wound, Bleed)> = Vec::new();

        for tick in 0..TICKS {
            let frame = Instant::now();

            if tick % DEATH_STRIDE == 0 {
                let subject = tick / DEATH_STRIDE;
                if subject < SUBJECTS {
                    let spike = Instant::now();
                    self.open(subject, tick, sink, &mut counts, &mut live);
                    bake_ms += ms(spike.elapsed());
                }
            }

            // **The tail.** `pulse_wound` returns `None` between beats and after the clot, and when it
            // returns a wound that wound's severity is already scaled by the flow curve — so one code
            // path serves the first arterial jet and the last seep.
            for (wound, bleed) in &live {
                let local = blood_wound(wound);
                if let Some(pulsed) = pulse_wound(bleed, &local, tick, HZ, &self.settings.blood) {
                    counts.pulses += 1;
                    for d in droplets(&pulsed, &self.settings.blood) {
                        counts.droplets += 1;
                        sink.droplet(&d);
                    }
                    for st in stains(&pulsed, &self.settings.blood, PLANE_Y) {
                        counts.stains += 1;
                        sink.stain(&st);
                    }
                    // A pulse scales the severity and nothing else, so the two feel numbers read the
                    // wound's own `glam` form at that severity — the same floats, in the same order,
                    // that the pulsed wound carried before the blood model moved out.
                    let feel = Wound { severity: pulsed.severity, ..*wound };
                    let trauma = trauma_for(&feel, &self.settings);
                    sink.shake(shake_offset(trauma, feel.normal, tick, &self.settings));
                }
            }

            // **Cessation is one material predicate now**: the same yield-stress crossing that
            // arrests a rivulet on a wall. Monotone once it fails, so a retain is a retire and never
            // a resurrection.
            live.retain(|(_, bleed)| {
                flows(
                    blood::bleed::driving_stress(bleed, tick, HZ, &self.settings.blood),
                    yield_stress(bleed.age(tick), HZ, &self.settings.blood),
                )
            });

            tick_ms.push(ms(frame.elapsed()));
        }

        Run {
            counts,
            tick_ms,
            bake_ms,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The goldens. Re-bless deliberately, never reflexively: a moved digest means the carnage changed,
// and the whole value of the timing is that it did not.
// ---------------------------------------------------------------------------------------------

/// FNV-1a over every emitted value, then the counts.
///
/// It is a fact about the current fracture, spatter and bleed code, not a target — if it moves, the two
/// timings either side of the move are measuring different massacres.
///
/// **Re-blessed three times, and every reason is worth keeping.** The first version folded only
/// `center_local`, which comes from the convex cell, so it never observed `outer` or `cap` — it would
/// have passed a change that stopped building the drawn meshes altogether. The second re-bless was
/// because the *workload* was wrong: `bore::apply` handed back ~16 root cells against a
/// `TARGET_PIECES` of 12, so the recursive cut loop performed **zero cuts**
/// (`nodes=16 leaves=16 interior=0`) and the benchmark never touched the plane cutter, the tree, or a
/// frontier query. Fixed by a six-cell humanoid subject and a target of 34, which now binds exactly:
/// 544 = 34 x 16.
///
/// The third is this one: `append_cut_faces` gained two independent cut-face reductions — a decimated
/// relief ring ([`bevy_carnage`'s `RELIEF_STRIDE`]) and a plain centre fan for unrelieved faces. It
/// is a *cosmetic* re-bless, and the evidence is which counts held: `fragments`, `ejecta`, `bonds`,
/// `wounds`, `pulses`, `droplets`, `stains` and `hitstop` are all byte-identical, so the simulation
/// is untouched and only the drawn surface moved. Setting the stride to 1 and disabling the flat-face
/// branch reproduces the previous digest `0xb672df832d78b94c` exactly, which is how "no incidental
/// drift" was established rather than assumed.
///
/// The fourth is `0.2.0`'s cessation change, and it is the cleanest re-bless of the four. `clotted`
/// — an integer comparison against `clot_ticks` — is gone, and a wound now stops when
/// `rheo::flows` says the stress driving blood out of it no longer exceeds the blood's own yield
/// stress. A falling driving stress meets a rising yield stress *inside* the taper, so every wound
/// arrests slightly earlier. **The counts localise the change to that predicate and nothing else:**
/// `fragments`, `ejecta`, `bonds`, `wounds`, `skin_tris`, `cap_tris` and `hitstop` are all
/// byte-identical, so the fracture, the bond graph, the wound extraction and the feel curves are
/// untouched — and `pulses` fell by 865, which is **exactly** the wound count. Every wound lost one
/// heartbeat and no wound lost two. `droplets` and `stains` follow that one lost beat.
const GOLDEN_DIGEST: u64 = 0xfc71_5deb_a0ea_5cf8;

/// What the script is supposed to produce. Checked field by field so a failure names the thing that
/// moved instead of just saying a hash did.
///
/// Three of these are worth reading rather than scrolling past:
///
/// - **`fragments` is 544, exactly `TARGET_PIECES * SUBJECTS`.** That equality is the evidence the cut
///   loop is doing its job; when it was 274 against a target of 12, the target was not binding at all.
/// - **`bonds` is 1164 against the old 417.** Six shells cut to 34 pieces give the bond graph real
///   branching, which is what makes `radial` + `wounds_from_reach` a meaningful query.
/// - **`cap_tris` is 1.4x `skin_tris`, and used to be 3.4x.** The drawn surface of a fractured body
///   is still mostly newly-created cut face — so anything touching cap generation moves far more
///   triangles than its name suggests — but the two reductions in `append_cut_faces` took it from
///   250,942 to 106,721 (**-57.5 %**), which is where the bake's `soften` cost went.
/// - **`skin_tris` moved 73,660 -> 73,684, and that +24 is not a leak.** `append_cut_faces` writes
///   into the same soup as the render skin and `draw_piece` softens the *combined* result on purpose,
///   so that the relaxation bevels the skin/cap edge; the split by interior tag happens afterwards.
///   Perturbing cap vertices therefore perturbs the weld along the shared seam by 0.03 %. Measured in
///   isolation the two reductions give +5 and +25 — their combination being +24 rather than +30 is
///   the evidence it is a weld interaction and not a systematic offset. `RELIEF_STRIDE`'s own docs
///   carry the full table.
const GOLDEN_COUNTS: Counts = Counts {
    fragments: 544,
    ejecta: 172,
    bonds: 1164,
    wounds: 865,
    pulses: 7785,
    droplets: 290_678,
    stains: 327_568,
    skin_tris: 73_684,
    cap_tris: 106_721,
    hitstop: 1792,
};

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let last = sorted.len() - 1;
    let idx = ((last as f64) * q).round().max(0.0) as usize;
    sorted.get(idx.min(last)).copied().unwrap_or(0.0)
}

fn cmp_f64(a: &f64, b: &f64) -> Ordering {
    a.partial_cmp(b).unwrap_or(Ordering::Equal)
}

fn main() {
    let scene = Scene::new();

    // ---- Pass 1: what did we produce? Untimed, so the hashing cannot pollute the measurement.
    let mut digest = Digest::new();
    let observed = scene.run(&mut digest);
    let c = observed.counts;
    for v in [
        c.fragments,
        c.ejecta,
        c.bonds,
        c.wounds,
        c.pulses,
        c.droplets,
        c.stains,
        c.skin_tris,
        c.cap_tris,
        c.hitstop,
    ] {
        digest.u64(v);
    }
    let seen = digest.h;

    // ---- Pass 2: what did it cost? Discarded warm-ups, then REPS timed passes.
    //
    // **Two warm-ups, not one, and the second is not about caches.** One pass is ~25 ms of work —
    // enough to fault in the code and warm the allocator, not enough to outlast whatever the build
    // that preceded it left running. The second is there to be thrown away on a machine that has not
    // finished settling, so the first *timed* rep is not the one paying for it.
    let mut sink_acc = 0.0f32;
    for _ in 0..WARMUPS {
        let mut warm = Blackhole::default();
        let _ = scene.run(&mut warm);
        sink_acc += warm.acc;
    }

    let mut runs: Vec<Run> = Vec::with_capacity(REPS);
    for _ in 0..REPS {
        let mut bh = Blackhole::default();
        runs.push(scene.run(&mut bh));
        sink_acc += bh.acc;
    }

    // **The FASTEST rep, not the median, and this was a correction.**
    //
    // The first design took the median, on the reasonable-sounding grounds that one descheduled run
    // should not move the figure. On this host that is the wrong estimator, and four measured runs
    // said so: the same binary on the same tree reported 122.8 ms idle, 152.3 ms with nothing running
    // but forty seconds after a compile, and 174.5 ms four seconds after one. The machine is a fanless
    // MacBook Air, so its *clock ceiling* moves for minutes after a build — and a median tracks the
    // ceiling instead of the code, which makes cooling down look like an optimisation.
    //
    // The minimum does not have that failure. Noise is one-sided: contention and throttling only ever
    // make a rep slower, never faster, so across enough reps the fastest one is the closest available
    // estimate of what the work costs at full clock. It is the standard microbenchmark estimator for
    // exactly this reason.
    //
    // Every reported figure still comes from that ONE rep — mean, p99, max, bake, sim — so they stay
    // mutually consistent instead of being stitched from nine different runs.
    runs.sort_by(|a, b| cmp_f64(&a.total_ms(), &b.total_ms()));
    let Some(fastest) = runs.first() else {
        eprintln!("bench_carnage: no timed run completed — REPS is {REPS}");
        std::process::exit(1);
    };

    let total = fastest.total_ms();
    let mut ticks = fastest.tick_ms.clone();
    ticks.sort_by(cmp_f64);

    // ---- The stability gate. Checked before the carnage gate because a number produced on a machine
    // this busy is not evidence of anything, whatever the digest says.
    //
    // **It asks whether the FASTEST reps agree, not whether all of them do, and that was the third
    // attempt at this check.** Slowest-minus-fastest was the obvious statistic and it is the wrong one:
    // contention and throttling produce a long *slow* tail, so that spread is set by the worst rep,
    // while the figure being reported comes from the best. Measured on this host right after a compile:
    // slowest-minus-fastest 72 % — rejected — with a fastest rep of 137.4 ms against 122.8 ms idle. The
    // gate was firing on reps nobody was going to use.
    //
    // So: sort ascending, and require the best [`BEST_K`] to sit within [`TIGHT_LIMIT_PCT`] of each
    // other. If they do, the minimum is a plateau several independent reps found, which is what makes
    // it an estimate rather than a lucky sample. If they do not, the machine was still moving
    // underneath the measurement and no single rep means anything.
    //
    // The original 168 %-spread run — a `cargo build` on every core — fails this too, and for the right
    // reason: no two of its reps agreed on anything.
    let noise = spread(&runs);
    let noise_pct = if total > 0.0 { noise / total * 100.0 } else { 0.0 };
    let kth = runs.get(BEST_K - 1).map(Run::total_ms).unwrap_or(total);
    let tight_pct = if total > 0.0 {
        (kth - total) / total * 100.0
    } else {
        0.0
    };
    if tight_pct > TIGHT_LIMIT_PCT {
        eprintln!();
        eprintln!("bench_carnage: NO STABLE PLATEAU — refusing to report a timing.");
        eprintln!();
        eprintln!("  fastest rep total            {total:>10.3} ms");
        eprintln!("  {BEST_K}rd-fastest rep total          {kth:>10.3} ms");
        eprintln!("  best-{BEST_K} spread               {tight_pct:>10.2} % (limit {TIGHT_LIMIT_PCT:.2} %)");
        eprintln!("  slowest minus fastest        {noise:>10.3} ms  ({noise_pct:.1} %, informational)");
        eprintln!();
        eprintln!("  The fastest reps did not agree, so the minimum is a lucky sample rather than a");
        eprintln!("  measurement. Something else is using the CPU — a compile, a test run, an indexer");
        eprintln!("  — or this fanless host is still shedding heat from one. Wait and re-run; nothing");
        eprintln!("  about the code under test is implicated.");
        eprintln!();
        std::process::exit(1);
    }

    // ---- The gate. A timing is only meaningful if the carnage is unchanged.
    if seen != GOLDEN_DIGEST || c != GOLDEN_COUNTS {
        eprintln!();
        eprintln!("bench_carnage: THE CARNAGE MOVED — refusing to report a timing.");
        eprintln!();
        eprintln!("  A perf number is only meaningful as 'the same carnage, cheaper'. Something");
        eprintln!("  changed what this script emits, so the two runs are not comparable.");
        eprintln!();
        eprintln!("                    expected              actual");
        eprintln!("  digest      {:#018x}  {:#018x}", GOLDEN_DIGEST, seen);
        let rows: [(&str, u64, u64); 10] = [
            ("fragments", GOLDEN_COUNTS.fragments, c.fragments),
            ("ejecta", GOLDEN_COUNTS.ejecta, c.ejecta),
            ("bonds", GOLDEN_COUNTS.bonds, c.bonds),
            ("wounds", GOLDEN_COUNTS.wounds, c.wounds),
            ("pulses", GOLDEN_COUNTS.pulses, c.pulses),
            ("droplets", GOLDEN_COUNTS.droplets, c.droplets),
            ("stains", GOLDEN_COUNTS.stains, c.stains),
            ("skin_tris", GOLDEN_COUNTS.skin_tris, c.skin_tris),
            ("cap_tris", GOLDEN_COUNTS.cap_tris, c.cap_tris),
            ("hitstop", GOLDEN_COUNTS.hitstop, c.hitstop),
        ];
        for (name, want, got) in rows {
            let flag = if want == got { ' ' } else { '<' };
            eprintln!("  {name:<10}  {want:>18}  {got:>18} {flag}");
        }
        eprintln!();
        eprintln!("  If the change was deliberate, re-bless the block at the bottom of");
        eprintln!("  examples/bench_carnage.rs — and say so in the commit:");
        eprintln!();
        eprintln!("    const GOLDEN_DIGEST: u64 = {seen:#018x};");
        eprintln!("    const GOLDEN_COUNTS: Counts = Counts {{");
        eprintln!("        fragments: {},", c.fragments);
        eprintln!("        ejecta: {},", c.ejecta);
        eprintln!("        bonds: {},", c.bonds);
        eprintln!("        wounds: {},", c.wounds);
        eprintln!("        pulses: {},", c.pulses);
        eprintln!("        droplets: {},", c.droplets);
        eprintln!("        stains: {},", c.stains);
        eprintln!("        skin_tris: {},", c.skin_tris);
        eprintln!("        cap_tris: {},", c.cap_tris);
        eprintln!("        hitstop: {},", c.hitstop);
        eprintln!("    }};");
        eprintln!();
        std::process::exit(1);
    }

    // ---- The report.
    println!();
    println!(
        "bevy_carnage — {SUBJECTS} bodies, {TICKS} ticks at {HZ} Hz, best of {REPS} timed reps"
    );
    println!(
        "  {} fragments · {} plugs · {} wounds · {} pulses · {} droplets · {} stains",
        c.fragments, c.ejecta, c.wounds, c.pulses, c.droplets, c.stains
    );
    println!(
        "  {} skin triangles · {} cut-face triangles drawn",
        c.skin_tris, c.cap_tris
    );
    println!("  digest {seen:#018x} — unchanged, so the timing below is comparable");
    println!();

    println!("METRIC carnage_frame_ms={:.5}", total / f64::from(TICKS));
    println!("METRIC carnage_total_ms={total:.3}");
    println!("METRIC carnage_p99_ms={:.5}", percentile(&ticks, 0.99));
    println!(
        "METRIC carnage_max_ms={:.5}",
        ticks.last().copied().unwrap_or(0.0)
    );
    println!("METRIC bake_ms={:.3}", fastest.bake_ms);
    println!("METRIC sim_ms={:.3}", total - fastest.bake_ms);
    println!("METRIC fragments={}", c.fragments);
    println!("METRIC ejecta={}", c.ejecta);
    println!("METRIC bonds={}", c.bonds);
    println!("METRIC wounds={}", c.wounds);
    println!("METRIC pulses={}", c.pulses);
    println!("METRIC droplets={}", c.droplets);
    println!("METRIC stains={}", c.stains);
    println!("METRIC skin_tris={}", c.skin_tris);
    println!("METRIC cap_tris={}", c.cap_tris);

    println!("ASI digest={seen:#018x}");
    println!("ASI reps={REPS}");
    println!("ASI ticks={TICKS}");
    println!("ASI subjects={SUBJECTS}");
    println!("ASI spread_ms={noise:.3}");
    println!("ASI noise_pct={noise_pct:.3}");
    println!("ASI best{BEST_K}_pct={tight_pct:.3}");
    println!("ASI sink={sink_acc:.3}");
}

/// Slowest timed rep minus fastest, in milliseconds.
///
/// Informational, and deliberately not the gate: on a throttling host the slow tail is set by reps the
/// report never uses. The gate is the best-[`BEST_K`] plateau above. This is here so a reader can see
/// how bad the tail was on a run that still passed.
fn spread(runs: &[Run]) -> f64 {
    let first = runs.first().map(Run::total_ms).unwrap_or(0.0);
    let last = runs.last().map(Run::total_ms).unwrap_or(0.0);
    last - first
}
