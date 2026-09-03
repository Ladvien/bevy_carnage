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

/// Everything `bevy_carnage` is allowed to depend on. An engine, a serializer, a mesh validator, and
/// a particle system.
///
/// **`isomesh` was added deliberately, and this comment is the review the assertion below asks for.**
/// It buys the one thing this crate could never do for itself — say whether a fragment is closed,
/// manifold and consistently wound — and it is admitted on terms that keep the boundary meaningful: it
/// is `no_std`, it has exactly one dependency of its own (`libm`), and **its public API is `[f32; 3]`
/// rather than any math library's vector type**, so it cannot drag a second `glam` into a consumer's
/// tree. A crate that pinned `glam` would have been refused on that ground alone, however good it was.
///
/// Note what it is *not*: a geometry backend this crate cannot work without. The fracture is still
/// `soup.rs` and owes `isomesh` nothing — it is a second opinion about the output, not a source of it.
///
/// **`bevy_hanabi` was added in AG-030, and it is admitted on two terms rather than one.**
///
/// First, **it is optional and behind the `vfx` feature**, so the deterministic half of this crate
/// never sees it. `cargo build --release --no-default-features --features serde` resolves no
/// `bevy_hanabi` and no `bevy_render` at all — that is the property the CI's plain `cargo build` step
/// exists to defend, and a mandatory particle dependency would have destroyed it.
///
/// Second — and this is the sharper reason — **it renders and it cannot report.** Hanabi 0.19 has no
/// public GPU→CPU readback path whatsoever: its only `map_async` is behind
/// `#[cfg(all(test, feature = "gpu_tests"))]`, and its `copy_buffer_to_buffer` calls are internal
/// buffer reallocation. So a particle's position is *physically unable* to reach a golden, a hash, or
/// a simulation, and the crate's "cosmetic output never re-enters the deterministic half" rule ends up
/// enforced by the library rather than by anyone remembering it. A particle system that offered
/// readback would have needed a different answer here, however good its visuals were.
///
/// Note what it is *not*, again: a source of truth about anything. Where blood *lands* is
/// `bloodstain`, on the CPU, from `hash_f32`, and it is available with `vfx` off entirely.
///
/// **`bloodstain` was added on 2026-09-02, and it is the easiest admission of the four** — because it
/// is not an addition at all. It is *this crate's own blood model*, carved out: the Comiskey spatter,
/// the pools, the bleed schedule, `hash_f32` and `WoundKind` all used to live in `src/`. Nothing new
/// entered the tree; a boundary was drawn inside it.
///
/// It passes `isomesh`'s terms and for the same reasons: `no_std`, two dependencies of its own
/// (`libm` and an optional `serde`), and **no math library in its public API** — every signature is
/// `[f32; 3]`. That last property is the load-bearing one, and it is why the conversion has exactly
/// one home, `src/v3.rs`.
const ALLOWED_DEPS: &[&str] = &["bevy", "serde", "isomesh", "bevy_hanabi", "bloodstain"];

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
