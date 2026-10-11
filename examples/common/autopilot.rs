//! **A windowed example that drives itself**: it presses its own keys, saves its own screenshots, and
//! quits on a script.
//!
//! The reason is a platform fact, not a convenience. On macOS an OS-level screenshot and an
//! OS-level keystroke both need permissions a CI-style process does not have — without Screen
//! Recording, `CGWindowListCopyWindowInfo` returns no window names and `screencapture` has nothing
//! to aim at; without Accessibility, `osascript` keystrokes go nowhere. So the example takes the
//! screenshot from *inside* its own render graph and injects the input into *its own* message queue.
//! Neither needs the OS's blessing.
//!
//! **It is inert unless asked.** [`Autopilot`] reads `CARNAGE_AUTOPILOT` once, in `build`. With the
//! variable absent it adds nothing to the `App` — no resource, no system — so an example that
//! registers it behaves exactly as it did before. On wasm32 `std::env::var` is always `Err`, so the
//! plugin is just as inert in a browser, and the call site needs no `cfg`.
//!
//! # The script
//!
//! `CARNAGE_AUTOPILOT` is whitespace-separated `<frame>:<action>` tokens. `frame` is a `u32` counted
//! in `Update` runs, the first being frame 0. The actions:
//!
//! | action | does |
//! |---|---|
//! | `shot` | saves the primary window to `$CARNAGE_AUTOPILOT_OUT/f{frame:05}.png` — the variable is required whenever a `shot` appears, and the directory is created if absent |
//! | `key=<Name>` | presses `<Name>` at `frame`, releases it at `frame + 1` |
//! | `hold=<Name>*<n>` | presses `<Name>` at `frame`, releases it at `frame + n` (`n ≥ 1`) |
//! | `exit` | writes `AppExit::Success` |
//!
//! `<Name>` is a `KeyCode` variant name — `Digit1`, `KeyG`, `Space`, `ArrowUp`, `BracketRight`,
//! `Comma`, `Period`, `Escape` and the rest of the short list in [`key_code`]. An unknown name is a
//! malformed script, not a key that quietly does nothing.
//!
//! ```sh
//! CARNAGE_AUTOPILOT="30:shot 40:key=Digit1 160:shot 200:exit" CARNAGE_AUTOPILOT_OUT=/tmp/ap \
//!     cargo run --release -p bevy_carnage --example sever
//! ```
//!
//! A malformed script is `error!`ed with the reason and the plugin stays inert, so the example runs
//! as if the variable were unset: a typo never half-drives a window.
//!
//! # Why a keyboard *message* and not `ButtonInput`
//!
//! The press is written as a [`KeyboardInput`] message in `PreUpdate`, before `InputSystems`, and
//! Bevy's own `keyboard_input_system` turns it into `ButtonInput<KeyCode>`. So an example reading
//! either the resource (`just_pressed`) or the message stream sees the same press on the same frame
//! a real key would have produced. Poking `ButtonInput` directly would satisfy only the first kind.
//!
//! A shot is taken in `Update` of the frame it names and lands one or two frames later — readback is
//! asynchronous — so put `exit` a few dozen frames after the last `shot`.

use std::path::{Path, PathBuf};

use bevy::{
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput, NativeKey},
    },
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};

/// The script. Absent means the plugin does nothing at all.
const SCRIPT_VAR: &str = "CARNAGE_AUTOPILOT";

/// Where `shot` writes. Required exactly when the script contains a `shot`.
const OUT_VAR: &str = "CARNAGE_AUTOPILOT_OUT";

/// **Register this after `DefaultPlugins`.** See the module header for what it reads and does.
pub struct Autopilot;

/// What a script step does that is not a key.
enum Step {
    /// Save the primary window to this path.
    Shot(PathBuf),
    /// Quit.
    Exit,
}

/// **A parsed script, and how far it has run.**
///
/// Two lists rather than one tagged list, because the two kinds are consumed by two systems in two
/// schedules and neither has any use for the other's entries. Both are sorted by frame, stably, so
/// two events on one frame keep the order the script wrote them in — a `key=` release and the next
/// token's press on the same frame arrive release-first.
///
/// An entry is **due** when its frame is `<=` the current one, not `==`: if the primary window is
/// missing on the frame an input was due, the input fires on the next frame instead of being lost.
#[derive(Resource)]
struct AutopilotScript {
    /// `(frame, key, press or release)`.
    keys: Vec<(u32, KeyCode, ButtonState)>,
    /// `(frame, step)`.
    steps: Vec<(u32, Step)>,
    /// `Update` runs completed so far. Frame 0 is the first.
    frame: u32,
}

impl Plugin for Autopilot {
    fn build(&self, app: &mut App) {
        let script = match std::env::var(SCRIPT_VAR) {
            Ok(script) => script,
            // The normal case. Nothing is added to the app.
            Err(std::env::VarError::NotPresent) => return,
            Err(e) => {
                error!("autopilot: {SCRIPT_VAR}: {e}");
                return;
            }
        };
        let out = std::env::var_os(OUT_VAR).map(PathBuf::from);
        let script = match parse(&script, out.as_deref()) {
            Ok(script) => script,
            Err(reason) => {
                error!("autopilot: {reason}");
                return;
            }
        };
        // A shot is the one action that writes to disk, so it is the one that needs the directory to
        // exist. `parse` has already refused a script that has a shot and no `out`.
        if let Some(out) = out.as_deref()
            && script.steps.iter().any(|(_, step)| matches!(step, Step::Shot(_)))
            && let Err(e) = std::fs::create_dir_all(out)
        {
            error!("autopilot: cannot create {}: {e}", out.display());
            return;
        }
        info!(
            "autopilot: {} key events, {} steps{}",
            script.keys.len(),
            script.steps.len(),
            out.as_deref().map(|o| format!(", shots to {}", o.display())).unwrap_or_default(),
        );
        app.insert_resource(script)
            .add_systems(PreUpdate, inject.before(InputSystems))
            .add_systems(Update, act)
            .add_systems(Last, tick);
    }
}

/// Parse a script. The `Err` is the reason, worded for an `error!` line.
fn parse(spec: &str, out: Option<&Path>) -> Result<AutopilotScript, String> {
    let mut keys = Vec::new();
    let mut steps = Vec::new();
    let mut tokens = 0usize;
    for token in spec.split_whitespace() {
        tokens += 1;
        let (frame, action) = token
            .split_once(':')
            .ok_or_else(|| format!("`{token}`: expected <frame>:<action>"))?;
        let frame: u32 = frame
            .parse()
            .map_err(|_| format!("`{token}`: `{frame}` is not a frame number (a u32)"))?;
        let after = |n: u32| {
            frame
                .checked_add(n)
                .ok_or_else(|| format!("`{token}`: frame {frame} + {n} overflows a u32"))
        };

        if action == "shot" {
            let out = out.ok_or_else(|| {
                format!("`{token}`: a shot needs {OUT_VAR}, and it is not set")
            })?;
            steps.push((frame, Step::Shot(out.join(format!("f{frame:05}.png")))));
        } else if action == "exit" {
            steps.push((frame, Step::Exit));
        } else if let Some(name) = action.strip_prefix("key=") {
            let key = key_code(name).ok_or_else(|| format!("`{token}`: unknown key `{name}`"))?;
            keys.push((frame, key, ButtonState::Pressed));
            keys.push((after(1)?, key, ButtonState::Released));
        } else if let Some(rest) = action.strip_prefix("hold=") {
            let (name, n) = rest
                .split_once('*')
                .ok_or_else(|| format!("`{token}`: expected hold=<Name>*<frames>"))?;
            let key = key_code(name).ok_or_else(|| format!("`{token}`: unknown key `{name}`"))?;
            let n = n
                .parse::<u32>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or_else(|| format!("`{token}`: `{n}` is not a hold length (a u32 >= 1)"))?;
            keys.push((frame, key, ButtonState::Pressed));
            keys.push((after(n)?, key, ButtonState::Released));
        } else {
            return Err(format!(
                "`{token}`: unknown action `{action}` (shot, key=<Name>, hold=<Name>*<n>, exit)"
            ));
        }
    }
    if tokens == 0 {
        return Err(format!("{SCRIPT_VAR} is set but holds no <frame>:<action> tokens"));
    }
    // Stable, so same-frame events keep the script's order.
    keys.sort_by_key(|(frame, ..)| *frame);
    steps.sort_by_key(|(frame, _)| *frame);
    Ok(AutopilotScript { keys, steps, frame: 0 })
}

/// A `KeyCode` variant name to the key. Explicit, so the set a script may name is the set written
/// here and nothing else: adding a key an example binds is one line.
fn key_code(name: &str) -> Option<KeyCode> {
    Some(match name {
        "Digit0" => KeyCode::Digit0,
        "Digit1" => KeyCode::Digit1,
        "Digit2" => KeyCode::Digit2,
        "Digit3" => KeyCode::Digit3,
        "Digit4" => KeyCode::Digit4,
        "Digit5" => KeyCode::Digit5,
        "Digit6" => KeyCode::Digit6,
        "Digit7" => KeyCode::Digit7,
        "Digit8" => KeyCode::Digit8,
        "Digit9" => KeyCode::Digit9,
        "KeyA" => KeyCode::KeyA,
        "KeyB" => KeyCode::KeyB,
        "KeyC" => KeyCode::KeyC,
        "KeyD" => KeyCode::KeyD,
        "KeyE" => KeyCode::KeyE,
        "KeyF" => KeyCode::KeyF,
        "KeyG" => KeyCode::KeyG,
        "KeyH" => KeyCode::KeyH,
        "KeyI" => KeyCode::KeyI,
        "KeyJ" => KeyCode::KeyJ,
        "KeyK" => KeyCode::KeyK,
        "KeyL" => KeyCode::KeyL,
        "KeyM" => KeyCode::KeyM,
        "KeyN" => KeyCode::KeyN,
        "KeyO" => KeyCode::KeyO,
        "KeyP" => KeyCode::KeyP,
        "KeyQ" => KeyCode::KeyQ,
        "KeyR" => KeyCode::KeyR,
        "KeyS" => KeyCode::KeyS,
        "KeyT" => KeyCode::KeyT,
        "KeyU" => KeyCode::KeyU,
        "KeyV" => KeyCode::KeyV,
        "KeyW" => KeyCode::KeyW,
        "KeyX" => KeyCode::KeyX,
        "KeyY" => KeyCode::KeyY,
        "KeyZ" => KeyCode::KeyZ,
        "Space" => KeyCode::Space,
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "BracketLeft" => KeyCode::BracketLeft,
        "BracketRight" => KeyCode::BracketRight,
        "Comma" => KeyCode::Comma,
        "Period" => KeyCode::Period,
        "Escape" => KeyCode::Escape,
        _ => return None,
    })
}

/// Write every key event that is due as the message a real keypress would have been. Runs before
/// `InputSystems`, so `keyboard_input_system` folds it into `ButtonInput<KeyCode>` this frame.
///
/// With no primary window there is nothing for the input to be *addressed* to, so it waits — and
/// stays due, rather than being consumed and lost.
fn inject(
    mut script: ResMut<AutopilotScript>,
    window: Query<Entity, With<PrimaryWindow>>,
    mut keyboard: MessageWriter<KeyboardInput>,
) {
    let Ok(window) = window.single() else { return };
    let frame = script.frame;
    for (_, key_code, state) in script.keys.extract_if(.., |(due, ..)| *due <= frame) {
        keyboard.write(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state,
            text: None,
            repeat: false,
            window,
        });
    }
}

/// Take the screenshots and quit, for every step that is due.
fn act(
    mut script: ResMut<AutopilotScript>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let frame = script.frame;
    for (_, step) in script.steps.extract_if(.., |(due, _)| *due <= frame) {
        match step {
            Step::Shot(path) => {
                commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
            }
            Step::Exit => {
                exit.write(AppExit::Success);
            }
        }
    }
}

/// End of the frame: the next `Update` is the next frame number.
fn tick(mut script: ResMut<AutopilotScript>) {
    script.frame = script.frame.saturating_add(1);
}
