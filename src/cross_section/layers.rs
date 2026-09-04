//! **How deep each tissue lies, by body region.** The table the bands are cut from.
//!
//! A cut face through a limb is not a uniform pink disc. Going inward from the skin it crosses the
//! dermis, a layer of subcutaneous fat, the muscle, the cortical shell of the bone and then the
//! marrow, and each of those has a thickness that has been measured on living adults with an
//! ultrasound probe — because needle lengths, insulin injections and body-composition estimates all
//! depend on the numbers. This module carries those numbers with their sources; the rest of the crate
//! only asks it "what is at `d` millimetres below the skin, here?"
//!
//! # Sources, per row
//!
//! **Skin (dermis + epidermis) and subcutaneous fat.** Akkus, Oguz, Uzunlulu & Kizilgul, *"Evaluation
//! of skin and subcutaneous adipose tissue thickness for optimal insulin injection"*, J. Diabetes
//! Metab. 3:8 (2012), `doi:10.4172/2155-6156.1000216` — ultrasound on 200 adults: skin **1.95 mm**
//! triceps, **2.35 mm** anterior abdomen, **1.97 mm** anterior thigh; subcutaneous fat **6.42 mm**
//! triceps, **15.73 mm** abdomen, **7.92 mm** thigh. Derraik et al., *"Effects of age, gender, BMI,
//! and anatomical site on skin thickness in children and adults with diabetes"*, PLoS ONE 9(1)
//! (2014), `doi:10.1371/journal.pone.0086637` — adult dermis **2.09 mm** abdomen and **1.86 mm**
//! thigh in men, subcutis **16.9 mm** abdomen and **9.3 mm** thigh. The two studies agree to a few
//! tenths on skin and the fat rows here are their means.
//!
//! **Muscle.** Abe, Loenneke & Thiebaud, *"Morphological and functional relationships with ultrasound
//! measured muscle thickness of the lower extremity"*, Ultrasound 23(3) (2015),
//! `doi:10.1177/1742271X14554678`, Table 1 — the cadaver-validated site thicknesses: triceps brachii
//! **22.5 mm**, forearm **15.8 mm**, abdomen **7.3 mm**, quadriceps **10.3 mm**, hamstring
//! **26.1 mm**. The limb row is the mean of the four limb sites; the torso row is the abdomen.
//!
//! **Head.** Facial soft-tissue thickness — everything between skin and bone at a landmark —
//! is the forensic-reconstruction quantity. Dimitrova et al., *"Facial soft tissue thicknesses in
//! Bulgarian adults: relation to sex, body mass index and bilateral asymmetry"*, Folia Morphol. 77(3)
//! (2018), `doi:10.5603/fm.a2017.0114`: glabella **5.5–5.9 mm**, nasion **7.9–8.0 mm**, gonion
//! **13–18 mm**. The head row totals **6.8 mm** to bone, the mean of the two midline landmarks,
//! and the split of that total into skin, fat and muscle is this crate's own — the paper measures
//! the sum, not the parts.
//!
//! **Cortical bone is not sourced.** No paper tabulating long-bone cortex thickness was in the
//! corpus this crate was written from, and the four candidates tried
//! (`doi:10.1073/pnas.1321605111`, `doi:10.1002/ar.20778`, `doi:10.1016/j.jchb.2009.07.001`,
//! `doi:10.1016/j.media.2010.01.003`) had no open-access copy. The `cortex_mm` values are stated
//! plainly as this crate's own and are a caller's to override.
//!
//! **Nor is the bone's outer diameter.** `bone_mm` is the same kind of number as `cortex_mm` and is
//! flagged the same way: **this crate's own**, 27 mm on a limb and 0 on the torso and the head.
//! Nothing in the corpus this crate was written from tabulates long-bone outer diameter, and the
//! value is a caller's to override. It is not a thickness — it does not enter `starts_mm`,
//! `span_mm`, `thickness_mm` or `at`, so no strip and no band moves because of it. It is read in
//! exactly one place, [`crate::cross_section::uv1_at_core`], where it makes a limb's bone a **core at the deepest
//! point of a cut face** rather than everything past the cortex start.

/// Where on the body a cut is, which decides which thickness row applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Region {
    /// Arm or leg: thin fat, thick muscle, a long bone with a wide marrow cavity.
    Limb,
    /// Trunk: the thickest fat on the body over a thin muscular wall.
    Torso,
    /// Head and face: a few millimetres of everything over a dense cortical shell.
    Head,
}

impl Region {
    /// Every region, in declaration order — for callers that bake one strip per region.
    pub const ALL: [Region; 3] = [Region::Limb, Region::Torso, Region::Head];
}

/// The tissue at a given depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Layer {
    /// Epidermis and dermis.
    Skin,
    /// Subcutaneous adipose tissue — lobules of fat in a fibrous net.
    Fat,
    /// Skeletal muscle, striated along the fibre direction.
    Muscle,
    /// The dense cortical shell of the bone.
    Cortex,
    /// The marrow cavity inside the cortex.
    Marrow,
}

impl Layer {
    /// Every layer, outside to inside.
    pub const ALL: [Layer; 5] = [Layer::Skin, Layer::Fat, Layer::Muscle, Layer::Cortex, Layer::Marrow];
}

/// **Layer thicknesses for one region, millimetres, outside to inside.**
///
/// The four finite layers are followed by marrow, which this type treats as extending to any depth;
/// `marrow_mm` is only how much of it a strip texture draws before it repeats.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layers {
    /// Dermis plus epidermis.
    pub skin_mm: f32,
    /// Subcutaneous fat.
    pub fat_mm: f32,
    /// Muscle, skin-side to bone-side.
    pub muscle_mm: f32,
    /// Cortical bone. **Not corpus-sourced** — see the module docs.
    pub cortex_mm: f32,
    /// The marrow band a strip draws. Marrow itself is unbounded.
    pub marrow_mm: f32,
    /// **Outer diameter of the long bone in this region, millimetres — `0` where the region has no
    /// core.** *This crate's own number*, like [`cortex_mm`](Self::cortex_mm), and for the same
    /// reason: no paper tabulating long-bone outer diameter was in the corpus this crate was written
    /// from.
    ///
    /// It is what turns the depth table from a stack of slabs into a body: a limb's bone is a **core
    /// at the deepest point of a cut**, not everything past 27.8 mm, so a cut through a thigh should
    /// show muscle right up to the bone and then bone at the middle. `annotate_cap` uses it exactly
    /// that way — see [`crate::cross_section::uv1_at_core`].
    ///
    /// **`0` on the torso and `∞` on the head**, and both are statements rather than gaps. The
    /// trunk's contents are not a cylinder of cortical bone: past the muscle wall there is a cavity
    /// this table cannot name, so a boneless row draws the muscle to the deepest point and never a
    /// cortex or a marrow. A skull is a shell with everything inside it: bone without end keeps the
    /// depth-only model, so the cortex begins at its measured depth and the interior follows.
    ///
    /// **Absent from an authored table, it reads as `∞`** — the depth-only model every table written
    /// before this field existed was written against — so a 0.1.1 RON file deserialises in 0.1.2 and
    /// bands exactly as it did. `0` would have been the wrong default: it is a statement about the
    /// trunk, not a fallback.
    #[cfg_attr(feature = "serde", serde(default = "unbounded"))]
    pub bone_mm: f32,
}

/// The pre-`bone_mm` behaviour, for a table authored before the field existed.
#[cfg(feature = "serde")]
fn unbounded() -> f32 {
    f32::INFINITY
}

impl Layers {
    /// The measured row for a region. See the module docs for every number's source.
    pub const fn for_region(region: Region) -> Self {
        match region {
            Region::Limb => Self {
                skin_mm: 1.9,
                fat_mm: 7.2,
                muscle_mm: 18.7,
                cortex_mm: 5.0,
                marrow_mm: 8.0,
                bone_mm: 27.0,
            },
            Region::Torso => Self {
                skin_mm: 2.2,
                fat_mm: 16.0,
                muscle_mm: 7.3,
                cortex_mm: 2.0,
                marrow_mm: 4.0,
                bone_mm: 0.0,
            },
            Region::Head => Self {
                skin_mm: 2.0,
                fat_mm: 2.6,
                muscle_mm: 2.2,
                cortex_mm: 6.0,
                marrow_mm: 4.0,
                bone_mm: f32::INFINITY,
            },
        }
    }

    /// Depth at which each layer begins, outside to inside, in [`Layer::ALL`] order.
    pub fn starts_mm(&self) -> [f32; 5] {
        let skin = 0.0;
        let fat = skin + self.skin_mm.max(0.0);
        let muscle = fat + self.fat_mm.max(0.0);
        let cortex = muscle + self.muscle_mm.max(0.0);
        let marrow = cortex + self.cortex_mm.max(0.0);
        [skin, fat, muscle, cortex, marrow]
    }

    /// Total depth a strip spans: every finite layer plus the drawn marrow band.
    pub fn span_mm(&self) -> f32 {
        self.starts_mm()[4] + self.marrow_mm.max(0.0)
    }

    /// The thickness of each layer, in [`Layer::ALL`] order; marrow reports its drawn band.
    pub fn thickness_mm(&self, layer: Layer) -> f32 {
        match layer {
            Layer::Skin => self.skin_mm,
            Layer::Fat => self.fat_mm,
            Layer::Muscle => self.muscle_mm,
            Layer::Cortex => self.cortex_mm,
            Layer::Marrow => self.marrow_mm,
        }
        .max(0.0)
    }

    /// **The tissue at `depth_mm` below the skin**, and how far into that layer the point is, `[0, 1)`.
    ///
    /// Negative depths are skin; depths past the cortex are marrow, with the fraction wrapping over the
    /// drawn marrow band so a texture can tile it.
    pub fn at(&self, depth_mm: f32) -> (Layer, f32) {
        let d = if depth_mm.is_finite() { depth_mm.max(0.0) } else { 0.0 };
        let s = self.starts_mm();
        for (i, layer) in Layer::ALL.iter().enumerate().take(4) {
            let (lo, hi) = (s[i], s[i + 1]);
            if d < hi {
                let t = if hi > lo { (d - lo) / (hi - lo) } else { 0.0 };
                return (*layer, t.clamp(0.0, 0.999_99));
            }
        }
        let band = self.marrow_mm.max(1.0e-3);
        let into = (d - s[4]) / band;
        (Layer::Marrow, (into - into.floor()).clamp(0.0, 0.999_99))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows are the papers' numbers, to the tenth of a millimetre they were reported at.
    #[test]
    fn the_measured_rows_are_the_papers() {
        let limb = Layers::for_region(Region::Limb);
        assert!((limb.skin_mm - 1.9).abs() < 1.0e-6);
        assert!((limb.fat_mm - 7.2).abs() < 1.0e-6);
        // Mean of triceps 22.5, forearm 15.8, quadriceps 10.3, hamstring 26.1 (Abe 2015, Table 1).
        assert!((limb.muscle_mm - (22.5 + 15.8 + 10.3 + 26.1) / 4.0).abs() < 0.05);
        let torso = Layers::for_region(Region::Torso);
        assert!((torso.muscle_mm - 7.3).abs() < 1.0e-6, "the abdomen row is Abe's 7.3 mm");
        let head = Layers::for_region(Region::Head);
        let to_bone = head.skin_mm + head.fat_mm + head.muscle_mm;
        // Mean of glabella (5.5–5.9) and nasion (7.9–8.0), Dimitrova 2018.
        assert!((to_bone - 6.8).abs() < 0.05, "head soft tissue to bone is {to_bone}");
    }

    /// Walking inward visits every layer once, in order, and never comes back out.
    #[test]
    fn depth_walks_the_layers_in_order() {
        let l = Layers::for_region(Region::Limb);
        let mut seen = 0usize;
        let mut last = Layer::Skin;
        let mut d = 0.0f32;
        while d < l.span_mm() * 2.0 {
            let (layer, frac) = l.at(d);
            assert!((0.0..1.0).contains(&frac));
            let ix = Layer::ALL.iter().position(|x| *x == layer).unwrap_or(0);
            let last_ix = Layer::ALL.iter().position(|x| *x == last).unwrap_or(0);
            assert!(ix >= last_ix, "went back out from {last:?} to {layer:?} at {d} mm");
            if ix != last_ix {
                seen += 1;
            }
            last = layer;
            d += 0.1;
        }
        assert_eq!(seen, 4, "four boundaries, five layers");
        assert_eq!(l.at(-3.0).0, Layer::Skin);
        assert_eq!(l.at(f32::NAN).0, Layer::Skin);
    }

    /// **The bone diameter is this crate's own, and it is not a thickness.** It must not enter the
    /// depth stack: a strip, a band and every digest over one are functions of `starts_mm` and
    /// `span_mm`, so a number that leaked into either would move goldens in three crates.
    #[test]
    fn the_bone_diameter_is_authored_and_outside_the_depth_stack() {
        assert_eq!(Layers::for_region(Region::Limb).bone_mm, 27.0);
        assert_eq!(Layers::for_region(Region::Torso).bone_mm, 0.0, "the trunk has no long bone");
        assert!(Layers::for_region(Region::Head).bone_mm.is_infinite(), "a skull is a shell, not a core");
        for region in Region::ALL {
            let table = Layers::for_region(region);
            let widened = Layers { bone_mm: table.bone_mm + 40.0, ..table };
            assert_eq!(widened.starts_mm(), table.starts_mm(), "{region:?}: bone_mm moved a band");
            assert_eq!(widened.span_mm(), table.span_mm(), "{region:?}: bone_mm moved the span");
            assert_eq!(widened.at(12.0), table.at(12.0), "{region:?}: bone_mm moved a lookup");
        }
    }
}
