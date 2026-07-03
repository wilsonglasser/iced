//! Terminal-shaped software rendering benchmark.
//!
//! Reproduces the frame an embedded terminal pane (a `canvas` widget)
//! produces on every output batch: a full-window layer with per-row
//! background quads plus a few text runs per row, drawn with
//! full-window damage. Two workloads:
//!
//! - "scrolling": every row's content changes every frame (build log
//!   at full speed), so text shaping runs cold each frame.
//! - "static": content is stable (idle screen redraw), so the text
//!   cache is hot and raster + clip-mask costs dominate.
//!
//! Canvas text reaches the engine as `Text::Cached` with INFINITE clip
//! bounds, so every run triggers a clip-mask adjustment; before the
//! bounds memoization (iced-rs/iced#3368) each of those was a
//! full-window mask clear + refill. This bench measures exactly that
//! path, run it on two revisions to compare:
//!
//! ```bash
//! cargo run --release --example terminal_frame_bench --features geometry
//! ```
//!
//! Reference numbers, WSL2 dev box, 1200x750 (2026-07-02):
//! pre-memo  (9b4ec059): scrolling 22.8 ms/frame, static 17.8 ms/frame
//! with memo (9cd328d5): scrolling 16.5 ms/frame, static 10.5 ms/frame
//! The ~6.5-7.3 ms/frame delta is the eliminated mask memsets (200
//! adjusts x ~0.9 MB mask); it scales with window area and inversely
//! with memory bandwidth, so old machines save proportionally more.

use std::time::{Duration, Instant};

use iced_tiny_skia::Renderer;
use iced_tiny_skia::core::renderer::{self, Quad, Renderer as _};
use iced_tiny_skia::core::{Background, Color, Font, Pixels, Point, Rectangle, Size};
use iced_tiny_skia::geometry::Frame;
use iced_tiny_skia::graphics::Viewport;
use iced_tiny_skia::graphics::geometry::frame::Backend as _;
use iced_tiny_skia::graphics::geometry::{self, Renderer as _};

const WIDTH: u32 = 1200;
const HEIGHT: u32 = 750;
const ROWS: usize = 40;
const RUNS_PER_ROW: usize = 5;
const WARMUP: usize = 30;
const FRAMES: usize = 300;

fn main() {
    for (label, unique_content) in [("scrolling", true), ("static", false)] {
        let per_frame = run_workload(unique_content);
        println!(
            "{label:>9}: {FRAMES} frames at {WIDTH}x{HEIGHT}, {ROWS} rows x {RUNS_PER_ROW} runs \
             + quads: avg {per_frame:.3} ms/frame ({:.1} fps possible)",
            1000.0 / per_frame
        );
    }
}

/// Renders the workload and returns the average draw time per frame in
/// milliseconds. `unique_content` decides whether every row reshapes
/// every frame (scrolling output) or the text cache stays hot (static
/// screen).
fn run_workload(unique_content: bool) -> f64 {
    let mut renderer = Renderer::new(renderer::Settings::default());
    let viewport = Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0);
    let bounds = Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32));

    let mut buffer = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
    let mut clip_mask = tiny_skia::Mask::new(WIDTH, HEIGHT).expect("Create clip mask");

    let row_height = HEIGHT as f32 / ROWS as f32;
    let run_width = WIDTH as f32 / RUNS_PER_ROW as f32;

    let mut total = Duration::ZERO;

    for frame_index in 0..(WARMUP + FRAMES) {
        renderer.reset(bounds);

        // Row backgrounds, like a selection, a prompt line, a status bar.
        for row in 0..ROWS {
            if row % 4 == 0 {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle::new(
                            Point::new(0.0, row as f32 * row_height),
                            Size::new(WIDTH as f32, row_height),
                        ),
                        ..Quad::default()
                    },
                    Background::Color(Color::from_rgb(0.10, 0.12, 0.14)),
                );
            }
        }

        // The grid: a few styled runs per row.
        let stamp = if unique_content { frame_index } else { 0 };
        let mut frame = Frame::new(bounds);
        for row in 0..ROWS {
            for run in 0..RUNS_PER_ROW {
                frame.fill_text(geometry::Text {
                    content: format!("{stamp:06} r{row:02}c{run} cargo check workspace ok"),
                    position: Point::new(run as f32 * run_width, row as f32 * row_height),
                    max_width: run_width,
                    color: Color::WHITE,
                    size: Pixels(14.0),
                    font: Font::MONOSPACE,
                    ..geometry::Text::default()
                });
            }
        }
        renderer.draw_geometry(frame.into_geometry());

        let mut pixels =
            tiny_skia::PixmapMut::from_bytes(&mut buffer, WIDTH, HEIGHT).expect("Create pixmap");

        let started = Instant::now();
        renderer.draw(
            &mut pixels,
            &mut clip_mask,
            &viewport,
            &[bounds],
            Color::BLACK,
        );
        let elapsed = started.elapsed();

        if frame_index >= WARMUP {
            total += elapsed;
        }
    }

    total.as_secs_f64() * 1e3 / FRAMES as f64
}
