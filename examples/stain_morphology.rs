//! **A wall of stains, each one rasterised from its own impact.**
//!
//! ```text
//!   left / right   impact angle
//!   up / down      impact speed
//!   R              substrate roughness
//!   B              the old four-splat wall, beside it
//! ```
//!
//! # What the wall is showing
//!
//! Every cell is a *different impact* and nothing else changes: one 4 mm droplet, swept across the
//! impact angle left to right and across the impact speed top to bottom. Each cell rasterises its own
//! silhouette through [`bloodstain::stain::rasterise`](bevy_carnage::rasterise) into its own texture,
//! so what is on screen is the model's output rather than an artist's guess at it. Three published
//! rules are visible at once:
//!
//! - **The aspect ratio is `sin θ`** — the impact-angle relation the whole of bloodstain-pattern
//!   analysis rests on (Hulse-Smith et al., `doi:10.1520/jfs2003224`). A perpendicular hit is a disc;
//!   a 15° hit is a lance. The readout prints the measured ratio beside `sin θ` so the claim can be
//!   checked rather than believed.
//! - **Spines come from `0.76 · We^0.5 · sin³θ`** — Knock & Davison's angle-inclusive law at R² ≈ 0.9
//!   (`doi:10.1111/j.1556-4029.2007.00505.x`). The cube of `sin θ` is why a shallow, fast impact is
//!   smooth-rimmed while a slower perpendicular one is not.
//! - **Satellites appear past Mundo's splash threshold `K = We^0.5 · Re^0.25 ≥ 57.7`**, and `Re`
//!   is taken at the shear-thinned viscosity the impact itself implies.
//!
//! `R` walks the substrate: a rough surface pins the advancing edge, shortening the stain and merging
//! neighbouring spines (Adam, `doi:10.1016/j.forsciint.2011.12.002`).
//!
//! # `B` is the argument, and it is built rather than asserted
//!
//! `B` draws the *same* wall the old way — four baked masks picked by `seed % 4`, which is what this
//! crate shipped before `0.2.0` — beside the derived one, and prints the arithmetic that killed it.
//! The probability that `n` consecutive stains drawn from `n` masks contain a repeat is `1 − n!/nⁿ`;
//! for four masks that is **90.6 %**, so the fourth stain is where a repeat is expected. The number on
//! screen is computed from the variant count in [`OLD_VARIANTS`], and the *first actual repeat* in the
//! wall is found by scanning it, ringed in orange, and reported by index. Authoring more textures
//! moves the number and not the shape of the problem.
//!
//! No asset files, no `vfx` feature: the masks are generated at startup and drawn as `bevy_ui`
//! images, so this runs identically in a browser and on a desktop.
//!
//! Run: `cargo run --release -p bevy_carnage --example stain_morphology`

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use bevy_carnage::BloodSettings;
use bevy_carnage::blood::dry::appearance;
use bevy_carnage::blood::rheo::viscosity;
use bevy_carnage::blood::stain::{
    Impact, SPINE_COEFF, SPINE_WE_MIN, SPLASH_K_CRIT, StainShape, rasterise, reynolds, stain_shape,
    weber,
};

/// Impact angles across the wall, degrees. Descending, so `right` is the shallower hit.
const ANGLES_DEG: [f32; 6] = [90.0, 75.0, 60.0, 45.0, 30.0, 15.0];
/// Impact speeds down the wall, m/s. Descending, so `up` is the faster hit.
const SPEEDS: [f32; 5] = [40.0, 20.0, 10.0, 5.0, 2.0];
/// Substrate roughness stops `R` cycles. Starts on the shipped `substrate_roughness`, index 1.
const ROUGHNESS: [f32; 4] = [0.0, 0.2, 0.5, 0.8];
/// The one droplet the whole wall is swept from, metres — 4 mm, matching `bloodstain`'s own
/// `stain_sweep` terminal example, so the two demos can be read against each other.
const DROPLET_M: f32 = 0.004;
/// Texels per side of a mask. Matches `bevy_carnage::decal`'s `MASK_SIZE`: enough that spines are not
/// visibly stepped, small enough that thirty of them are free to build.
const PX: u32 = 64;
/// Drawn size of one cell, logical pixels.
const CELL: f32 = 66.0;
/// Width of the speed labels down the left of each wall.
const LABEL_W: f32 = 58.0;
/// The fixed-tick rate the drying model's age is quoted in. Only age 0 is asked for here.
const HZ: u32 = 60;
/// Area the mask's colour is taken at, m² — a stain-sized patch, so `appearance` returns the sRGB of
/// *fresh* blood rather than of a pool.
const STAIN_AREA_M2: f32 = 1.0e-3;

/// **The four baked masks this crate shipped before `0.2.0`, as data.**
///
/// A baked texture cannot know the impact that made it, so none of these carries an aspect ratio, a
/// travel direction or a splash: they are four fixed silhouettes, and a stain was assigned one by
/// `seed % 4`. That is the whole scheme, reproduced here so the comparison is real.
const OLD_VARIANTS: [StainShape; 4] = [
    StainShape { major: 0.010, minor: 0.010, spines: 5, satellites: 0, direction: [1.0, 0.0], seed: 0x11 },
    StainShape { major: 0.010, minor: 0.010, spines: 9, satellites: 2, direction: [1.0, 0.0], seed: 0x22 },
    StainShape { major: 0.010, minor: 0.010, spines: 3, satellites: 0, direction: [1.0, 0.0], seed: 0x33 },
    StainShape { major: 0.010, minor: 0.010, spines: 13, satellites: 4, direction: [1.0, 0.0], seed: 0x44 },
];

/// The key legend, and it says the same thing as `web/play.html`'s `notes-stain_morphology` block.
///
/// ASCII only: Bevy 0.19's default font carries 95 codepoints, so an arrow glyph or a `θ` would draw
/// as nothing at all.
const LEGEND: &str = "LEFT / RIGHT  impact angle     UP / DOWN  speed     \
                      R  substrate roughness     B  the old four-splat wall";

/// A cell of the derived wall, by `(speed row, angle column)`.
#[derive(Component)]
struct DerivedCell(usize, usize);

/// The block holding the old four-splat wall. Toggled by `display`, not by `Visibility`, so hiding it
/// gives its space back to the derived wall instead of leaving a hole.
#[derive(Component)]
struct OldBlock;

/// The line reporting the highlighted cell's numbers.
#[derive(Component)]
struct Readout;

/// What the sweep is currently looking at.
#[derive(Resource)]
struct Wall {
    settings: BloodSettings,
    /// Fresh blood's sRGB bytes, taken from the drying model at age zero.
    rgb: [u8; 3],
    sel_row: usize,
    sel_col: usize,
    rough: usize,
    show_old: bool,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "stain_morphology".into(),
                canvas: Some("#carnage-canvas".into()),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, setup)
        .add_systems(Update, (sweep, hud).chain())
        .run();
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn(Camera2d);

    let settings = BloodSettings::default();
    // **Blood's colour comes from the drying model at age zero**, not from a swatch authored here:
    // `examples/drying` walks the same curve forward, so the two demos cannot disagree about what
    // fresh blood looks like.
    let fresh = appearance(0, HZ, STAIN_AREA_M2, &settings);
    let rgb = [byte(fresh.srgb[0]), byte(fresh.srgb[1]), byte(fresh.srgb[2])];
    let wall = Wall { settings, rgb, sel_row: 2, sel_col: 3, rough: 1, show_old: false };
    let rough = roughness_of(&wall);

    let dim = TextColor(Color::srgb(0.55, 0.55, 0.60));
    let body = TextColor(Color::srgb(0.86, 0.86, 0.90));
    let keys = TextColor(Color::srgb(0.98, 0.72, 0.42));

    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(12.0)),
                row_gap: Val::Px(10.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.055, 0.055, 0.065)),
        ))
        .with_children(|root| {
            root.spawn((Text::new(LEGEND), font(15.0), keys));
            root.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(28.0), ..default() })
                .with_children(|walls| {
                    walls
                        .spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(3.0), ..default() })
                        .with_children(|block| {
                            block.spawn((
                                Text::new("derived: one silhouette per impact"),
                                font(13.0),
                                body,
                            ));
                            block.spawn(row()).with_children(|hdr| {
                                hdr.spawn((Node { width: Val::Px(LABEL_W), ..default() },));
                                for &deg in ANGLES_DEG.iter() {
                                    hdr.spawn((
                                        Text::new(format!("{deg:.0} deg")),
                                        label(CELL),
                                        font(11.0),
                                        dim,
                                    ));
                                }
                            });
                            for (r, &speed) in SPEEDS.iter().enumerate() {
                                block.spawn(row()).with_children(|line| {
                                    line.spawn((
                                        Text::new(format!("{speed:.0} m/s")),
                                        label(LABEL_W),
                                        font(11.0),
                                        dim,
                                    ));
                                    for (c, &deg) in ANGLES_DEG.iter().enumerate() {
                                        let shape = stain_shape(
                                            &impact_of(speed, deg, rough),
                                            &wall.settings,
                                            cell_seed(r, c),
                                        );
                                        let image = images.add(mask_image(&shape, wall.rgb));
                                        line.spawn((
                                            cell(),
                                            ImageNode::new(image),
                                            BorderColor::all(Color::NONE),
                                            DerivedCell(r, c),
                                        ));
                                    }
                                });
                            }
                        });

                    // The old wall, and the arithmetic that retired it.
                    let baked: Vec<Handle<Image>> = OLD_VARIANTS
                        .iter()
                        .map(|shape| images.add(mask_image(shape, wall.rgb)))
                        .collect();
                    let repeat = first_repeat();
                    walls
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(3.0),
                                display: Display::None,
                                ..default()
                            },
                            OldBlock,
                        ))
                        .with_children(|block| {
                            block.spawn((
                                Text::new(format!("the old way: {} baked masks by seed % {}", OLD_VARIANTS.len(), OLD_VARIANTS.len())),
                                font(13.0),
                                body,
                            ));
                            block.spawn(row()).with_children(|hdr| {
                                for &deg in ANGLES_DEG.iter() {
                                    hdr.spawn((Text::new(format!("{deg:.0} deg")), label(CELL), font(11.0), dim));
                                }
                            });
                            let mut index = 0usize;
                            for (r, _) in SPEEDS.iter().enumerate() {
                                block.spawn(row()).with_children(|line| {
                                    for (c, _) in ANGLES_DEG.iter().enumerate() {
                                        index += 1;
                                        let variant = variant_of(r, c);
                                        // Unreachable while `variant_of` honours the array length,
                                        // and still not an index: a resolved variant that went out
                                        // of range must not take the process down.
                                        let Some(image) = baked.get(variant) else { continue };
                                        let ring = if repeat == Some(index) {
                                            Color::srgb(0.98, 0.55, 0.15)
                                        } else {
                                            Color::NONE
                                        };
                                        line.spawn((
                                            cell(),
                                            ImageNode::new(image.clone()),
                                            BorderColor::all(ring),
                                        ));
                                    }
                                });
                            }
                            block.spawn((Text::new(arithmetic(repeat)), font(12.0), dim));
                        });
                });
            root.spawn((Text::new(String::new()), font(13.0), body, Readout));
        });

    commands.insert_resource(wall);
}

/// Arrow keys move the highlighted cell, `R` walks the substrate, `B` shows the old wall.
fn sweep(
    keys: Res<ButtonInput<KeyCode>>,
    mut wall: ResMut<Wall>,
    mut images: ResMut<Assets<Image>>,
    mut cells: Query<(&DerivedCell, &mut ImageNode)>,
    mut old: Query<&mut Node, With<OldBlock>>,
) {
    let last = ANGLES_DEG.len().saturating_sub(1);
    let bottom = SPEEDS.len().saturating_sub(1);
    let (mut row, mut col) = (wall.sel_row, wall.sel_col);
    if keys.just_pressed(KeyCode::ArrowLeft) {
        col = col.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        col = (col + 1).min(last);
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        row = row.saturating_sub(1);
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        row = (row + 1).min(bottom);
    }
    if (row, col) != (wall.sel_row, wall.sel_col) {
        wall.sel_row = row;
        wall.sel_col = col;
    }

    if keys.just_pressed(KeyCode::KeyR) {
        wall.rough = (wall.rough + 1) % ROUGHNESS.len();
        let rough = roughness_of(&wall);
        // A new texture per cell rather than a write into the old one: a fresh handle is
        // unambiguously uploaded, and the masks are cheap enough that thirty of them is not a cost
        // worth being clever about.
        for (spot, mut node) in &mut cells {
            let (Some(&speed), Some(&deg)) = (SPEEDS.get(spot.0), ANGLES_DEG.get(spot.1)) else {
                continue;
            };
            let shape = stain_shape(&impact_of(speed, deg, rough), &wall.settings, cell_seed(spot.0, spot.1));
            node.image = images.add(mask_image(&shape, wall.rgb));
        }
    }

    if keys.just_pressed(KeyCode::KeyB) {
        wall.show_old = !wall.show_old;
        let display = if wall.show_old { Display::Flex } else { Display::None };
        for mut node in &mut old {
            node.display = display;
        }
    }
}

/// Ring the highlighted cell and print its numbers against the three laws.
fn hud(
    wall: Res<Wall>,
    mut cells: Query<(&DerivedCell, &mut BorderColor)>,
    mut readout: Query<&mut Text, With<Readout>>,
) {
    // Unconditional, deliberately: gating on `wall.is_changed()` would leave the readout blank until
    // the first keypress if the insert and the first `Update` ever fell on different frames, and one
    // `format!` a frame for thirty cells is not a cost worth that risk.
    for (spot, mut border) in &mut cells {
        *border = if (spot.0, spot.1) == (wall.sel_row, wall.sel_col) {
            BorderColor::all(Color::srgb(0.42, 0.86, 0.98))
        } else {
            BorderColor::all(Color::NONE)
        };
    }

    let (Some(&speed), Some(&deg)) = (SPEEDS.get(wall.sel_row), ANGLES_DEG.get(wall.sel_col)) else {
        return;
    };
    let s = &wall.settings;
    let rough = roughness_of(&wall);
    let impact = impact_of(speed, deg, rough);
    let shape = stain_shape(&impact, s, cell_seed(wall.sel_row, wall.sel_col));

    let we = weber(impact.diameter, impact.speed).max(0.0);
    let sin_t = impact.angle_rad.sin();
    // The same shear rate `stain_shape` reads, so the Reynolds number printed here is the one the
    // splash test actually used rather than a second estimate of it.
    let mu = viscosity(impact.speed / impact.diameter, s.hematocrit, s);
    let re = reynolds(impact.diameter, impact.speed, mu).max(0.0);
    let k = we.sqrt() * re.powf(0.25);
    let law = SPINE_COEFF * we.sqrt() * sin_t * sin_t * sin_t;
    let aspect = if shape.major > 0.0 { shape.minor / shape.major } else { 0.0 };
    let verdict = if k >= SPLASH_K_CRIT { "splash" } else { "deposit" };

    let text = format!(
        "cell: {deg:.0} deg   {speed:.0} m/s   droplet {:.1} mm   substrate roughness {rough:.2}\n\
         aspect minor/major {aspect:.3}  vs  sin(theta) {sin_t:.3}   \
         [Hulse-Smith 10.1520/jfs2003224]\n\
         spines {}  vs  0.76*We^0.5*sin^3(theta) = {law:.1}   (none below We {SPINE_WE_MIN:.0})   \
         [Knock & Davison 10.1111/j.1556-4029.2007.00505.x]\n\
         satellites {}   K = We^0.5*Re^0.25 = {k:.1}  vs  SPLASH_K_CRIT {SPLASH_K_CRIT:.1}  ->  \
         {verdict}   [Mundo 1995]\n\
         We {we:.0}   Re {re:.0}   viscosity {:.2} mPa.s   major {:.2} mm   minor {:.2} mm",
        impact.diameter * 1000.0,
        shape.spines,
        shape.satellites,
        mu * 1000.0,
        shape.major * 1000.0,
        shape.minor * 1000.0,
    );
    for mut line in &mut readout {
        line.0 = text.clone();
    }
}

/// The impact one cell of the wall stands for. One droplet, one substrate, two swept variables.
fn impact_of(speed: f32, angle_deg: f32, roughness: f32) -> Impact {
    Impact {
        speed,
        diameter: DROPLET_M,
        angle_rad: angle_deg.to_radians(),
        roughness,
        // Travel along `+u`, so every cell's long axis runs left to right and the sweep is
        // comparable cell to cell rather than each stain pointing somewhere else.
        travel: [1.0, 0.0],
    }
}

/// A per-cell seed: the jitter key a real stain would carry, from a place rather than a counter.
fn cell_seed(row: usize, col: usize) -> u32 {
    ((row as u32).wrapping_mul(0x9E37_79B9) ^ (col as u32).wrapping_mul(0x85EB_CA6B))
        .wrapping_add(0x5EED)
}

/// Which baked mask the old scheme would have handed this cell: `seed % n`, exactly as it was.
fn variant_of(row: usize, col: usize) -> usize {
    let n = OLD_VARIANTS.len().max(1) as u32;
    (cell_seed(row, col) % n) as usize
}

/// **The first repeat in the old wall, in scan order**, by 1-based stain index.
///
/// Measured rather than asserted: the wall is walked, the variants are remembered, and the first cell
/// whose mask has already appeared is the answer. `None` only if the wall is smaller than the mask
/// count, which the birthday bound makes unlikely enough to be worth reporting honestly.
fn first_repeat() -> Option<usize> {
    let mut seen = [false; OLD_VARIANTS.len()];
    let mut index = 0usize;
    for (r, _) in SPEEDS.iter().enumerate() {
        for (c, _) in ANGLES_DEG.iter().enumerate() {
            index += 1;
            let slot = seen.get_mut(variant_of(r, c))?;
            if *slot {
                return Some(index);
            }
            *slot = true;
        }
    }
    None
}

/// The birthday arithmetic, computed from the variant count rather than quoted.
///
/// `1 − n!/nⁿ` is the probability that `n` independent draws from `n` options contain a repeat. For
/// the four masks this crate shipped that is 90.6 %, which is why the fourth stain is where a repeat
/// is expected — and why authoring a fifth texture moves the number and not the problem.
fn arithmetic(repeat: Option<usize>) -> String {
    let n = OLD_VARIANTS.len() as u32;
    let mut permutations = 1.0f64;
    for k in 1..=n {
        permutations *= f64::from(k);
    }
    let repeats = 1.0 - permutations / f64::from(n).powi(n as i32);
    let measured = match repeat {
        Some(k) => format!("in this wall the first repeat is stain #{k}"),
        None => "this wall happens not to repeat".to_string(),
    };
    format!(
        "P(a repeat among {n} consecutive stains) = 1 - {n}!/{n}^{n} = {:.1} %\n\
         so the {n}th stain is where a repeat is expected; {measured}\n\
         a derived stain repeats only when the impact repeats",
        repeats * 100.0
    )
}

/// Which substrate roughness the sweep is on.
fn roughness_of(wall: &Wall) -> f32 {
    ROUGHNESS.get(wall.rough).copied().unwrap_or(wall.settings.substrate_roughness)
}

/// **One stain's mask as a UI texture**: the silhouette in alpha, blood's colour in RGB.
///
/// `new_fill` then a write into `data`, matching `bevy_carnage::decal::mask_image` — the colour lives
/// in the texture here rather than in a material, because a `bevy_ui` image has no material to carry
/// it and the mask is the only thing on screen.
fn mask_image(shape: &StainShape, rgb: [u8; 3]) -> Image {
    let mut image = Image::new_fill(
        Extent3d { width: PX, height: PX, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        // sRGB, because the colour channels are sampled as colour; alpha is linear either way and it
        // is the alpha that carries the shape.
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );

    let side = PX as usize;
    let mut coverage = vec![0u8; side * side];
    // `rasterise` refuses a wrong-sized buffer rather than half-filling it. The buffer is built from
    // the same constant, so the refusal is unreachable — and a clear mask is the honest result of "no
    // coverage was computed" rather than garbage uploaded to the GPU.
    if !rasterise(shape, PX, &mut coverage) {
        warn!("rasterise refused a {PX}x{PX} buffer; that cell stays clear");
        return image;
    }
    let Some(data) = image.data.as_mut() else {
        warn!("a freshly filled image has no pixel data; that cell stays clear");
        return image;
    };
    for (texel, &cover) in data.chunks_exact_mut(4).zip(coverage.iter()) {
        texel[0] = rgb[0];
        texel[1] = rgb[1];
        texel[2] = rgb[2];
        texel[3] = cover;
    }
    image
}

/// A `[0, 1]` channel as a texture byte.
fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn font(size: f32) -> TextFont {
    TextFont { font_size: FontSize::Px(size), ..default() }
}

fn row() -> Node {
    Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(3.0), ..default() }
}

fn label(width: f32) -> Node {
    Node { width: Val::Px(width), ..default() }
}

fn cell() -> Node {
    Node {
        width: Val::Px(CELL),
        height: Val::Px(CELL),
        border: UiRect::all(Val::Px(2.0)),
        ..default()
    }
}
