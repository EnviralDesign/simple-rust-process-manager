//! Line icons drawn with the painter on a 24-unit grid (the mockup's inline SVGs), so they stay
//! crisp at every scale and need no icon font.

use eframe::egui::{vec2, Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Stop,
    Restart,
    More,
    Search,
    Copy,
    Folder,
    Plus,
    Layers,
    Sliders,
    CaretRight,
    CaretDown,
    ArrowDown,
    Check,
    UpDown,
}

/// Paints `icon` centred in `rect` at `size` points.
pub fn paint(painter: &Painter, rect: Rect, size: f32, icon: Icon, color: Color32) {
    let origin = rect.center() - vec2(size, size) * 0.5;
    let scale = size / 24.0;
    let p = |x: f32, y: f32| -> Pos2 { origin + vec2(x, y) * scale };
    let stroke = Stroke::new((1.7_f32 * scale).max(1.15), color);
    let line = |points: Vec<Pos2>| Shape::line(points, stroke);
    let closed = |points: Vec<Pos2>| Shape::closed_line(points, stroke);
    let arc = |cx: f32, cy: f32, r: f32, from_deg: f32, to_deg: f32| -> Vec<Pos2> {
        let steps = 28;
        (0..=steps)
            .map(|i| {
                let t = from_deg + (to_deg - from_deg) * i as f32 / steps as f32;
                let a = t.to_radians();
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect()
    };

    let shapes: Vec<Shape> = match icon {
        Icon::Play => vec![Shape::convex_polygon(
            vec![p(7.0, 4.5), p(19.5, 12.0), p(7.0, 19.5)],
            color,
            Stroke::NONE,
        )],
        Icon::Stop => vec![Shape::rect_filled(
            Rect::from_min_max(p(6.0, 6.0), p(18.0, 18.0)),
            1.5 * scale,
            color,
        )],
        Icon::Restart => vec![
            // From 3 o'clock round through the bottom to half past one (the SVG's large arc).
            line(arc(12.0, 12.0, 8.0, 0.0, 315.0)),
            line(vec![p(20.0, 4.0), p(20.0, 9.0), p(15.0, 9.0)]),
        ],
        Icon::More => [5.0, 12.0, 19.0]
            .into_iter()
            .map(|x| Shape::circle_filled(p(x, 12.0), 1.8 * scale, color))
            .collect(),
        Icon::Search => vec![
            Shape::circle_stroke(p(11.0, 11.0), 6.5 * scale, stroke),
            line(vec![p(20.0, 20.0), p(15.8, 15.8)]),
        ],
        Icon::Copy => vec![
            Shape::rect_stroke(
                Rect::from_min_max(p(8.0, 8.0), p(20.0, 20.0)),
                2.0 * scale,
                stroke,
                eframe::egui::StrokeKind::Middle,
            ),
            line(vec![
                p(16.0, 8.0),
                p(16.0, 6.0),
                p(14.0, 4.0),
                p(6.0, 4.0),
                p(4.0, 6.0),
                p(4.0, 14.0),
                p(6.0, 16.0),
                p(8.0, 16.0),
            ]),
        ],
        Icon::Folder => vec![closed(vec![
            p(3.0, 7.0),
            p(5.0, 5.0),
            p(9.0, 5.0),
            p(11.0, 7.0),
            p(19.0, 7.0),
            p(21.0, 9.0),
            p(21.0, 17.0),
            p(19.0, 19.0),
            p(5.0, 19.0),
            p(3.0, 17.0),
        ])],
        Icon::Plus => vec![
            line(vec![p(12.0, 5.0), p(12.0, 19.0)]),
            line(vec![p(5.0, 12.0), p(19.0, 12.0)]),
        ],
        Icon::Layers => vec![
            closed(vec![p(12.0, 3.0), p(21.0, 8.0), p(12.0, 13.0), p(3.0, 8.0)]),
            line(vec![p(3.0, 13.0), p(12.0, 18.0), p(21.0, 13.0)]),
        ],
        Icon::Sliders => vec![
            line(vec![p(4.0, 7.0), p(13.0, 7.0)]),
            line(vec![p(17.0, 7.0), p(20.0, 7.0)]),
            line(vec![p(4.0, 17.0), p(7.0, 17.0)]),
            line(vec![p(11.0, 17.0), p(20.0, 17.0)]),
            Shape::circle_stroke(p(15.0, 7.0), 2.0 * scale, stroke),
            Shape::circle_stroke(p(9.0, 17.0), 2.0 * scale, stroke),
        ],
        Icon::CaretRight => vec![line(vec![p(9.0, 6.0), p(15.0, 12.0), p(9.0, 18.0)])],
        Icon::CaretDown => vec![line(vec![p(6.0, 9.0), p(12.0, 15.0), p(18.0, 9.0)])],
        Icon::ArrowDown => vec![
            line(vec![p(12.0, 5.0), p(12.0, 19.0)]),
            line(vec![p(6.0, 13.0), p(12.0, 19.0), p(18.0, 13.0)]),
        ],
        Icon::Check => vec![line(vec![p(5.0, 12.5), p(10.0, 17.0), p(19.0, 7.0)])],
        Icon::UpDown => vec![
            line(vec![p(8.0, 10.0), p(12.0, 6.0), p(16.0, 10.0)]),
            line(vec![p(8.0, 14.0), p(12.0, 18.0), p(16.0, 14.0)]),
        ],
    };
    painter.extend(shapes);
}

/// A rect of `size` centred on `center`, for callers that place icons by position.
pub fn square(center: Pos2, size: f32) -> Rect {
    Rect::from_center_size(center, vec2(size, size))
}
