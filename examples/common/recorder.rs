//! **The headless recorder both capture examples drive.** Native only.
//!
//! Rendering a demo to PNGs needs the same eight awkward things every time: `DefaultPlugins` with no
//! window and no winit, shaders compiled synchronously so frame 0 is not empty, a render target that
//! is not a surface, a camera pointed at it, a hand-pumped update loop, a device poll so a screenshot
//! does not read a half-drawn frame, and a few extra pumps at the end because screenshot readback
//! lands a frame or two late. None of that is interesting, all of it is easy to get subtly wrong, and
//! there are several recorders — so it lives here once rather than several times.
//!
//! What each recorder still owns is its *scene* and its *script*: what stands there, what happens to
//! it, and on which frame. That is the part worth reading.
//!
//! # Why this is its own file, gated on the target
//!
//! **It carries all three of this crate's wasm blockers** — `std::fs::create_dir_all`, `save_to_disk`
//! screenshots, and a blocking `device.poll` — and `common/body.rs` reaches the shared `material`
//! helper with `use super::material`, so importing the demo *subject* used to drag the recorder into
//! the module tree and break the wasm build on `std::fs`.
//!
//! Splitting it out is not a compatibility shim. **A recorder writes PNGs to a filesystem, and a
//! browser has no filesystem** — "absent in a browser" is a fact about the platform, not a choice
//! between two implementations. `common/mod.rs` declares this module
//! `#[cfg(not(target_arch = "wasm32"))]` for exactly that reason.

use bevy::{
    app::SubApps,
    asset::RenderAssetUsages,
    camera::RenderTarget,
    prelude::*,
    render::{
        RenderPlugin,
        render_resource::{Extent3d, PollType, TextureDimension, TextureFormat, TextureUsages},
        renderer::RenderDevice,
        view::screenshot::{Screenshot, save_to_disk},
    },
    window::ExitCondition,
    winit::WinitPlugin,
};

/// A headless Bevy app that writes one PNG per frame.
pub struct Recorder {
    app: SubApps,
    target: Handle<Image>,
    out: String,
    frame: u32,
}

impl Recorder {
    /// Build the app, its off-screen target and a camera looking through `camera`.
    ///
    /// `None` — with an `error!` naming the path — if the output directory cannot be created. A
    /// recorder that cannot write is refused at the door rather than discovering it 100 frames in.
    pub fn new(width: u32, height: u32, camera: Transform, out: &str) -> Option<Recorder> {
        Recorder::new_with(width, height, camera, out, |_| {})
    }

    /// [`Recorder::new`], plus a chance to add plugins.
    ///
    /// **This exists because a plugin cannot be added afterwards.** `new` calls `finish` and `cleanup`
    /// and then takes the sub-apps, which is what makes a hand-pumped loop possible at all — and after
    /// `cleanup` an `App` will not accept another plugin. `app()` is enough for systems and is what
    /// the fracture recorders use; a recorder that needs a *render* plugin (particles) has no other
    /// way in.
    ///
    /// One implementation with a no-op closure behind `new`, rather than two constructors that could
    /// drift about the eight awkward things this module exists to get right once.
    pub fn new_with(
        width: u32,
        height: u32,
        camera: Transform,
        out: &str,
        add_plugins: impl FnOnce(&mut App),
    ) -> Option<Recorder> {
        if let Err(e) = std::fs::create_dir_all(out) {
            error!("capture: cannot create {out}: {e}");
            return None;
        }

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                // Every shader must be ready before the first frame, or frame 0 renders empty.
                .set(RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
                .disable::<WinitPlugin>(),
        );
        // Before `finish`/`cleanup`, which is the only window in which a plugin can still be added.
        add_plugins(&mut app);
        // `run()` is never called, so the two things the runner would have done must be done here.
        app.finish();
        app.cleanup();
        let mut app = std::mem::take(app.sub_apps_mut());

        let mut image = Image::new_uninit(
            Extent3d { width, height, depth_or_array_layers: 1 },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT;
        let target = app.main.world_mut().resource_mut::<Assets<Image>>().add(image);

        app.main.world_mut().spawn((Camera3d::default(), RenderTarget::from(target.clone()), camera));

        Some(Recorder { app, target, out: out.to_string(), frame: 0 })
    }

    /// The main world, for building a scene or changing it between frames.
    pub fn world(&mut self) -> &mut World {
        self.app.main.world_mut()
    }

    /// The sub-apps, for the one thing `world()` cannot do: adding a system after `cleanup`.
    pub fn app(&mut self) -> &mut SubApps {
        &mut self.app
    }

    /// Screenshot the world as it stands, then advance it one frame.
    pub fn shoot(&mut self) {
        let path = format!("{}/frame{:04}.png", self.out, self.frame);
        self.app.main.world_mut().spawn(Screenshot::image(self.target.clone())).observe(save_to_disk(path));
        self.frame += 1;
        self.step();
    }

    /// Draw and discard `frames` frames, so the first *recorded* one is not empty.
    ///
    /// **Needed, and it shows up in the worst place.** The first frame or two after a scene is built
    /// come back blank — the pipeline has not drawn yet — which is invisible mid-clip and is exactly
    /// what a viewer sees as the still preview of a GIF. Call it after the scene exists and before
    /// the recording loop; anything the scene animates has not started yet, so nothing moves.
    pub fn warm_up(&mut self, frames: u32) {
        for _ in 0..frames {
            self.step();
        }
    }

    /// Advance one frame without recording it.
    pub fn step(&mut self) {
        self.app.update();
        // Without the wait, a screenshot can read back a half-drawn target.
        let device = self.app.main.world().resource::<RenderDevice>().wgpu_device().clone();
        if let Err(e) = device.poll(PollType::Wait { submission_index: None, timeout: None }) {
            warn!("capture: device poll failed, frame may be torn: {e:?}");
        }
    }

    /// Pump past the last screenshot so its readback lands, and report how many frames were written.
    pub fn finish(mut self) -> u32 {
        for _ in 0..4 {
            self.step();
        }
        self.frame
    }
}

