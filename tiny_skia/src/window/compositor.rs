use crate::core::backend;
use crate::core::renderer;
use crate::core::{Color, Rectangle, Size};
use crate::graphics::compositor::{self, Information};
use crate::graphics::damage;
use crate::graphics::{Shell, Viewport};
use crate::{Layer, Renderer};

use std::collections::VecDeque;
use std::num::NonZeroU32;

pub struct Compositor {
    context: softbuffer::Context<Box<dyn compositor::Display>>,
}

pub struct Surface {
    window: softbuffer::Surface<Box<dyn compositor::Display>, Box<dyn compositor::Window>>,
    clip_mask: tiny_skia::Mask,
    frames: VecDeque<Frame>,
    max_age: u8,
}

#[derive(Clone)]
struct Frame {
    background: Color,
    layers: Vec<Layer>,
}

impl crate::graphics::Compositor for Compositor {
    type Renderer = Renderer;
    type Surface = Surface;

    async fn new(
        settings: backend::Settings,
        display: impl compositor::Display,
        _compatible_window: impl compositor::Window,
        _shell: Shell,
    ) -> Result<Self, backend::Error> {
        if !settings.backend.is_software() && !settings.backend.matches("tiny-skia") {
            return Err(backend::Error::GraphicsAdapterNotFound {
                backend: "tiny-skia",
                reason: backend::Reason::DidNotMatch {
                    preferred_backend: settings.backend,
                },
            });
        }

        Ok(new(display))
    }

    fn create_renderer(&self, settings: renderer::Settings) -> Self::Renderer {
        Renderer::new(settings)
    }

    fn create_surface(
        &mut self,
        window: impl compositor::Window + Clone,
        width: u32,
        height: u32,
    ) -> Self::Surface {
        let window = softbuffer::Surface::new(&self.context, Box::new(window.clone()) as _)
            .expect("Create softbuffer surface for window");

        let mut surface = Surface {
            window,
            clip_mask: tiny_skia::Mask::new(1, 1).expect("Create clip mask"),
            frames: VecDeque::new(),
            max_age: 0,
        };

        if width > 0 && height > 0 {
            self.configure_surface(&mut surface, width, height);
        }

        surface
    }

    fn configure_surface(&mut self, surface: &mut Self::Surface, width: u32, height: u32) {
        surface
            .window
            .resize(
                NonZeroU32::new(width).expect("Non-zero width"),
                NonZeroU32::new(height).expect("Non-zero height"),
            )
            .expect("Resize surface");

        surface.clip_mask = tiny_skia::Mask::new(width, height).expect("Create clip mask");
        surface.frames.clear();
    }

    fn information(&self) -> Information {
        Information {
            adapter: String::from("CPU"),
            backend: String::from("tiny-skia"),
        }
    }

    fn present(
        &mut self,
        renderer: &mut Self::Renderer,
        surface: &mut Self::Surface,
        viewport: &Viewport,
        background_color: Color,
        on_pre_present: impl FnOnce(),
    ) -> Result<(), compositor::SurfaceError> {
        present(
            renderer,
            surface,
            viewport,
            background_color,
            on_pre_present,
        )
    }

    fn screenshot(
        &mut self,
        renderer: &mut Self::Renderer,
        viewport: &Viewport,
        background_color: Color,
    ) -> Vec<u8> {
        screenshot(renderer, viewport, background_color)
    }
}

pub fn new(display: impl compositor::Display) -> Compositor {
    #[allow(unsafe_code)]
    let context =
        softbuffer::Context::new(Box::new(display) as _).expect("Create softbuffer context");

    Compositor { context }
}

pub fn present(
    renderer: &mut Renderer,
    surface: &mut Surface,
    viewport: &Viewport,
    background: Color,
    on_pre_present: impl FnOnce(),
) -> Result<(), compositor::SurfaceError> {
    let perf_on = sw_perf::enabled();
    let frame_start = perf_on.then(std::time::Instant::now);

    let physical_size = viewport.physical_size();

    let mut buffer = surface
        .window
        .buffer_mut()
        .map_err(|_| compositor::SurfaceError::Lost)?;

    let last_frame = {
        let age = buffer.age();

        surface.max_age = surface.max_age.max(age);
        surface.frames.truncate(surface.max_age as usize);

        if age > 0 {
            surface.frames.get(age as usize - 1)
        } else {
            None
        }
    };

    let damage = last_frame
        .and_then(|last_frame| {
            (last_frame.background == background).then(|| {
                damage::diff(
                    &last_frame.layers,
                    renderer.layers(),
                    |layer| vec![layer.bounds],
                    Layer::damage,
                )
            })
        })
        .unwrap_or_else(|| vec![Rectangle::with_size(viewport.logical_size())]);

    let diff_done = perf_on.then(std::time::Instant::now);
    let mut damage_px = 0.0_f64;

    if damage.is_empty() {
        if let Some(last_frame) = last_frame {
            surface.frames.push_front(last_frame.clone());
        }
    } else {
        surface.frames.push_front(Frame {
            background,
            layers: renderer.layers().to_vec(),
        });

        let damage = damage::group(damage, Rectangle::with_size(viewport.logical_size()));
        if perf_on {
            damage_px = damage
                .iter()
                .map(|r| f64::from(r.width) * f64::from(r.height))
                .sum();
        }

        let mut pixels = tiny_skia::PixmapMut::from_bytes(
            bytemuck::cast_slice_mut(&mut buffer),
            physical_size.width,
            physical_size.height,
        )
        .expect("Create pixel map");

        renderer.draw(
            &mut pixels,
            &mut surface.clip_mask,
            viewport,
            &damage,
            background,
        );
    }

    let draw_done = perf_on.then(std::time::Instant::now);

    on_pre_present();
    let result = buffer.present().map_err(|_| compositor::SurfaceError::Lost);

    if let (Some(started), Some(diff_done), Some(draw_done)) = (frame_start, diff_done, draw_done)
    {
        let logical = viewport.logical_size();
        sw_perf::record(
            diff_done - started,
            draw_done - diff_done,
            draw_done.elapsed(),
            damage_px,
            f64::from(logical.width) * f64::from(logical.height),
        );
    }

    result
}

/// Env-gated software-render frame profiler (`ORYXIS_SW_PERF=1`).
///
/// Splits every presented frame into the three phases that matter for
/// attributing software-mode slowness, and prints a one-line aggregate
/// to stderr about once per second:
///
/// - `diff`: layer diffing to compute damage.
/// - `draw`: tiny-skia rasterization of the damaged regions.
/// - `present`: the softbuffer hand-off to the display server. On
///   remoted compositors (WSLg forwarding over RDP, VNC, ...) this is
///   where the transport cost shows up, so a high `present` with a low
///   `draw` means the platform is the bottleneck, not the renderer.
///
/// `damage` reports how much of the surface was repainted on average,
/// which separates full-window scroll frames from small localized
/// updates.
mod sw_perf {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    pub fn enabled() -> bool {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var("ORYXIS_SW_PERF")
                .map(|v| !v.is_empty() && v != "0")
                .unwrap_or(false)
        })
    }

    #[derive(Default)]
    struct Totals {
        frames: u32,
        diff: Duration,
        diff_max: Duration,
        draw: Duration,
        draw_max: Duration,
        present: Duration,
        present_max: Duration,
        damage_px: f64,
        surface_px: f64,
    }

    struct Window {
        started: Instant,
        totals: Totals,
    }

    static STATS: Mutex<Option<Window>> = Mutex::new(None);

    pub fn record(
        diff: Duration,
        draw: Duration,
        present: Duration,
        damage_px: f64,
        surface_px: f64,
    ) {
        let Ok(mut guard) = STATS.lock() else {
            return;
        };
        let window = guard.get_or_insert_with(|| Window {
            started: Instant::now(),
            totals: Totals::default(),
        });
        let t = &mut window.totals;
        t.frames += 1;
        t.diff += diff;
        t.diff_max = t.diff_max.max(diff);
        t.draw += draw;
        t.draw_max = t.draw_max.max(draw);
        t.present += present;
        t.present_max = t.present_max.max(present);
        t.damage_px += damage_px;
        t.surface_px += surface_px;

        if window.started.elapsed() >= Duration::from_secs(1) {
            let n = f64::from(t.frames.max(1));
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            eprintln!(
                "sw-perf: {} frames | diff avg {:.1} max {:.1} | draw avg {:.1} max {:.1} | \
                 present avg {:.1} max {:.1} (ms) | damage {:.0}% of surface",
                t.frames,
                ms(t.diff) / n,
                ms(t.diff_max),
                ms(t.draw) / n,
                ms(t.draw_max),
                ms(t.present) / n,
                ms(t.present_max),
                if t.surface_px > 0.0 {
                    t.damage_px / t.surface_px * 100.0
                } else {
                    0.0
                },
            );
            *guard = None;
        }
    }
}

pub fn screenshot(
    renderer: &mut Renderer,
    viewport: &Viewport,
    background_color: Color,
) -> Vec<u8> {
    let size = viewport.physical_size();

    let mut offscreen_buffer: Vec<u32> = vec![0; size.width as usize * size.height as usize];

    let mut clip_mask = tiny_skia::Mask::new(size.width, size.height).expect("Create clip mask");

    renderer.draw(
        &mut tiny_skia::PixmapMut::from_bytes(
            bytemuck::cast_slice_mut(&mut offscreen_buffer),
            size.width,
            size.height,
        )
        .expect("Create offscreen pixel map"),
        &mut clip_mask,
        viewport,
        &[Rectangle::with_size(Size::new(
            size.width as f32,
            size.height as f32,
        ))],
        background_color,
    );

    offscreen_buffer.iter().fold(
        Vec::with_capacity(offscreen_buffer.len() * 4),
        |mut acc, pixel| {
            const A_MASK: u32 = 0xFF_00_00_00;
            const R_MASK: u32 = 0x00_FF_00_00;
            const G_MASK: u32 = 0x00_00_FF_00;
            const B_MASK: u32 = 0x00_00_00_FF;

            let a = ((A_MASK & pixel) >> 24) as u8;
            let r = ((R_MASK & pixel) >> 16) as u8;
            let g = ((G_MASK & pixel) >> 8) as u8;
            let b = (B_MASK & pixel) as u8;

            acc.extend([r, g, b, a]);
            acc
        },
    )
}
