//! **The drying timeline as a table**: colour, gloss, the rim front, the serum halo and the cracks.
//!
//! Terminal only. Two humidities side by side, because the serum halo is a *threshold* — below about
//! 50 % relative humidity it never forms at all, and that is a fact about the fluid rather than a
//! knob to taste.
//!
//! ```sh
//! cargo run -p bloodstain --example dry_timeline
//! ```

use bevy_carnage::bloodstain::dry::{DRY_REF_AREA_M2, appearance, dry_ticks};
use bevy_carnage::bloodstain::{BloodSettings, rheo};

const HZ: u32 = 60;

/// A five-level bar, so a column of numbers also reads as a shape.
fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 10.0).round() as usize;
    let mut s = String::with_capacity(10);
    for i in 0..10 {
        s.push(if i < n { '#' } else { '.' });
    }
    s
}

fn main() {
    let dry_air = BloodSettings { humidity: 0.40, ..Default::default() };
    let humid = BloodSettings { humidity: 0.85, ..Default::default() };
    let area = DRY_REF_AREA_M2;
    let span = dry_ticks(area, HZ);

    println!(
        "bloodstain: the drying timeline\n\
         \n\
         pool area {:.1} cm², drying span {span} ticks ({:.0} s at {HZ} Hz)\n\
         colour walks oxyHb -> metHb -> hemichrome (Bremmer, doi:10.1016/j.forsciint.2011.07.027)\n\
         the rim dries first (Smith, doi:10.1038/s41598-020-65465-4); all masses collapse onto one\n\
         curve (Laan, doi:10.1016/j.forsciint.2016.08.005)\n\
         GLOSS is the strongest cue and it is not a colour (Oum, doi:10.1080/02699931.2010.496997)\n",
        area * 1.0e4,
        span as f32 / HZ as f32,
    );

    println!(
        "  {:>5}  {:>5}  {:>16}  {:>6}  {:<12}  {:<12}  {:<12}  halo@85%",
        "tick", "t", "sRGB", "rough", "rim", "halo@40%", "craquelure"
    );

    for k in 0..=10u32 {
        let tick = span * k / 10;
        let a = appearance(tick, HZ, area, &dry_air);
        let wet = appearance(tick, HZ, area, &humid);
        println!(
            "  {tick:>5}  {:>5.2}  {:>4.2} {:>4.2} {:>4.2}  {:>6.2}  {}  {}  {}  {}",
            tick as f32 / span as f32,
            a.srgb[0],
            a.srgb[1],
            a.srgb[2],
            a.roughness,
            bar(a.rim),
            bar(a.halo),
            bar(a.craquelure),
            bar(wet.halo),
        );
    }

    println!(
        "\nNote the halo column at 40 % humidity: exactly zero, at every age. Below the phase-\n\
         separation threshold the serum never leaves the wetted edge, so this is a refusal rather\n\
         than a very small number."
    );

    // And the other half of the material model: the same blood, arresting.
    println!(
        "\nthe same blood, as a yield-stress fluid — driving stress against yield stress\n\
         (a wound's clot and a rivulet arresting on a wall are one mechanism)\n"
    );
    println!("  {:>5}  {:>12}  {:>12}  {}", "tick", "driving Pa", "yield Pa", "flows?");
    let s = BloodSettings::default();
    for k in 0..=10u32 {
        let age = s.clot_ticks * k / 10;
        let driving = rheo::PERFUSION_STRESS_PA * bevy_carnage::bloodstain::bleed::envelope(age, &s);
        let y = rheo::yield_stress(age, HZ, &s);
        println!(
            "  {age:>5}  {driving:>12.3}  {y:>12.3}  {}",
            if rheo::flows(driving, y) { "yes" } else { "ARRESTED" }
        );
    }

    println!(
        "\nViscosity at three shear rates, Carreau-Yasuda with Cho & Kensey's constants:\n\
         \n  at rest   {:.4} Pa·s\n  walking   {:.4} Pa·s\n  arterial  {:.4} Pa·s\n\
         \nThat fall is why a fast rivulet is thin and races while a slow one thickens and beads.",
        rheo::viscosity(0.0, s.hematocrit, &s),
        rheo::viscosity(10.0, s.hematocrit, &s),
        rheo::viscosity(1000.0, s.hematocrit, &s),
    );
}
