//! **The framework surface: tone, tiers, budgets and gates.**
//!
//! Everything else in this crate answers "what does a wound do". This module answers "how much of it
//! should a player see, and when must it be refused" — which is the part that makes a gore system
//! usable by a developer rather than only by its author.
//!
//! # Reduction is substitution, never deletion
//!
//! The single most important rule here, and it is a shipped-game finding rather than a preference.
//! Vermintide 2's gore-off setting removed the effects and made the game **harder to read**: the hit
//! confirmation went with the blood. Gears of War 4 replaced blood with sparks and kept the
//! confirmation. So at every tier `spawn_wound_effects` still fires, still on the same tick, still in
//! the same direction and at the same magnitude — what changes is the *palette* and whether
//! mutilation is drawn. There is exactly one emitter path, parameterised.
//!
//! # Six toggles, not one master switch
//!
//! Because that is what shipped decompositions actually do: Killing Floor 2 exposes per-class decal
//! lifetimes, Vermintide 2 four separate switches, Assassin's Creed Valhalla per-category toggles. A
//! developer building a gore slider must never have to fork this crate, and [`CarnageSettings`] being
//! `deny_unknown_fields` means they cannot patch one in either.
//!
//! # Two of these gates are accessibility, and they are not optional
//!
//! A gore framework is, mechanically, a **saturated-red flash generator**. WCAG 2.1 SC 2.3.1's
//! technique G19 is explicit: three flashes or fewer in any one-second period is conformance, and
//! 3 Hz is the safe harbour. That limiter belongs *here*, in the thing that generates the flashes,
//! rather than in each integrator that might remember to add one — so [`FlashGate`] refuses the
//! fourth flash in any one-second window and there is no way to configure it away above that rate.
//!
//! [`occludes_aim`] is the second: decals and screen effects that would land inside a cone around the
//! aim point are refused, because gaze concentrates at screen centre while aiming and blood over the
//! reticle is blood over the information the player is using.

use bevy::math::Vec2;
use bevy::prelude::*;

use crate::CarnageSettings;

/// **How much gore is drawn.** Four stops, and they map onto rating descriptors rather than taste.
///
/// ESRB's content descriptors are *Animated Blood*, *Blood*, and *Blood and Gore* — the last defined
/// as depicting "mutilation of body parts". PEGI's gross-violence criterion turns on emphasis and
/// persistence rather than on presence. So one enum gives a **rating lever**, an **accessibility
/// lever** and a **tone lever** at once, which is why it is one enum and not three.
///
/// **Append-only**, like every other `#[repr(u32)]` in this family: the discriminant is authored in
/// config and stored in a save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum GoreTier {
    /// No blood at all: the hit is confirmed by the substitute palette instead. ESRB's *Animated
    /// Blood* territory, and the tier a photosensitivity or squeamishness setting selects.
    Stylised = 0,
    /// Blood, no dismemberment. ESRB *Blood*.
    Blood = 1,
    /// Blood and mutilation of body parts. ESRB *Blood and Gore*.
    BloodAndGore = 2,
    /// Persistent, emphasised gore. PEGI's gross-violence criterion.
    GrossViolence = 3,
}

/// **The framework's one policy resource.** What is drawn, how much of it, and what is refused.
///
/// Insert it before [`CarnagePlugin`](crate::CarnagePlugin) to own the values; the plugin
/// `init_resource`s it, which does nothing when it is already present.
#[derive(Resource, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct GorePolicy {
    /// The tier. Setting it is *not* enough on its own — call [`GorePolicy::for_tier`] to get the
    /// toggle set that goes with it, or set the toggles yourself and leave this as documentation of
    /// intent.
    pub tier: GoreTier,
    /// Blood decals on the world.
    pub blood_decals: bool,
    /// Limbs and chunks coming off.
    pub dismemberment: bool,
    /// Guts, and anything else that spills.
    pub viscera: bool,
    /// Blood on the camera plane. **The one most likely to be turned off for readability**, which is
    /// why it is separate from `blood_decals` rather than folded into it.
    pub screen_blood: bool,
    /// Bodies that fall rather than vanish.
    pub ragdolls: bool,
    /// Multiplier on every per-class lifetime. `0.0` despawns immediately; `1.0` is as authored.
    /// PEGI's criterion is about **persistence**, so this is the dial that moves a rating.
    pub persistence_scale: f32,
    /// **The named 0–1 scalar an accessibility slider binds to.** `feel::shake_offset` multiplies by
    /// it, and `0.0` is fully supported — a shake nobody can turn off is a motion-sickness bug.
    pub shake_scale: f32,
    /// Overall intensity in `[0, 1]`.
    ///
    /// **Default `0.6`, not `1.0`, and the shape is measured.** Barlett et al.
    /// (`doi:10.1016/j.jesp.2007.10.003`) find low blood produces no arousal effect while medium and
    /// maximum both do — so the low end buys nothing. Kao (`doi:10.1016/j.entcom.2020.100359`,
    /// N = 3018) finds juiciness is an **inverted U**: extreme juice is worse than none. So there is
    /// a floor below which the effect is absent and a ceiling above which it reverses, and shipping
    /// `1.0` would put a game on the wrong side of the second.
    pub intensity: f32,
    /// `0` = cartoon, `1` = photo-referenced. Tone rather than quantity: the same amount of blood
    /// reads very differently stylised.
    pub reference_class: f32,
    /// `0` = gravity flow, `1` = ballistic arc. The other tone axis, and the one that separates a
    /// horror game from an action game at identical volumes.
    pub ejection_profile: f32,
    /// How wet blood reads, `[0, 1]`, scaling the specular channel of
    /// [`bloodstain::dry::Appearance`]. **Wetness is the strongest disgust cue and it is not a
    /// colour** (Oum et al., `doi:10.1080/02699931.2010.496997`), so it gets a dial of its own rather
    /// than riding on `intensity`.
    pub wetness: f32,
    /// Half-angle of the cone around the aim point that refuses decals and screen effects, degrees.
    ///
    /// **`10.0`, and the number has a source.** Ten degrees is the visual-field unit WCAG itself uses
    /// for its flash-area threshold, and gaze concentrates at screen centre while aiming. **Do not
    /// write 30°** — that figure is folklore, and at 30° a third of the screen refuses blood.
    pub aim_exclusion_deg: f32,
    /// Ceiling on live decals. Overlapping decals converge on mud, which is both a readability
    /// failure and a variety failure.
    pub max_decals: u32,
    /// Ceiling on flashes per second. **Clamped to 3 by [`FlashGate::admit`] whatever this says** —
    /// WCAG's safe harbour is not a preference.
    pub max_flashes_per_second: u32,
}

/// The shipped [`GorePolicy`] values, one function per dial — the single-source pattern
/// [`CarnageSettings`] uses, for the same reason.
pub mod shipped {
    use super::GoreTier;

    pub(super) fn tier() -> GoreTier {
        GoreTier::BloodAndGore
    }
    pub(super) fn on() -> bool {
        true
    }
    pub(super) fn persistence_scale() -> f32 {
        1.0
    }
    pub(super) fn shake_scale() -> f32 {
        1.0
    }
    // The inverted-U's middle, not its top. See `GorePolicy::intensity`.
    pub(super) fn intensity() -> f32 {
        0.6
    }
    pub(super) fn reference_class() -> f32 {
        1.0
    }
    pub(super) fn ejection_profile() -> f32 {
        0.5
    }
    pub(super) fn wetness() -> f32 {
        1.0
    }
    // WCAG's own visual-field unit. Not 30.
    pub(super) fn aim_exclusion_deg() -> f32 {
        10.0
    }
    pub(super) fn max_decals() -> u32 {
        256
    }
    // WCAG 2.1 SC 2.3.1 technique G19's safe harbour.
    pub(super) fn max_flashes_per_second() -> u32 {
        3
    }
}

impl Default for GorePolicy {
    fn default() -> Self {
        GorePolicy {
            tier: shipped::tier(),
            blood_decals: shipped::on(),
            dismemberment: shipped::on(),
            viscera: shipped::on(),
            screen_blood: shipped::on(),
            ragdolls: shipped::on(),
            persistence_scale: shipped::persistence_scale(),
            shake_scale: shipped::shake_scale(),
            intensity: shipped::intensity(),
            reference_class: shipped::reference_class(),
            ejection_profile: shipped::ejection_profile(),
            wetness: shipped::wetness(),
            aim_exclusion_deg: shipped::aim_exclusion_deg(),
            max_decals: shipped::max_decals(),
            max_flashes_per_second: shipped::max_flashes_per_second(),
        }
    }
}

impl GorePolicy {
    /// **The toggle set that goes with a tier.**
    ///
    /// A convenience over setting fifteen fields, and the place the substitution rule is written down:
    /// every tier keeps `blood_decals`' *emitter* running, and what falls away as the tier drops is
    /// mutilation, then persistence, then the blood palette itself — never the hit confirmation.
    pub fn for_tier(tier: GoreTier) -> Self {
        let base = GorePolicy { tier, ..Default::default() };
        match tier {
            GoreTier::Stylised => GorePolicy {
                // **The emitter still fires.** `blood_decals` stays true and the caller swaps the
                // palette for `CarnageSettings::substitute_srgb` — that is the substitution, and
                // turning the channel off here is exactly the mistake this whole module records.
                dismemberment: false,
                viscera: false,
                screen_blood: false,
                persistence_scale: 0.25,
                intensity: 0.4,
                reference_class: 0.0,
                wetness: 0.0,
                ..base
            },
            GoreTier::Blood => {
                GorePolicy { dismemberment: false, viscera: false, persistence_scale: 0.6, ..base }
            }
            GoreTier::BloodAndGore => base,
            GoreTier::GrossViolence => {
                GorePolicy { persistence_scale: 1.5, intensity: 0.85, ..base }
            }
        }
    }

    /// Whether this policy draws blood as blood. `false` at [`GoreTier::Stylised`], where the same
    /// emitter runs with the substitute palette.
    pub fn draws_blood(&self) -> bool {
        self.tier > GoreTier::Stylised
    }

    /// Reject a policy that cannot mean what it says.
    pub fn validate(&self) -> Result<(), String> {
        for (name, v) in [
            ("persistence_scale", self.persistence_scale),
            ("shake_scale", self.shake_scale),
            ("intensity", self.intensity),
            ("reference_class", self.reference_class),
            ("ejection_profile", self.ejection_profile),
            ("wetness", self.wetness),
        ] {
            if !v.is_finite() || v < 0.0 {
                return Err(format!(
                    "carnage: {name} is {v} — every policy scalar must be finite and non-negative."
                ));
            }
        }
        if !(0.0..=90.0).contains(&self.aim_exclusion_deg) {
            return Err(format!(
                "carnage: aim_exclusion_deg is {} — it is a half-angle about the aim point, so it \
                 must be in [0, 90]. Ten is the documented value; thirty is folklore.",
                self.aim_exclusion_deg
            ));
        }
        if self.max_decals == 0 {
            return Err("carnage: max_decals is 0 — that turns off world blood through a ceiling \
                        rather than through `blood_decals`, which is the honest switch."
                .to_string());
        }
        Ok(())
    }
}

/// **WCAG's safe harbour, as a hard ceiling.** Three flashes in any one-second period.
///
/// WCAG 2.1 SC 2.3.1, technique G19: three flashes or fewer per second is conformance, and 3 Hz is
/// the safe harbour rate. [`GorePolicy::max_flashes_per_second`] can lower it and **cannot raise it**
/// — the clamp is in [`admit`](Self::admit), not in the settings validator, because a limiter a
/// caller can configure away is not a limiter.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct FlashGate {
    /// The last flashes admitted, oldest first. Fixed capacity: the ceiling is 3, so a ring of 4 can
    /// always answer "how many in the last second" without allocating.
    ticks: [Option<u32>; FLASH_RING],
    /// Where the next admission is written. Wraps.
    cursor: usize,
}

/// Slots in [`FlashGate`]'s ring. One more than the ceiling, so the window can be measured exactly.
const FLASH_RING: usize = 4;

/// The absolute ceiling on flashes per one-second window, whatever a policy says.
pub const WCAG_FLASHES_PER_SECOND: u32 = 3;

impl FlashGate {
    /// **May a flash happen on this tick?** `true` admits it and records it; `false` refuses.
    ///
    /// Refuses the fourth flash inside any one-second window, and admits again once the window has
    /// moved past the oldest of the three. Integer ticks throughout, so it cannot drift and it
    /// replays exactly.
    pub fn admit(&mut self, tick: u32, hz: u32, policy: &GorePolicy) -> bool {
        let hz = hz.max(1);
        let allowed = policy.max_flashes_per_second.min(WCAG_FLASHES_PER_SECOND).max(1) as usize;
        // How many admitted flashes fall inside the last second.
        let recent = self
            .ticks
            .iter()
            .filter(|slot| slot.is_some_and(|t| tick.wrapping_sub(t) < hz))
            .count();
        if recent >= allowed {
            return false;
        }
        self.ticks[self.cursor % FLASH_RING] = Some(tick);
        self.cursor = self.cursor.wrapping_add(1);
        true
    }

    /// How many flashes the last second holds. For a debug readout — a demo that draws the meter is
    /// how a developer sees the gate working rather than trusting it.
    pub fn recent(&self, tick: u32, hz: u32) -> u32 {
        let hz = hz.max(1);
        self.ticks.iter().filter(|s| s.is_some_and(|t| tick.wrapping_sub(t) < hz)).count() as u32
    }
}

/// **Would this land on the reticle?** `true` means refuse it.
///
/// `ndc` is the effect's centre in normalised device coordinates (`[-1, 1]` on both axes, origin at
/// screen centre) and `radius_ndc` is how far it reaches. The exclusion is a cone about the aim
/// point, converted from [`GorePolicy::aim_exclusion_deg`] against a nominal 90° horizontal field —
/// so ten degrees is a ninth of the half-width, which is a reticle-sized hole rather than a third of
/// the screen.
///
/// **Blood over the reticle is blood over the information the player is using.** Gaze concentrates at
/// screen centre while aiming, so this is where an effect is both most visible and least welcome.
pub fn occludes_aim(ndc: Vec2, radius_ndc: f32, policy: &GorePolicy) -> bool {
    if !ndc.is_finite() || !radius_ndc.is_finite() {
        // A non-finite position is not a place, and refusing to draw there is the safe answer.
        return true;
    }
    // Nominal half-field. A caller with a different FOV scales `aim_exclusion_deg` accordingly rather
    // than this crate guessing at a camera it does not own.
    const HALF_FIELD_DEG: f32 = 45.0;
    let exclusion = (policy.aim_exclusion_deg / HALF_FIELD_DEG).clamp(0.0, 1.0);
    ndc.length() - radius_ndc.max(0.0) < exclusion
}

/// **The live-decal ceiling, with canonical eviction.**
///
/// Overlapping decals converge on mud: past a few dozen in one place, a floor stops reading as
/// individual stains and starts reading as a brown texture — which loses both the readability and the
/// variety the derived stain morphology exists to buy.
///
/// Oldest-first eviction, by the tick a decal was admitted and then by its own id. **A total order, so
/// which decal is evicted is a function of the record rather than of ECS iteration order** — the same
/// rule every other ordering decision in this crate keeps.
#[derive(Resource, Clone, Debug, Default)]
pub struct DecalBudget {
    live: Vec<(u32, u64)>,
}

impl DecalBudget {
    /// How many decals are live.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Is the budget empty?
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// **Admit one decal, and say what to despawn.**
    ///
    /// Returns `None` when the decal is refused outright (the policy has world blood off), otherwise
    /// the ids the caller must despawn to stay inside the ceiling — usually empty, and at most one
    /// per admission once the budget is full.
    pub fn admit(&mut self, tick: u32, id: u64, policy: &GorePolicy) -> Option<Vec<u64>> {
        if !policy.blood_decals {
            return None;
        }
        self.live.push((tick, id));
        let cap = policy.max_decals.max(1) as usize;
        let mut evicted = Vec::new();
        while self.live.len() > cap {
            // SORT-OK: `(tick, id)` is unique per admission — one id is admitted once — so the key is
            // total and the minimum is a function of the record alone.
            let Some(oldest) = self
                .live
                .iter()
                .enumerate()
                .min_by_key(|(_, (t, i))| (*t, *i))
                .map(|(index, _)| index)
            else {
                break;
            };
            evicted.push(self.live.remove(oldest).1);
        }
        Some(evicted)
    }

    /// Forget a decal the caller despawned itself, so the budget does not hold a ghost.
    pub fn release(&mut self, id: u64) {
        self.live.retain(|(_, i)| *i != id);
    }
}

/// **One stop per frame, not a sum.**
///
/// Hit stop is a *pause*, and pauses do not add: five wounds in one tick asking for three ticks each
/// is fifteen ticks of freeze, which reads as a hitch rather than as five impacts. The game-feel
/// survey's own warning is that hit stop spent everywhere stops reading as impact anywhere
/// (`doi:10.1109/tg.2021.3072241`), so this takes the **maximum** and caps the total per second.
///
/// `pending` is drained: the caller collects every wound's `hitstop_ticks` into it during the tick and
/// calls this once. Draining is what makes double-counting impossible.
pub fn coalesce_hitstop(
    pending: &mut Vec<u32>,
    hz: u32,
    s: &CarnageSettings,
) -> u32 {
    let one = pending.iter().copied().max().unwrap_or(0);
    pending.clear();
    let hz = hz.max(1);
    // The per-second ceiling, as whole ticks.
    let budget = (s.hitstop_budget_per_second.clamp(0.0, 1.0) * hz as f32).round() as u32;
    one.min(budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The fourth flash in a second is refused, and admitted once the window has passed.** WCAG
    /// 2.1 SC 2.3.1's safe harbour, as a test rather than as a comment.
    #[test]
    fn the_flash_gate_refuses_the_fourth_flash_in_a_second() {
        let policy = GorePolicy::default();
        let mut gate = FlashGate::default();
        assert!(gate.admit(0, 60, &policy), "the first flash is admitted");
        assert!(gate.admit(10, 60, &policy), "the second");
        assert!(gate.admit(20, 60, &policy), "the third");
        assert!(!gate.admit(30, 60, &policy), "the fourth inside one second must be refused");
        assert_eq!(gate.recent(30, 60), 3, "and the meter must say why");

        // Once the window has moved past the first three, it admits again.
        assert!(gate.admit(61, 60, &policy), "past the window the gate opens again");
    }

    /// **A caller cannot configure the ceiling upward.** A limiter that can be turned off is not a
    /// limiter, so the clamp lives in `admit` rather than in a validator.
    #[test]
    fn a_policy_cannot_raise_the_flash_ceiling() {
        let greedy = GorePolicy { max_flashes_per_second: 60, ..Default::default() };
        let mut gate = FlashGate::default();
        let admitted = (0..10u32).filter(|t| gate.admit(*t, 60, &greedy)).count();
        assert_eq!(
            admitted, WCAG_FLASHES_PER_SECOND as usize,
            "the WCAG ceiling must hold whatever the policy asks for"
        );
    }

    /// A lower ceiling *is* honoured — the clamp is one-directional.
    #[test]
    fn a_policy_can_lower_the_flash_ceiling() {
        let cautious = GorePolicy { max_flashes_per_second: 1, ..Default::default() };
        let mut gate = FlashGate::default();
        assert!(gate.admit(0, 60, &cautious));
        assert!(!gate.admit(1, 60, &cautious), "one per second means one");
    }

    /// The aim cone is reticle-sized, not a third of the screen — the number `aim_exclusion_deg`'s
    /// doc comment insists on, asserted.
    #[test]
    fn the_aim_cone_excludes_the_reticle_and_not_the_screen() {
        let p = GorePolicy::default();
        assert!(occludes_aim(Vec2::ZERO, 0.0, &p), "dead centre must be refused");
        assert!(
            !occludes_aim(Vec2::new(0.5, 0.0), 0.0, &p),
            "half way to the edge must be allowed, or blood has nowhere to land"
        );
        // Ten degrees of a 45° half-field is 0.222 in NDC.
        assert!(occludes_aim(Vec2::new(0.15, 0.0), 0.0, &p), "just inside the cone is refused");
        assert!(!occludes_aim(Vec2::new(0.30, 0.0), 0.0, &p), "just outside it is admitted");
        // A large effect near the cone still reaches into it.
        assert!(
            occludes_aim(Vec2::new(0.4, 0.0), 0.3, &p),
            "an effect that REACHES the reticle must be refused, not only one centred on it"
        );
        assert!(occludes_aim(Vec2::new(f32::NAN, 0.0), 0.0, &p), "a non-place must be refused");

        let off = GorePolicy { aim_exclusion_deg: 0.0, ..Default::default() };
        assert!(!occludes_aim(Vec2::new(0.001, 0.0), 0.0, &off), "zero degrees excludes nothing");
    }

    /// The decal budget holds its ceiling and evicts the **oldest**, deterministically.
    #[test]
    fn the_decal_budget_evicts_oldest_first_and_holds_its_cap() {
        let p = GorePolicy { max_decals: 4, ..Default::default() };
        let mut budget = DecalBudget::default();
        for id in 0..4u64 {
            let evicted = budget.admit(id as u32, id, &p).expect("world blood is on");
            assert!(evicted.is_empty(), "nothing is evicted below the cap");
        }
        let evicted = budget.admit(100, 99, &p).expect("admitted");
        assert_eq!(evicted, vec![0], "the oldest decal must go first");
        assert_eq!(budget.len(), 4, "and the cap must hold");

        // Twice over the same input gives the same eviction — no iteration order anywhere.
        let mut a = DecalBudget::default();
        let mut b = DecalBudget::default();
        let run = |d: &mut DecalBudget| -> Vec<u64> {
            (0..40u64).flat_map(|id| d.admit((id / 2) as u32, id, &p).unwrap_or_default()).collect()
        };
        assert_eq!(run(&mut a), run(&mut b), "eviction must be a function of the record");

        let off = GorePolicy { blood_decals: false, ..Default::default() };
        assert!(
            DecalBudget::default().admit(0, 1, &off).is_none(),
            "with world blood off the decal is refused outright rather than admitted and evicted"
        );
    }

    /// **Simultaneous wounds produce one stop, not a sum** — and the total is capped per second.
    #[test]
    fn hitstop_coalesces_to_one_stop_and_stays_inside_its_budget() {
        let s = CarnageSettings::default();
        let mut pending = vec![3u32, 3, 3, 3, 3];
        let one = coalesce_hitstop(&mut pending, 60, &s);
        assert_eq!(one, 3, "five wounds asking for three ticks each is three ticks, not fifteen");
        assert!(pending.is_empty(), "the queue must be drained, or the next tick double-counts");

        let mut greedy = vec![600u32];
        let capped = coalesce_hitstop(&mut greedy, 60, &s);
        let budget = (s.hitstop_budget_per_second * 60.0).round() as u32;
        assert_eq!(capped, budget, "a single absurd request must be capped to the per-second budget");

        let mut none: Vec<u32> = Vec::new();
        assert_eq!(coalesce_hitstop(&mut none, 60, &s), 0, "no wounds is no freeze");
    }

    /// **Reduction is substitution.** At every tier the emitter still runs; what changes is what it
    /// draws. A tier that switched `blood_decals` off would be the Vermintide 2 mistake, in code.
    #[test]
    fn every_tier_keeps_the_hit_confirmation() {
        for tier in
            [GoreTier::Stylised, GoreTier::Blood, GoreTier::BloodAndGore, GoreTier::GrossViolence]
        {
            let p = GorePolicy::for_tier(tier);
            assert!(
                p.blood_decals,
                "{tier:?} switched the decal emitter off — reduction is substitution, never deletion"
            );
            assert!(p.validate().is_ok(), "{tier:?} must be a valid policy");
            assert_eq!(p.tier, tier, "the tier must record itself");
        }
        // And the tiers actually differ in what they draw.
        assert!(!GorePolicy::for_tier(GoreTier::Stylised).draws_blood());
        assert!(!GorePolicy::for_tier(GoreTier::Blood).dismemberment);
        assert!(GorePolicy::for_tier(GoreTier::BloodAndGore).dismemberment);
        assert!(
            GorePolicy::for_tier(GoreTier::GrossViolence).persistence_scale
                > GorePolicy::for_tier(GoreTier::Blood).persistence_scale,
            "PEGI's criterion is persistence, so the top tier must persist longer"
        );
    }

    /// The shipped intensity is the inverted U's middle, not its top — the measured shape, pinned so
    /// a later "turn it up" edit has to argue with the citation.
    #[test]
    fn the_shipped_intensity_is_not_maximal() {
        let p = GorePolicy::default();
        assert!(
            p.intensity > 0.3 && p.intensity < 0.8,
            "intensity {} is outside the band the juiciness literature supports",
            p.intensity
        );
        assert_eq!(p.aim_exclusion_deg, 10.0, "ten degrees is the documented value");
        assert_eq!(p.max_flashes_per_second, WCAG_FLASHES_PER_SECOND);
        assert!(p.validate().is_ok());
    }

    /// Each refusal fires. A validator nobody has seen fail is a comment.
    #[test]
    fn the_policy_door_refuses_what_cannot_mean_anything() {
        let bad = |f: fn(&mut GorePolicy)| {
            let mut p = GorePolicy::default();
            f(&mut p);
            p.validate().expect_err("this policy must be refused")
        };
        assert!(bad(|p| p.intensity = -1.0).contains("intensity"));
        assert!(bad(|p| p.shake_scale = f32::NAN).contains("shake_scale"));
        // 120° is not a half-angle about anything: refused. Note that 30° is *valid* and merely
        // folklore — the validator's job is impossibility, not taste, and the taste argument lives
        // in the field's own doc comment where a developer reading the dial will see it.
        assert!(bad(|p| p.aim_exclusion_deg = 120.0).contains("aim_exclusion_deg"));
        assert!(bad(|p| p.max_decals = 0).contains("max_decals"));
    }
}
