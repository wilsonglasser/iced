//! Compute the damage between frames.
use crate::core::{Point, Rectangle};

/// Diffs the damage regions given some previous and current primitives.
pub fn diff<T>(
    previous: &[T],
    current: &[T],
    bounds: impl Fn(&T) -> Vec<Rectangle>,
    diff: impl Fn(&T, &T) -> Vec<Rectangle>,
) -> Vec<Rectangle> {
    let damage = previous.iter().zip(current).flat_map(|(a, b)| diff(a, b));

    if previous.len() == current.len() {
        damage.collect()
    } else {
        let (smaller, bigger) = if previous.len() < current.len() {
            (previous, current)
        } else {
            (current, previous)
        };

        // Extend damage by the added/removed primitives
        damage
            .chain(bigger[smaller.len()..].iter().flat_map(bounds))
            .collect()
    }
}

/// Computes the damage regions given some previous and current primitives.
pub fn list<T>(
    previous: &[T],
    current: &[T],
    bounds: impl Fn(&T) -> Vec<Rectangle>,
    are_equal: impl Fn(&T, &T) -> bool,
) -> Vec<Rectangle> {
    diff(previous, current, &bounds, |a, b| {
        if are_equal(a, b) {
            vec![]
        } else {
            bounds(a).into_iter().chain(bounds(b)).collect()
        }
    })
}

/// Groups the given damage regions that are close together inside the given
/// bounds.
///
/// The returned regions never overlap, and when they would cover most of
/// `bounds` anyway the whole thing collapses into a single full repaint.
/// Both properties matter for the software renderer, which runs the full
/// background + layer pass once per region: overlapping regions raster
/// the same pixels twice, and scroll-like frames (old + new bounds of
/// every primitive) used to produce grouped damage totalling 130%+ of
/// the surface.
pub fn group(mut damage: Vec<Rectangle>, bounds: Rectangle) -> Vec<Rectangle> {
    const AREA_THRESHOLD: f32 = 20_000.0;

    /// Grouped share of `bounds` beyond which one full repaint beats
    /// painting the pieces (fewer per-region layer passes, no seams).
    const FULL_REPAINT_SHARE: f32 = 0.8;

    damage.sort_by(|a, b| {
        a.center()
            .distance(Point::ORIGIN)
            .total_cmp(&b.center().distance(Point::ORIGIN))
    });

    let mut output: Vec<Rectangle> = Vec::new();
    let mut scaled = damage
        .into_iter()
        .filter_map(|region| region.intersection(&bounds))
        .filter(|region| region.width >= 1.0 && region.height >= 1.0);

    if let Some(mut current) = scaled.next() {
        for region in scaled {
            let union = current.union(&region);

            if union.area() - current.area() - region.area() <= AREA_THRESHOLD {
                current = union;
            } else {
                output.push(current);
                current = region;
            }
        }

        output.push(current);
    }

    // The greedy pass above only merges neighbours in sort order, so
    // regions far apart in that order can still overlap. Merge any
    // intersecting pair to a fixpoint so the output is disjoint and no
    // pixel is rastered twice. The group count is small (the threshold
    // above keeps it a handful), so the quadratic scan is negligible.
    let mut merged = true;
    while merged {
        merged = false;
        'scan: for i in 0..output.len() {
            for j in (i + 1)..output.len() {
                if output[i].intersects(&output[j]) {
                    let union = output[i].union(&output[j]);
                    output[i] = union;
                    let _ = output.swap_remove(j);
                    merged = true;
                    break 'scan;
                }
            }
        }
    }

    // Disjoint regions covering most of the surface: repaint it whole.
    let covered: f32 = output.iter().map(Rectangle::area).sum();
    if covered >= bounds.area() * FULL_REPAINT_SHARE {
        return vec![bounds];
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Size;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle {
            x,
            y,
            width,
            height,
        }
    }

    const BOUNDS: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 1000.0,
        height: 1000.0,
    };

    /// No pixel may be rastered twice: whatever goes in, the output
    /// regions are pairwise disjoint.
    #[test]
    fn output_regions_never_overlap() {
        // Two wide bands and a tall band crossing both; the greedy
        // neighbour merge alone leaves overlapping groups here.
        let damage = vec![
            rect(0.0, 0.0, 800.0, 100.0),
            rect(300.0, 0.0, 100.0, 500.0),
            rect(0.0, 380.0, 800.0, 100.0),
        ];

        let groups = group(damage, BOUNDS);
        for (i, a) in groups.iter().enumerate() {
            for b in &groups[i + 1..] {
                assert!(
                    !a.intersects(b),
                    "groups must be disjoint, got {a:?} and {b:?}"
                );
            }
        }
    }

    /// Scroll-like damage (old + new bounds of everything, overlapping
    /// heavily) collapses into one full-surface repaint instead of
    /// painting 130%+ of the window in pieces.
    #[test]
    fn near_full_coverage_collapses_to_full_repaint() {
        let damage = vec![
            rect(0.0, 0.0, 1000.0, 700.0),
            rect(0.0, 300.0, 1000.0, 700.0),
        ];

        assert_eq!(group(damage, BOUNDS), vec![BOUNDS]);
    }

    /// Small, far-apart updates (a cursor blink, a clock tick) must NOT
    /// balloon into a full repaint.
    #[test]
    fn sparse_damage_stays_partial() {
        let damage = vec![rect(10.0, 10.0, 20.0, 20.0), rect(900.0, 900.0, 30.0, 15.0)];

        let groups = group(damage, BOUNDS);
        assert_eq!(groups.len(), 2);
        let covered: f32 = groups.iter().map(Rectangle::area).sum();
        assert!(covered < BOUNDS.area() * 0.01);
    }

    /// Regions are clamped to the surface; damage reported past the
    /// edges (scrolled-out content) cannot exceed it.
    #[test]
    fn damage_is_clamped_to_bounds() {
        let damage = vec![rect(-500.0, -500.0, 4000.0, 200.0)];

        let groups = group(damage, BOUNDS);
        let covered: f32 = groups.iter().map(Rectangle::area).sum();
        assert!(covered <= BOUNDS.area() + f32::EPSILON);
        for g in &groups {
            assert_eq!(*g, g.intersection(&BOUNDS).unwrap());
        }
    }

    /// A single full-bounds damage stays exactly one full repaint.
    #[test]
    fn full_bounds_roundtrip() {
        let bounds = Rectangle::with_size(Size::new(320.0, 200.0));
        assert_eq!(group(vec![bounds], bounds), vec![bounds]);
    }
}
