//! A surface renderer draws a Vello scene with the path renderer the window's device already has.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zgui_geom::{Scale, Size};
use zgui_platform::Surface;
use zgui_platform_headless::Harness;
use zgui_render::RenderTarget;
use zgui_render_wgpu::{Builder, WgpuRenderer, wgpu};
use zgui_runtime::{App, AppError, Runtime};
use zgui_view::{BuildCx, IntoView, View};
use zgui_wgpu::vello::kurbo::{Affine, Rect};
use zgui_wgpu::vello::peniko::{Color, Fill};
use zgui_wgpu::{SurfaceElementExt, SurfaceRenderCx, SurfaceRenderer, WgpuSurfaces, vello};

/// The colour the scene fills its middle with, as stored bytes.
const FILLED: [u8; 4] = [255, 0, 0, 255];

/// The colour behind the scene, as stored bytes.
const BASE: [u8; 4] = [0, 0, 255, 255];

/// What a render read back from the texture: the texel at the centre and at a corner.
type Read = Arc<Mutex<Option<([u8; 4], [u8; 4])>>>;

/// Draws a square in the middle of the surface with Vello, then reads two texels back.
struct Square {
    /// The usages it asks for.
    usage: wgpu::TextureUsages,
    /// What it read.
    read: Read,
}

impl SurfaceRenderer for Square {
    fn usage(&self) -> wgpu::TextureUsages {
        self.usage
    }

    fn render(&mut self, cx: &mut SurfaceRenderCx<'_>) {
        let (width, height) = (f64::from(cx.size.width), f64::from(cx.size.height));
        let mut scene = vello::Scene::new();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(255, 0, 0, 255),
            None,
            &Rect::new(width / 4.0, height / 4.0, width * 0.75, height * 0.75),
        );
        cx.render_vello(&scene, Color::from_rgba8(0, 0, 255, 255))
            .expect("the device runs Vello");
        let centre = texel(cx, cx.size.width / 2, cx.size.height / 2);
        let corner = texel(cx, 1, 1);
        *self.read.lock().unwrap() = Some((centre, corner));
    }
}

/// Copies the texel at `(x, y)` to a buffer and reads it.
fn texel(cx: &SurfaceRenderCx<'_>, x: u32, y: u32) -> [u8; 4] {
    let buffer = cx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test.vello.readback"),
        size: 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = cx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: cx.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout::default(),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    cx.queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    cx.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the copy completes");
    let view = buffer
        .slice(..)
        .get_mapped_range()
        .expect("the buffer is mapped");
    view[..].try_into().expect("four bytes were copied")
}

/// A real wgpu renderer drawing into an offscreen texture, or `None` on a machine without one.
fn device() -> Option<WgpuRenderer> {
    let target = RenderTarget::new(Size::new(64, 64), Scale::new(1.0));
    match Builder::new().offscreen(target, wgpu::TextureFormat::Bgra8Unorm, false) {
        Ok(renderer) => Some(renderer),
        Err(failure) => {
            eprintln!("skipped: no usable graphics device ({failure})");
            None
        }
    }
}

/// A window showing `surface`, drawn by `renderer`.
fn app(renderer: WgpuRenderer, surface: Square) -> Harness<Runtime> {
    let mut renderer = Some(renderer);
    let mut surface = Some(surface);
    let handler = App::new()
        .with_title("vello")
        .with_size(64.0, 64.0)
        .with_stylesheet(
            "root { display: block; width: 64px; height: 64px } surface { width: 32px; height: 32px }",
        )
        .with_renderer(Box::new(
            move |_surface: &Arc<dyn Surface>, target| -> Result<_, AppError> {
                let mut renderer = renderer.take().expect("one window asks for one renderer");
                zgui_render::Renderer::configure(&mut renderer, target);
                Ok(Box::new(renderer) as Box<dyn zgui_render::Renderer>)
            },
        ))
        .with_text_engine(Box::new(|| {
            Box::new(zgui_layout::Paragraphs::new(
                zgui_testkit_scene::MonoShaper::new(),
            ))
        }))
        .with_glyph_raster(Box::new(|| {
            Arc::new(zgui_testkit_scene::MonoRaster::new())
        }))
        .into_handler(move |cx: &mut BuildCx<'_>| {
            let surface = surface.take().expect("the view is built once");
            Box::new(
                zgui_elements::r#box()
                    .class("root")
                    .child(zgui_elements::surface().renderer(surface))
                    .into_view()
                    .build(cx),
            )
        })
        .expect("the reactive runtime installs");
    let mut harness = Harness::new(handler);
    harness.app_mut().windows_mut()[0].install_embed_host(Box::new(WgpuSurfaces::new()));
    harness
}

#[test]
fn a_renderer_draws_a_vello_scene_into_its_texture() {
    let Some(renderer) = device() else {
        return;
    };
    let read = Read::default();
    let mut app = app(
        renderer,
        Square {
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            read: Arc::clone(&read),
        },
    );
    app.run_for(Duration::from_millis(50), Duration::from_millis(16));
    let (centre, corner) = read.lock().unwrap().expect("the surface rendered");
    assert_eq!(centre, FILLED, "the scene was drawn");
    assert_eq!(corner, BASE, "on the base colour");
}

#[test]
fn drawing_with_vello_without_storage_binding_says_what_is_missing() {
    let Some(renderer) = device() else {
        return;
    };
    let mut app = app(
        renderer,
        Square {
            usage: wgpu::TextureUsages::COPY_SRC,
            read: Read::default(),
        },
    );
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.run_for(Duration::from_millis(50), Duration::from_millis(16));
    }))
    .expect_err("drawing without STORAGE_BINDING is refused");
    let message = refused
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| refused.downcast_ref::<&str>().copied())
        .unwrap_or_default();
    assert!(
        message.contains("return STORAGE_BINDING from SurfaceRenderer::usage"),
        "the panic names the fix: {message}"
    );
}
