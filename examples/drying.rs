//! **The coagulation timeline**: one scalar age, five channels, two pools that differ only in area.
//!
//! Scrub the age and watch every channel move at once, straight off
//! [`bevy_carnage::blood::dry::appearance`] — nothing here is keyframed and nothing is a colour
//! ramp:
//!
//! * **colour** walks oxyhaemoglobin -> methaemoglobin -> hemichrome (Bremmer et al.,
//!   `10.1016/j.forsciint.2011.07.027`) — three stops, not two, which is why old blood is a brown
//!   rather than a dark red. It goes straight into `base_color`.
//! * **roughness** goes into `perceptual_roughness`. Gloss, not hue, is the strongest cue (Oum,
//!   Lieberman & Aylward, `10.1080/02699931.2010.496997`), so wetness is specular here.
//! * **rim** is the rim-first drying front (Smith, Nicloux & Brutin,
//!   `10.1038/s41598-020-65465-4`): a matte ring grows inward from the edge while the centre is
//!   still glossy.
//! * **halo** is the serum ring **outside** the pool, and it is **exactly absent** below 50 %
//!   relative humidity (Laan et al., `10.1016/j.forsciint.2016.08.005`) — a refusal, not a very
//!   small number.
//! * **craquelure** cracks the late crust.
//!
//! The two pools carry 10 cm^2 and 40 cm^2 of blood and nothing else differs.
//! [`bevy_carnage::blood::dry::dry_ticks`] scales with the square root of the area, so the small
//! one is bone dry while the big one is still wet in the middle — that gap is the thing to notice.
//!
//! ```text
//!   left / right   scrub the age
//!   H              humidity
//!   Space          play
//! ```
//!
//! Builds all its geometry in code and ships no assets, so it runs in a browser. Every string on
//! screen is ASCII, because the embedded default font is a 95-codepoint subset and anything else
//! renders as tofu.
//!
//! Run: `cargo run --release -p bevy_carnage --example drying`

use bevy::prelude::*;
use bevy_carnage::blood::dry::{
    CRAQUELURE_ONSET, HALO_HUMIDITY, SRGB_HEMI, SRGB_MET, SRGB_OXY, appearance, dry_ticks,
};
use bevy_carnage::{BloodSettings, hash_f32};

/// The fixed tick, and the rate `bloodstain`'s shipped tick counts are authored for. Every age here
/// is an integer count of these — no `Instant` anywhere, because there is no such clock in a
/// browser.
const HZ: u32 = 60;

/// **The two pools, and the only thing that differs between them.** 10 cm^2 and 40 cm^2.
const AREAS: [f32; 2] = [1.0e-3, 4.0e-3];
/// Where each sits on the floor, metres. Far enough apart that the larger serum halo clears the
/// smaller pool.
const CENTRES: [f32; 2] = [-0.055, 0.075];
const NAMES: [&str; 2] = ["pool A", "pool B"];

/// Quantisation of the rim front: one prebuilt annulus per step, so the ring advances without
/// building a mesh on the fly.
const RIM_STEPS: u32 = 32;
/// How far in, as a fraction of the radius, the fully advanced front reaches. The centre never
/// finishes drying before the edge, which is the whole point of a rim-first front.
const RIM_DEPTH: f32 = 0.55;
/// Outer radius of the serum halo, as a multiple of the pool radius.
const HALO_OUTER: f32 = 1.38;
/// Serum is pale straw, not red — it is the phase that separated out.
const SERUM_RGB: [f32; 3] = [0.88, 0.82, 0.58];

/// Heights above the floor, so coplanar discs cannot z-fight.
const Y_HALO: f32 = 0.0004;
const Y_DISC: f32 = 0.0008;
const Y_RIM: f32 = 0.0012;
const Y_CRACK: f32 = 0.0016;

/// The humidity stops. Two below the phase-separation threshold and two above, so the refusal is
/// one keypress away in either direction.
const HUMIDITIES: [f32; 4] = [0.30, 0.45, 0.55, 0.85];

/// Ticks of age `Space` adds per fixed tick. The pools dry in 30 s and 60 s of model time; at 4x
/// that is 7.5 s and 15 s of watching, and the readout says so rather than pretending.
const PLAY_STEP: u32 = 4;
/// Ticks per second the arrows scrub, so a full sweep of the longer pool takes about five seconds.
const SCRUB_PER_SEC: f32 = 700.0;

/// Crack count at full craquelure.
const MAX_CRACKS: u32 = 14;

/// The age axis, the humidity, and the blood the whole demo reads.
#[derive(Resource)]
struct Scrub {
    /// Age in ticks, held as a float only so the scrub is smooth.
    age: f32,
    playing: bool,
    /// Cursor into [`HUMIDITIES`]. **The value itself lives in `blood.humidity`** — this is only
    /// where the next `H` press comes from.
    stop: usize,
    blood: BloodSettings,
    /// The longest drying span of any pool: the end of the axis.
    span: u32,
}

impl Default for Scrub {
    fn default() -> Self {
        // Start at 0.45: below the threshold, so the very first `H` press is the halo appearing.
        let stop = 1;
        let humidity = HUMIDITIES[stop.min(HUMIDITIES.len() - 1)];
        let blood = BloodSettings { humidity, ..Default::default() };
        let span = AREAS.iter().map(|a| dry_ticks(*a, HZ)).max().unwrap_or(1).max(1);
        Scrub { age: 0.0, playing: false, stop, blood, span }
    }
}

impl Scrub {
    fn age_ticks(&self) -> u32 {
        self.age.max(0.0).round() as u32
    }
}

/// The prebuilt rim annuli, indexed by front step.
#[derive(Resource)]
struct Rims(Vec<Handle<Mesh>>);

/// The pool disc — index into [`AREAS`].
#[derive(Component)]
struct Disc(usize);
/// The matte ring growing inward from that pool's edge.
#[derive(Component)]
struct Rim(usize);
/// The serum ring outside it.
#[derive(Component)]
struct Halo(usize);
/// A pool's readout panel, tinted to its live colour.
#[derive(Component)]
struct Panel(usize);
/// The five channels of that pool, as numbers and bars.
#[derive(Component)]
struct PoolText(usize);
/// The age / humidity / play line.
#[derive(Component)]
struct Readout;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bevy_carnage - drying".into(),
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_hz(HZ as f64))
        .init_resource::<Scrub>()
        .add_systems(Startup, setup)
        .add_systems(Update, (keys, paint, panels, readout, cracks).chain())
        .add_systems(FixedUpdate, play)
        .run();
}

/// A pool's radius from its area. The area is the authored quantity because that is what
/// [`bevy_carnage::blood::dry::dry_ticks`] scales with.
fn radius(area: f32) -> f32 {
    (area / std::f32::consts::PI).sqrt()
}

/// Where a pool sits, or `None` for an index no pool has — so a stale component cannot panic.
fn centre(index: usize) -> Option<Vec3> {
    CENTRES.get(index).map(|x| Vec3::new(*x, 0.0, 0.0))
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    scrub: Res<Scrub>,
) {
    let look = Vec3::new(0.01, 0.0, 0.0);
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.01, 0.17, 0.18).looking_at(look, Vec3::Y),
    ));
    // **The key light is placed so its reflection off the floor lands in the camera.** Roughness is
    // the channel that carries the wetness claim, and a specular channel with no specular highlight
    // to move is invisible.
    commands.spawn((
        DirectionalLight { illuminance: 13_000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(0.01, 0.69, -0.73).looking_at(look, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 2_600.0, shadow_maps_enabled: false, ..default() },
        Transform::from_xyz(0.30, 0.50, 0.55).looking_at(look, Vec3::Y),
    ));
    // Kept low so the highlight has somewhere to be brighter than.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.58, 0.62, 0.74),
        brightness: 260.0,
        ..default()
    });

    let floor = meshes.add(Mesh::from(Plane3d::default().mesh().size(2.0, 2.0)));
    let floor_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.17, 0.17, 0.19),
        perceptual_roughness: 0.90,
        ..default()
    });
    commands.spawn((Mesh3d(floor), MeshMaterial3d(floor_mat)));

    // One annulus per rim step, unit outer radius, scaled per pool. Step 0 is degenerate and is
    // hidden rather than drawn: at age zero there is no dry edge at all.
    let flat = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    let rim_zero = meshes.add(Mesh::from(Annulus::new(0.999, 1.0)));
    let mut rims: Vec<Handle<Mesh>> = vec![rim_zero.clone()];
    rims.extend((1..=RIM_STEPS).map(|step| {
        let inner = (1.0 - RIM_DEPTH * step as f32 / RIM_STEPS as f32).min(0.999);
        meshes.add(Mesh::from(Annulus::new(inner, 1.0)))
    }));
    let halo_mesh = meshes.add(Mesh::from(Annulus::new(1.0, HALO_OUTER)));

    for (i, area) in AREAS.iter().enumerate() {
        let Some(c) = centre(i) else { continue };
        let r = radius(*area);
        let disc = meshes.add(Mesh::from(Circle::new(r)));

        let fresh = Color::srgb(SRGB_OXY[0], SRGB_OXY[1], SRGB_OXY[2]);
        let disc_mat = materials.add(StandardMaterial {
            base_color: fresh,
            perceptual_roughness: scrub.blood.wet_roughness,
            ..default()
        });
        commands.spawn((
            Disc(i),
            Mesh3d(disc),
            MeshMaterial3d(disc_mat),
            Transform::from_translation(c + Vec3::Y * Y_DISC).with_rotation(flat),
        ));

        // The dry rim: same blood, same colour, fully matte. Only the roughness differs, which is
        // exactly the claim.
        let rim_mat = materials.add(StandardMaterial {
            base_color: fresh,
            perceptual_roughness: scrub.blood.dry_roughness,
            ..default()
        });
        commands.spawn((
            Rim(i),
            Mesh3d(rim_zero.clone()),
            MeshMaterial3d(rim_mat),
            Transform::from_translation(c + Vec3::Y * Y_RIM).with_rotation(flat).with_scale(
                Vec3::splat(r),
            ),
            Visibility::Hidden,
        ));

        let halo_mat = materials.add(StandardMaterial {
            base_color: Color::srgba(SERUM_RGB[0], SERUM_RGB[1], SERUM_RGB[2], 0.0),
            perceptual_roughness: 0.55,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        commands.spawn((
            Halo(i),
            Mesh3d(halo_mesh.clone()),
            MeshMaterial3d(halo_mat),
            Transform::from_translation(c + Vec3::Y * Y_HALO).with_rotation(flat).with_scale(
                Vec3::splat(r),
            ),
            Visibility::Hidden,
        ));
    }
    commands.insert_resource(Rims(rims));

    build_ui(&mut commands);
}

fn build_ui(commands: &mut Commands) {
    let font = TextFont { font_size: FontSize::Px(14.0), ..default() };
    let pale = TextColor(Color::srgba(1.0, 1.0, 1.0, 0.88));

    commands.spawn((
        Text::new(format!(
            "bevy_carnage - DRYING: one scalar age, five channels\n\
             \n\
             left / right   scrub the age\n\
             H              humidity\n\
             Space          play\n\
             \n\
             The colour walks three stops, not two -- which is why old blood is a brown\n\
             rather than a dark red:  oxyHb {:.2} {:.2} {:.2}  ->  metHb {:.2} {:.2} {:.2}\n\
             ->  hemichrome {:.2} {:.2} {:.2}\n\
             The rim dries first: matte at the edge, still glossy in the middle.\n\
             A serum halo appears OUTSIDE the pool only at or above {:.0}% humidity --\n\
             below that there is none at all, at any age. Gloss, not hue, is the strongest\n\
             cue, so wetness is a specular channel here. Craquelure cracks the crust past\n\
             t = {:.2}.  The two pools differ ONLY in area, and dry_ticks scales with it.",
            SRGB_OXY[0], SRGB_OXY[1], SRGB_OXY[2],
            SRGB_MET[0], SRGB_MET[1], SRGB_MET[2],
            SRGB_HEMI[0], SRGB_HEMI[1], SRGB_HEMI[2],
            HALO_HUMIDITY * 100.0,
            CRAQUELURE_ONSET,
        )),
        font.clone(),
        pale,
        Node { position_type: PositionType::Absolute, top: px(12), left: px(14), ..default() },
    ));

    commands.spawn((
        Readout,
        Text::new(""),
        font.clone(),
        TextColor(Color::srgba(1.0, 0.94, 0.72, 0.95)),
        Node { position_type: PositionType::Absolute, top: px(12), right: px(14), ..default() },
    ));

    for i in 0..AREAS.len() {
        let mut root = Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            padding: px(6).all(),
            ..default()
        };
        if i == 0 {
            root.left = px(14);
        } else {
            root.right = px(14);
        }
        // The panel is tinted to the pool's live colour, so the channel that is a colour is shown
        // behind its own numbers instead of only printed.
        commands.spawn((Panel(i), root, BackgroundColor(Color::NONE))).with_children(|b| {
            b.spawn((PoolText(i), Text::new(""), font.clone(), pale));
        });
    }
}

fn keys(keys: Res<ButtonInput<KeyCode>>, time: Res<Time>, mut s: ResMut<Scrub>) {
    let dt = time.delta_secs();
    if keys.pressed(KeyCode::ArrowRight) {
        s.age += dt * SCRUB_PER_SEC;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        s.age -= dt * SCRUB_PER_SEC;
    }
    let span = s.span as f32;
    s.age = s.age.clamp(0.0, span);

    if keys.just_pressed(KeyCode::KeyH) {
        s.stop = (s.stop + 1) % HUMIDITIES.len();
        if let Some(h) = HUMIDITIES.get(s.stop) {
            s.blood.humidity = *h;
        }
    }
    if keys.just_pressed(KeyCode::Space) {
        s.playing = !s.playing;
    }
}

/// Play the age forward in **integer ticks**, and loop the axis rather than stopping on it.
///
/// The loop is a property of the scrub, not of the blood: every channel
/// [`bevy_carnage::blood::dry::appearance`] returns is monotone in age, and this file never asks
/// it for anything else.
fn play(mut s: ResMut<Scrub>) {
    if !s.playing {
        return;
    }
    let next = s.age_ticks().saturating_add(PLAY_STEP);
    s.age = if next > s.span { 0.0 } else { next as f32 };
}

/// **The five channels into the material and the geometry**, and nothing in between.
fn paint(
    scrub: Res<Scrub>,
    rims: Res<Rims>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    discs: Query<(&Disc, &MeshMaterial3d<StandardMaterial>)>,
    mut rings: Query<
        (&Rim, &mut Mesh3d, &mut Visibility, &MeshMaterial3d<StandardMaterial>),
        Without<Halo>,
    >,
    mut halos: Query<(&Halo, &mut Visibility, &MeshMaterial3d<StandardMaterial>), Without<Rim>>,
) {
    let age = scrub.age_ticks();
    for (disc, skin) in &discs {
        let Some(area) = AREAS.get(disc.0) else { continue };
        let a = appearance(age, HZ, *area, &scrub.blood);
        if let Some(mut mat) = materials.get_mut(&skin.0) {
            mat.base_color = Color::srgb(a.srgb[0], a.srgb[1], a.srgb[2]);
            mat.perceptual_roughness = a.roughness;
        }
    }
    for (rim, mut mesh, mut vis, skin) in &mut rings {
        let Some(area) = AREAS.get(rim.0) else { continue };
        let a = appearance(age, HZ, *area, &scrub.blood);
        let step = (a.rim.clamp(0.0, 1.0) * RIM_STEPS as f32).round() as usize;
        let want = if step == 0 { Visibility::Hidden } else { Visibility::Visible };
        if *vis != want {
            *vis = want;
        }
        if let Some(handle) = rims.0.get(step) {
            if mesh.0 != *handle {
                mesh.0 = handle.clone();
            }
        }
        // The ring is the *dried* edge: the pool's colour at this age, at full dry roughness.
        if let Some(mut mat) = materials.get_mut(&skin.0) {
            mat.base_color = Color::srgb(a.srgb[0], a.srgb[1], a.srgb[2]);
            mat.perceptual_roughness = scrub.blood.dry_roughness;
        }
    }
    for (halo, mut vis, skin) in &mut halos {
        let Some(area) = AREAS.get(halo.0) else { continue };
        let a = appearance(age, HZ, *area, &scrub.blood);
        // **Exactly zero is exactly hidden.** Below the phase-separation threshold there is no
        // serum outside the pool at all, so there is nothing to draw very faintly.
        let want = if a.halo > 0.0 { Visibility::Visible } else { Visibility::Hidden };
        if *vis != want {
            *vis = want;
        }
        if let Some(mut mat) = materials.get_mut(&skin.0) {
            mat.base_color =
                Color::srgba(SERUM_RGB[0], SERUM_RGB[1], SERUM_RGB[2], a.halo.clamp(0.0, 1.0) * 0.8);
        }
    }
}

/// The late crack network, drawn as a deterministic set of polylines from the frozen generator.
fn cracks(mut gizmos: Gizmos, scrub: Res<Scrub>) {
    let age = scrub.age_ticks();
    for (i, area) in AREAS.iter().enumerate() {
        let a = appearance(age, HZ, *area, &scrub.blood);
        if a.craquelure <= 0.0 {
            continue;
        }
        let Some(c) = centre(i) else { continue };
        let r = radius(*area);
        let n = (a.craquelure * MAX_CRACKS as f32).round() as u32;
        let colour = Color::srgba(0.10, 0.04, 0.03, (0.30 + 0.70 * a.craquelure).min(1.0));
        for k in 0..n {
            let seed = ((i as u32) << 16) | k;
            let mut heading = hash_f32(seed ^ 0x1234_5678) * std::f32::consts::TAU;
            let mut p = c + Vec3::new(heading.cos(), 0.0, heading.sin()) * r * 0.10;
            let mut pts = Vec::with_capacity(6);
            pts.push(p + Vec3::Y * Y_CRACK);
            for step in 1..6u32 {
                heading += (hash_f32(seed ^ (step * 7919)) - 0.5) * 0.9;
                p += Vec3::new(heading.cos(), 0.0, heading.sin()) * r * 0.2;
                let out = p - c;
                let far = out.length();
                if far > r * 0.95 {
                    p = c + out / far.max(1.0e-6) * r * 0.95;
                    pts.push(p + Vec3::Y * Y_CRACK);
                    break;
                }
                pts.push(p + Vec3::Y * Y_CRACK);
            }
            gizmos.linestrip(pts, colour);
        }
    }
}

/// A ten-cell bar, the same shape `bloodstain`'s own `dry_timeline` example prints — the default
/// font is FiraMono, so a column of these lines up.
fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 10.0).round() as usize;
    (0..10).map(|i| if i < n { '#' } else { '.' }).collect()
}

/// Which pair of colour stops the pool is between, and how far.
fn stop_label(t: f32) -> String {
    if t < 0.35 {
        format!("oxyHb->metHb {:>3.0}%", (t / 0.35).clamp(0.0, 1.0) * 100.0)
    } else {
        format!("metHb->hemi  {:>3.0}%", ((t - 0.35) / 0.65).clamp(0.0, 1.0) * 100.0)
    }
}

/// **The five channels as numbers and bars**, plus the panel behind them tinted to the live colour,
/// so the colour channel is shown rather than only printed.
fn panels(
    scrub: Res<Scrub>,
    mut texts: Query<(&PoolText, &mut Text)>,
    mut tints: Query<(&Panel, &mut BackgroundColor)>,
) {
    let age = scrub.age_ticks();
    let (wet, dry) = (scrub.blood.wet_roughness, scrub.blood.dry_roughness);
    for (panel, mut text) in &mut texts {
        let Some(area) = AREAS.get(panel.0) else { continue };
        let a = appearance(age, HZ, *area, &scrub.blood);
        let span = dry_ticks(*area, HZ);
        let t = (age as f32 / span as f32).clamp(0.0, 1.0);
        // Roughness is barred across the wet-to-dry span, since that is the range it can occupy.
        let rough_f = if dry > wet { (a.roughness - wet) / (dry - wet) } else { 0.0 };
        let name = NAMES.get(panel.0).copied().unwrap_or("pool");
        let (cm2, secs, stop) = (area * 1.0e4, span as f32 / HZ as f32, stop_label(t));
        let (r, g, b) = (a.srgb[0], a.srgb[1], a.srgb[2]);
        let (rough, rim, halo, crq) = (a.roughness, a.rim, a.halo, a.craquelure);
        // Exactly zero is named, not barred: below the threshold there is no serum outside the pool
        // at all, so there is no very small bar either.
        let halo_bar = if halo > 0.0 { format!("[{}]", bar(halo)) } else { "absent".into() };
        let next = format!(
            "{name}   {cm2:.0} cm2   dry span {span} t ({secs:.1} s)   t = {t:.2}\n\
             colour      {r:.2} {g:.2} {b:.2}   {stop}\n\
             roughness   {rough:.2}   [{}]\n\
             rim         {rim:.2}   [{}]\n\
             halo        {halo:.2}   {halo_bar}\n\
             craquelure  {crq:.2}   [{}]",
            bar(rough_f),
            bar(rim),
            bar(crq),
        );
        if text.0 != next {
            text.0 = next;
        }
    }
    for (panel, mut tint) in &mut tints {
        let Some(area) = AREAS.get(panel.0) else { continue };
        let a = appearance(age, HZ, *area, &scrub.blood);
        let want = Color::srgba(a.srgb[0], a.srgb[1], a.srgb[2], 0.55);
        if tint.0 != want {
            tint.0 = want;
        }
    }
}

fn readout(scrub: Res<Scrub>, mut out: Query<&mut Text, With<Readout>>) {
    let age = scrub.age_ticks();
    let (span, secs) = (scrub.span, age as f32 / HZ as f32);
    let (rh, thr) = (scrub.blood.humidity, HALO_HUMIDITY);
    let verdict = if rh >= thr { "halo forms" } else { "NO halo, at any age" };
    let play = if scrub.playing {
        format!("playing, {PLAY_STEP} ticks per tick")
    } else {
        "paused -- Space plays".to_string()
    };
    let text = format!(
        "age {age:>4} t of {span} ({secs:>5.2} s)\n\
         humidity {rh:.2}   serum halo threshold {thr:.2}   {verdict}\n\
         {play}",
    );
    for mut t in &mut out {
        if t.0 != text {
            t.0 = text.clone();
        }
    }
}
