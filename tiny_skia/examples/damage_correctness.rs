//! Damage-tracking correctness harness for the software renderer.
//!
//! Renders a sidebar-shaped scene (cards whose background color flips on
//! "hover", an actions pill that appears/disappears, a scroll-like shift,
//! text edits) twice per frame:
//!
//! - **incremental**: the real compositor path, `Layer::damage` diff
//!   against the previous frame's layers, `damage::group`, then a
//!   partial `Renderer::draw` restricted to the grouped regions;
//! - **reference**: a fresh full-damage draw of the same content.
//!
//! Any byte difference between the two buffers is an under-paint (or a
//! clip leak) in the damage path. Run on two revisions to bisect a
//! visual regression:
//!
//! ```bash
//! cargo run --release --example damage_correctness --features geometry
//! ```
//!
//! Exit code is non-zero on the first mismatching frame, and the two
//! buffers are dumped as PNGs next to the target dir for eyeballing.

use iced_tiny_skia::Renderer;
use iced_tiny_skia::core::renderer::{self, Quad, Renderer as _};
use iced_tiny_skia::core::{
    Background, Border, Color, Font, Pixels, Point, Rectangle, Size,
};
use iced_tiny_skia::geometry::Frame;
use iced_tiny_skia::graphics::Viewport;
use iced_tiny_skia::graphics::damage;
use iced_tiny_skia::graphics::geometry::frame::Backend as _;
use iced_tiny_skia::graphics::geometry::{self, Renderer as _};
use iced_tiny_skia::Layer;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 700;
/// Sidebar strip on the right, terminal-like canvas on the left.
const SIDEBAR_X: f32 = 600.0;
const CARDS: usize = 8;
const CARD_H: f32 = 56.0;

/// One step of the scripted interaction. Mirrors what the real sidebar
/// does between frames.
#[derive(Clone, Copy, Debug)]
struct Scene {
    /// Card index whose background is in the "hovered" tint.
    hovered: Option<usize>,
    /// Card index showing the floating actions pill (extra quads).
    pill: Option<usize>,
    /// Vertical scroll offset applied to every card.
    scroll: f32,
    /// Content stamp for one card's text (text edit).
    stamp: usize,
    /// Content stamp for the terminal-like canvas text.
    term_stamp: usize,
}

fn script() -> Vec<(&'static str, Scene)> {
    let base = Scene {
        hovered: None,
        pill: None,
        scroll: 0.0,
        stamp: 0,
        term_stamp: 0,
    };
    vec![
        ("base", base),
        ("hover card 2", Scene { hovered: Some(2), ..base }),
        ("pill on card 2", Scene { hovered: Some(2), pill: Some(2), ..base }),
        ("pill moves to 5", Scene { hovered: Some(5), pill: Some(5), ..base }),
        ("hover off", base),
        ("scroll 24px", Scene { scroll: 24.0, ..base }),
        ("scroll back", base),
        ("text edit", Scene { stamp: 1, ..base }),
        ("terminal output", Scene { stamp: 1, term_stamp: 1, ..base }),
        ("terminal + hover", Scene { stamp: 1, term_stamp: 2, hovered: Some(1), ..base }),
        ("identical frame", Scene { stamp: 1, term_stamp: 2, hovered: Some(1), ..base }),
        ("everything off", Scene { stamp: 1, term_stamp: 2, ..base }),
    ]
}

/// Paints `scene` into `renderer` (which must have been `reset`).
fn paint(renderer: &mut Renderer, scene: &Scene) {
    let bounds = Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32));

    // Sidebar panel background.
    renderer.fill_quad(
        Quad {
            bounds: Rectangle::new(
                Point::new(SIDEBAR_X, 0.0),
                Size::new(WIDTH as f32 - SIDEBAR_X, HEIGHT as f32),
            ),
            ..Quad::default()
        },
        Background::Color(Color::from_rgb(0.09, 0.10, 0.12)),
    );

    // Cards: rounded quads whose fill flips on hover, like the snippet
    // rows. Same bounds either way, only the color changes, exactly the
    // diff the sidebar exercise showed dropping.
    for card in 0..CARDS {
        let y = 16.0 + card as f32 * (CARD_H + 8.0) - scene.scroll;
        let hovered = scene.hovered == Some(card);
        renderer.fill_quad(
            Quad {
                bounds: Rectangle::new(
                    Point::new(SIDEBAR_X + 12.0, y),
                    Size::new(WIDTH as f32 - SIDEBAR_X - 24.0, CARD_H),
                ),
                border: Border {
                    color: Color::from_rgb(0.2, 0.7, 0.6),
                    width: if hovered { 1.0 } else { 0.0 },
                    radius: 8.0.into(),
                },
                ..Quad::default()
            },
            Background::Color(if hovered {
                Color::from_rgb(0.16, 0.20, 0.24)
            } else {
                Color::from_rgb(0.12, 0.14, 0.17)
            }),
        );

        // Floating actions pill (three small quads) over the card.
        if scene.pill == Some(card) {
            for action in 0..3 {
                renderer.fill_quad(
                    Quad {
                        bounds: Rectangle::new(
                            Point::new(
                                WIDTH as f32 - 130.0 + action as f32 * 36.0,
                                y + 12.0,
                            ),
                            Size::new(30.0, 30.0),
                        ),
                        border: Border { radius: 6.0.into(), ..Border::default() },
                        ..Quad::default()
                    },
                    Background::Color(Color::from_rgb(0.20, 0.24, 0.30)),
                );
            }
        }
    }

    // Text on the cards + the terminal-like canvas on the left: geometry
    // text reaches the engine as cached runs with their own clip bounds,
    // the same shape the embedded terminal produces.
    let mut frame = Frame::new(bounds);
    for card in 0..CARDS {
        let y = 16.0 + card as f32 * (CARD_H + 8.0) - scene.scroll;
        let stamp = if card == 3 { scene.stamp } else { 0 };
        frame.fill_text(geometry::Text {
            content: format!("snippet-{card} v{stamp} echo hello"),
            position: Point::new(SIDEBAR_X + 22.0, y + 18.0),
            color: Color::WHITE,
            size: Pixels(13.0),
            font: Font::MONOSPACE,
            ..geometry::Text::default()
        });
    }
    for row in 0..24 {
        frame.fill_text(geometry::Text {
            content: format!("{:04} build output line {row}", scene.term_stamp),
            position: Point::new(8.0, 8.0 + row as f32 * 22.0),
            color: Color::from_rgb(0.8, 0.85, 0.8),
            size: Pixels(14.0),
            font: Font::MONOSPACE,
            ..geometry::Text::default()
        });
    }
    renderer.draw_geometry(frame.into_geometry());
}

fn main() {
    let bounds = Rectangle::with_size(Size::new(WIDTH as f32, HEIGHT as f32));
    let viewport = Viewport::with_physical_size(Size::new(WIDTH, HEIGHT), 1.0);
    let background = Color::from_rgb(0.05, 0.05, 0.06);

    // Incremental world: persistent renderer + buffer, partial draws.
    let mut inc_renderer = Renderer::new(renderer::Settings::default());
    let mut inc_buffer = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
    let mut inc_mask = tiny_skia::Mask::new(WIDTH, HEIGHT).expect("mask");
    let mut last_layers: Vec<Layer> = Vec::new();

    // Reference world: persistent renderer (caches allowed), but every
    // frame is drawn with FULL damage into a fresh-cleared buffer.
    let mut ref_renderer = Renderer::new(renderer::Settings::default());
    let mut ref_buffer = vec![0_u8; (WIDTH * HEIGHT * 4) as usize];
    let mut ref_mask = tiny_skia::Mask::new(WIDTH, HEIGHT).expect("mask");

    let mut failures = 0;

    for (index, (label, scene)) in script().into_iter().enumerate() {
        // --- incremental path (the real compositor recipe) ---
        inc_renderer.reset(bounds);
        paint(&mut inc_renderer, &scene);

        let current_layers = inc_renderer.layers().to_vec();
        let raw_damage = if last_layers.is_empty() {
            vec![bounds]
        } else {
            damage::diff(
                &last_layers,
                &current_layers,
                |layer| vec![layer.bounds],
                Layer::damage,
            )
        };
        last_layers = current_layers;

        if !raw_damage.is_empty() {
            let grouped = damage::group(raw_damage, bounds);
            let mut pixels =
                tiny_skia::PixmapMut::from_bytes(&mut inc_buffer, WIDTH, HEIGHT)
                    .expect("pixmap");
            inc_renderer.draw(&mut pixels, &mut inc_mask, &viewport, &grouped, background);
        }

        // --- reference: full redraw of the same content ---
        ref_renderer.reset(bounds);
        paint(&mut ref_renderer, &scene);
        ref_buffer.fill(0);
        let mut pixels =
            tiny_skia::PixmapMut::from_bytes(&mut ref_buffer, WIDTH, HEIGHT).expect("pixmap");
        ref_renderer.draw(&mut pixels, &mut ref_mask, &viewport, &[bounds], background);

        // --- compare ---
        let diff_pixels = inc_buffer
            .chunks_exact(4)
            .zip(ref_buffer.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        if diff_pixels == 0 {
            println!("frame {index:02} {label:>18}: OK");
        } else {
            failures += 1;
            // Bounding box of the mismatch, to see WHERE it under-paints.
            let (mut min_x, mut min_y, mut max_x, mut max_y) =
                (u32::MAX, u32::MAX, 0_u32, 0_u32);
            for (i, (a, b)) in inc_buffer
                .chunks_exact(4)
                .zip(ref_buffer.chunks_exact(4))
                .enumerate()
            {
                if a != b {
                    let (x, y) = (i as u32 % WIDTH, i as u32 / WIDTH);
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
            println!(
                "frame {index:02} {label:>18}: MISMATCH {diff_pixels} px in \
                 ({min_x},{min_y})-({max_x},{max_y})"
            );
            // Raw RGBA dumps for eyeballing (`magick -size 900x700 -depth 8
            // rgba:/tmp/damage_inc_NN.raw out.png` if ever needed).
            let _ = std::fs::write(format!("/tmp/damage_inc_{index:02}.raw"), &inc_buffer);
            let _ = std::fs::write(format!("/tmp/damage_ref_{index:02}.raw"), &ref_buffer);
        }
    }

    if failures > 0 {
        println!("{failures} mismatching frame(s)");
        std::process::exit(1);
    }
    println!("all frames match");
}
