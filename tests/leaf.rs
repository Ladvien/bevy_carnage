//! **The crate boundary, enforced.**
//!
//! `bevy_carnage` breaks meshes and bleeds them. It takes `bevy` (a trimmed feature set), optionally
//! `serde`, and optionally `bevy_hanabi` behind the `vfx` feature — and it must never learn anything
//! about a particular game.
//!
//! The source scan below forbids more than crate names. `GoreSettings`, `FigurineSource`, `GunModel`,
//! `SquadMember` and `Dungeon` are there because each is a specific temptation this crate already
//! faced: it was extracted from a game where the bake read the gore settings block directly, queried a
//! component called `FigurineSource`, and pruned a subtree marked `GunModel`. Those names came out in
//! the extraction and became `FractureSettings`, `FractureSubject` and `DetachedPart`. Putting one
//! back would compile, and would quietly make this crate useless to anyone whose game has no guns.
//!
//! `avian` is forbidden for a sharper reason than tidiness. This crate hands out a centre and a
//! half-extent per fragment and stops there; the moment it spawns a rigid body it has chosen a physics
//! engine on the caller's behalf, and a fracture library that only works with one solver is not a
//! fracture library.
//!
//! `MainCamera` is forbidden for a reason `feel.rs` states in as many words: this crate returns a
//! shake offset and a trauma number, and the caller moves its own camera. A crate that reached for a
//! camera would be a second writer of a transform the consumer already owns.
//!
//! `bevy_app` is not forbidden here (the trimmed `bevy` umbrella pulls it, and this crate does register
//! two plugins), but neither configures a run condition: the caller keeps its schedule.

use std::path::{Path, PathBuf};

/// **The gore family came home on 2026-09-04.** `bloodstain`, `bevy_wetmap`, `bevy_viscera`,
/// `bevy_cross_section`, `bevy_flaymap`, `bevy_laceration` and `bevy_fracture_modes` were seven
/// published crates this one composed; they are now the modules of the same names under `src/`
/// (`bevy_carnage::bloodstain`, `::wetmap`, …), because this is the one crate a game is meant to
/// depend on and seven leaves were seven version pins to keep aligned. Their own ratchets — engine-free
/// `bloodstain`, `[f32; 3]` in its public API, the conversion with one home in `src/v3.rs` — are
/// now module conventions rather than crate boundaries, and `bloodstain_is_still_engine_free` below
/// holds the one that matters most.
///
/// **`libm`** is the one math library, unconditional: `bloodstain` and the flesh tables both bake
/// with it so a golden means the same bytes on every machine.
const ALLOWED_DEPS: &[&str] = &[
    "bevy",
    "serde",
    "isomesh",
    "bevy_hanabi",
    "libm",
];

/// Names that would mean the boundary has been crossed. The first three are crates; the rest are
/// identifiers from the game this was extracted from, checked as substrings because re-introducing one
/// is a mistake that compiles.
const FORBIDDEN_DEP_MARKERS: &[&str] = &[
    "avian",
    "emerge",
    "foundation_vs_slop",
    "GoreSettings",
    "FigurineSource",
    "GunModel",
    "SquadMember",
    "MainCamera",
    "Dungeon",
];

fn crate_root() -> PathBuf {
    // Cargo runs a test binary with the cwd set to its own package root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Strip `//` line comments and `/* */` blocks so a doc comment *about* the game this came out of — of
/// which this crate has several, deliberately, because they are the determinism record — is not
/// mistaken for a dependency on it.
///
/// Deliberately simple: it does not track string literals, so a `"//"` inside a string would end the
/// line early. That is acceptable here because the only thing this feeds is a substring search for
/// crate names, and an over-eager strip can only ever cause a false *pass* on a line that already
/// contains no dependency.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_block = false;
    for line in src.lines() {
        let mut rest = line;
        loop {
            if in_block {
                match rest.find("*/") {
                    Some(end) => {
                        in_block = false;
                        rest = &rest[end + 2..];
                    }
                    None => break,
                }
            } else {
                let line_at = rest.find("//");
                let block_at = rest.find("/*");
                match (line_at, block_at) {
                    (Some(l), b) if b.is_none_or(|b| l < b) => {
                        out.push_str(&rest[..l]);
                        break;
                    }
                    (_, Some(b)) => {
                        out.push_str(&rest[..b]);
                        in_block = true;
                        rest = &rest[b + 2..];
                    }
                    _ => {
                        out.push_str(rest);
                        break;
                    }
                }
            }
        }
        out.push('\n');
    }
    out
}

#[test]
fn no_source_file_reaches_for_a_game() {
    let src = crate_root().join("src");
    let mut files = Vec::new();
    rust_sources(&src, &mut files);
    assert!(
        files.len() >= 4,
        "expected to scan the whole crate, found only {} file(s) — has the layout moved?",
        files.len()
    );

    let mut offenders = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let code = strip_comments(&text);
        for (n, line) in code.lines().enumerate() {
            for marker in FORBIDDEN_DEP_MARKERS {
                if line.contains(marker) {
                    offenders.push(format!(
                        "{}:{} — {}",
                        path.strip_prefix(crate_root()).unwrap_or(path).display(),
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "bevy_carnage must stay game-free, but {} line(s) reference a game or a physics engine.\n  {}\n\n\
         This crate is what lets a fracture be reused by a project that has no guns, no squad, and a \
         different solver. If a concept genuinely needs to cross the boundary, the answer is a neutral \
         name here and the game's vocabulary in the caller's facade — not a dependency.",
        offenders.len(),
        offenders.join("\n  ")
    );
}

#[test]
fn the_dependency_list_stays_closed() {
    let manifest = std::fs::read_to_string(crate_root().join("Cargo.toml"))
        .expect("bevy_carnage must have a Cargo.toml");

    // Only the `[dependencies]` table — a dev-dependency on something heavier would be a different
    // (and much less alarming) conversation.
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("bevy_carnage must declare a [dependencies] table");
    let deps = deps.split("\n[").next().unwrap_or(deps);

    for line in deps.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `.` is a separator too, so an inherited dependency written as the dotted key
        // `serde.workspace = true` reads as `serde` rather than as a crate called
        // `serde.workspace`. A crate name cannot contain a dot, so nothing real is truncated —
        // and the check keeps biting on the part that matters, the name before it.
        let name = line.split(['=', ' ', '.']).next().unwrap_or("");
        // A dependency line is `name = ...` or `name.workspace = true`. Anything else inside this
        // table is a CONTINUATION of a multi-line value — a feature array's entries, a closing
        // bracket — and is not a crate name. Reading one as a crate name produced a genuinely
        // baffling failure ("declares `\"bevy_asset\",`"), so continuations are skipped by shape.
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        assert!(
            ALLOWED_DEPS.contains(&name),
            "bevy_carnage declares `{name}`, which is not in its allowed set {ALLOWED_DEPS:?}.\n\
             Widening this is a design decision, not a convenience. If it is genuinely warranted, add \
             it to ALLOWED_DEPS in this test in the same commit, so the change is visible in review."
        );
        for marker in FORBIDDEN_DEP_MARKERS {
            assert!(
                !name.contains(marker),
                "bevy_carnage declares `{name}` — the crate exists precisely so it does not depend on \
                 a game or a solver."
            );
        }
    }
}

/// **`bloodstain` stays engine-free, as a module.** It was a `no_std` crate with `libm` as its only
/// dependency and `[f32; 3]` in every public signature, and that is the property that let anything
/// depend on it — including the two `glam` versions this workspace resolves, which a leaf naming
/// either would have collided with. Folding it into this crate must not let `bevy` seep in: the
/// module never names `bevy`, `glam` or `std`, and the conversion to `Vec3` keeps its one home in
/// `src/v3.rs`.
#[test]
fn bloodstain_is_still_engine_free() {
    let mut files = Vec::new();
    rust_sources(&crate_root().join("src/bloodstain"), &mut files);
    assert!(!files.is_empty(), "src/bloodstain must exist");
    for path in files {
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        // The unit tests at the tail of each file are `std` by design (`println!`, `format!`); the
        // module convention is that `#[cfg(test)]` is the last item, so the scan stops there.
        let src = src.split("#[cfg(test)]").next().unwrap_or("");
        let stripped = strip_comments(src);
        for marker in ["bevy::", "bevy_", "glam::", "Vec3", "Vec2"] {
            assert!(
                !stripped.contains(marker),
                "{} names `{marker}` — the blood model is engine-free and stays so",
                path.display()
            );
        }
        for (i, line) in stripped.lines().enumerate() {
            assert!(
                !line.contains("std::"),
                "{}:{}: `{}` — the blood model reaches for std",
                path.display(),
                i + 1,
                line.trim()
            );
        }
    }
}
