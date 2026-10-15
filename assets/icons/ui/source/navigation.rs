//! Navigation drawing definitions. Camera projection and picking remain in the UI widgets.
use crate::theme::Tokens;
use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, pos2, vec2};
use solidcraft_engine::geom::Vec3;
pub(crate) struct Facet {
    pub(crate) direction: Vec3,
    pub(crate) points: Vec<Pos2>,
    pub(crate) outer_edges: Vec<bool>,
    pub(crate) centre: Pos2,
    pub(crate) label: String,
    pub(crate) text_axes: Option<(egui::Vec2, egui::Vec2)>,
    pub(crate) depth: f64,
}
// Private marker for label meshes with atlas-pixel UVs. The output hook replaces
// it with the font texture before any renderer sees the frame.
pub(crate) const LABEL_PIXELS: egui::TextureId = egui::TextureId::User(u64::from_le_bytes(*b"cubetext"));

struct LabelAtlas;

impl egui::plugin::Plugin for LabelAtlas {
    fn debug_name(&self) -> &'static str {
        "navigation cube label atlas"
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let size = ctx.fonts(|f| f.font_image_size());
        let normalize = vec2(1.0 / size[0].max(1) as f32, 1.0 / size[1].max(1) as f32);
        for shape in &mut output.shapes {
            if let Shape::Mesh(mesh) = &mut shape.shape
                && mesh.texture_id == LABEL_PIXELS
            {
                let mesh = std::sync::Arc::make_mut(mesh);
                mesh.texture_id = egui::TextureId::default();
                for v in &mut mesh.vertices {
                    v.uv = (v.uv.to_vec2() * normalize).to_pos2();
                }
            }
        }
    }
}

/// Warp the font mesh into the projected face, so labels turn with the model.
pub(crate) fn face_text(painter: &egui::Painter, face: &Facet, ink: Color32, font_size: f32) {
    let Some((r, down)) = face.text_axes else { return };
    let opacity = ((face.depth - 0.18) / 0.17).clamp(0.0, 1.0) as f32;
    if opacity <= 0.0 {
        return;
    }
    let ink = ink.gamma_multiply(opacity);
    let galley = painter.layout_no_wrap(face.label.clone(), FontId::proportional(font_size), ink);
    let centre = galley.rect.center();
    // Keep atlas-pixel UVs until the frame is complete: another label or widget
    // can grow the atlas after this mesh is created, invalidating early UVs.
    painter.ctx().add_plugin(LabelAtlas);
    let text = Shape::Text(egui::epaint::TextShape::new(Pos2::ZERO, galley, ink));
    let atlas = painter.ctx().fonts(|f| f.font_image_size());
    let primitives =
        painter.ctx().tessellate(vec![egui::epaint::ClippedShape { clip_rect: Rect::EVERYTHING, shape: text }], painter.ctx().pixels_per_point());
    for primitive in primitives {
        if let egui::epaint::Primitive::Mesh(mut mesh) = primitive.primitive {
            mesh.texture_id = LABEL_PIXELS;
            for v in &mut mesh.vertices {
                v.uv = pos2(v.uv.x * atlas[0] as f32, v.uv.y * atlas[1] as f32);
                let local = v.pos - centre;
                v.pos = face.centre + r * local.x + down * (local.y * 1.35);
            }
            painter.add(Shape::mesh(mesh));
        }
    }
}

pub(crate) fn paint_cube(painter: &egui::Painter, faces: &[Facet], hovered: Option<usize>, t: Tokens, labels: bool, scale: f32) {
    // A shared, opaque triangle mesh has no antialiasing fringes between facets.
    // Per-polygon fringes and miter joins overlap as a face becomes edge-on.
    let mut fill_mesh = egui::epaint::Mesh::default();
    let mut edges: Vec<(Pos2, Pos2, usize, bool)> = Vec::new();
    let hovered_direction = hovered.and_then(|i| faces.get(i)).map(|f| f.direction);
    for face in faces {
        let base = t.cube_face;
        let shade = (0.72 + 0.28 * face.depth.min(1.0)) as f32;
        let highlighted = hovered_direction == Some(face.direction);
        let fill = if highlighted {
            t.cube_hover
        } else {
            Color32::from_rgb((f32::from(base.r()) * shade) as u8, (f32::from(base.g()) * shade) as u8, (f32::from(base.b()) * shade) as u8)
        };
        let centre = fill_mesh.vertices.len() as u32;
        fill_mesh.colored_vertex(face.centre, fill);
        for &p in &face.points {
            fill_mesh.colored_vertex(p, fill);
        }
        for (j, (&a, &b)) in face.points.iter().zip(face.points.iter().cycle().skip(1)).take(face.points.len()).enumerate() {
            fill_mesh.add_triangle(centre, centre + 1 + j as u32, centre + 1 + ((j + 1) % face.points.len()) as u32);
            // Coplanar picking regions have no visible seams. Only actual cube
            // edges and the hovered target's boundary receive an outline.
            let outer = face.outer_edges.get(j).copied().unwrap_or(false);
            if !highlighted && !outer {
                continue;
            }
            if let Some(edge) = edges
                .iter_mut()
                .find(|(p, q, _, _)| (p.distance_sq(a) < 1e-6 && q.distance_sq(b) < 1e-6) || (p.distance_sq(b) < 1e-6 && q.distance_sq(a) < 1e-6))
            {
                edge.2 += usize::from(highlighted);
                edge.3 |= outer;
            } else {
                edges.push((a, b, usize::from(highlighted), outer));
            }
        }
    }
    painter.add(Shape::mesh(fill_mesh));
    for (a, b, highlights, outer) in edges {
        // Two highlighted coplanar patches share an internal subdivision, not
        // a target boundary. Actual cube edges remain visible across faces.
        if !outer && highlights != 1 {
            continue;
        }
        let highlighted = highlights > 0;
        painter.line_segment([a, b], Stroke::new(1.0, if highlighted { t.accent } else { t.cube_border }));
    }
    if labels {
        for face in faces {
            face_text(painter, face, t.cube_text, 14.0 * scale);
        }
    }
}
pub(crate) fn arrow(painter: &egui::Painter, at: Pos2, direction: egui::Vec2, hover: bool, t: Tokens) {
    let side = vec2(-direction.y, direction.x);
    painter.add(Shape::convex_polygon(
        vec![at + direction * 7.0, at - direction * 5.0 + side * 7.0, at - direction * 5.0 - side * 7.0],
        if hover { t.accent_soft } else { t.cube_face },
        Stroke::new(1.0, if hover { t.accent } else { t.cube_border }),
    ));
}
pub(crate) fn curved_arrow(painter: &egui::Painter, at: Pos2, clockwise: bool, hover: bool, t: Tokens, scale: f32) {
    let (start, end) =
        if clockwise { (-std::f32::consts::FRAC_PI_4 + 0.12, 0.0) } else { (-std::f32::consts::FRAC_PI_4 - 0.12, -std::f32::consts::FRAC_PI_2) };
    let points: Vec<_> = (0..=16)
        .map(|i| {
            let angle = egui::lerp(start..=end, f32::from(i as u8) / 16.0);
            at + vec2(angle.cos(), angle.sin()) * (32.0 * scale)
        })
        .collect();
    painter.add(Shape::line(points.clone(), Stroke::new(5.0 * scale, if hover { t.accent } else { t.cube_border })));
    painter.add(Shape::line(points.clone(), Stroke::new(3.0 * scale, if hover { t.accent_soft } else { t.cube_face })));
    if let Some(tip) = points.last() {
        let sign = if clockwise { 1.0 } else { -1.0 };
        arrow(painter, *tip, vec2(-end.sin(), end.cos()) * (sign * scale), hover, t);
    }
}
pub(crate) const AXIS_RADIUS: f32 = 11.0;

/// The token colour of the axis a direction lies along.
fn axis_colour(d: &Vec3, t: &Tokens) -> Color32 {
    if d.x != 0.0 {
        t.triad[0]
    } else if d.y != 0.0 {
        t.triad[1]
    } else {
        t.triad[2]
    }
}

/// 0 for a point at the back of the sphere, 1 for the front: how much of the way to full strength.
fn facing(depth: f64) -> f32 {
    ((depth as f32).clamp(-1.0, 1.0) * 0.5 + 0.5).powf(0.8)
}

/// The orbit gizmo: a shaded sphere with one ring per axis plane, turning with the camera, and the
/// six signed axes as marks on it. `basis` is the camera's (right, up, back), `radius` the
/// sphere's, `tips` the signed axes sorted back to front, `active` while the pointer is over the
/// gizmo or dragging it. Rings and marks behind the sphere are drawn first and fainter, so the
/// front ones cross over them.
pub(crate) fn orbit_gizmo(
    painter: &egui::Painter,
    c: Pos2,
    basis: (Vec3, Vec3, Vec3),
    radius: f32,
    tips: &[(&'static str, Vec3, Pos2, f64)],
    hovered: Option<usize>,
    active: bool,
    t: Tokens,
) {
    let (right, up, back) = basis;
    let project = |v: Vec3| (c + vec2((v.dot(right) as f32) * radius, -(v.dot(up) as f32) * radius), v.dot(back));
    // The sphere: a soft body, a lighter spot where light would fall, and a fine outline.
    let body = radius + 10.0;
    painter.circle_filled(c, body, t.overlay.gamma_multiply(if active { 1.0 } else { 0.6 }));
    // Stacked, shrinking discs give the highlight a soft edge.
    for k in 0..5 {
        let r = body * (0.62 - 0.1 * k as f32);
        painter.circle_filled(c + vec2(-body * 0.28, -body * 0.32), r, t.cube_face.gamma_multiply(if active { 0.05 } else { 0.026 }));
    }
    painter.circle_stroke(c, body, Stroke::new(1.0, t.cube_border.gamma_multiply(if active { 0.9 } else { 0.5 })));

    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    let ring = |k: usize, front: bool| {
        let (a, b) = (axes[(k + 1) % 3], axes[(k + 2) % 3]);
        let n = 72;
        let pts: Vec<_> = (0..=n)
            .map(|i| {
                let th = std::f64::consts::TAU * f64::from(i) / f64::from(n);
                project(a * th.cos() + b * th.sin())
            })
            .collect();
        for w in pts.windows(2) {
            let (p0, d0) = w[0];
            let (p1, d1) = w[1];
            let depth = (d0 + d1) * 0.5;
            if (depth >= 0.0) != front {
                continue;
            }
            let strength = facing(depth);
            let colour = t.triad[k].gamma_multiply(0.18 + 0.82 * strength * if active { 1.0 } else { 0.8 });
            painter.line_segment([p0, p1], Stroke::new(if front { 1.0 + 1.2 * strength } else { 1.0 }, colour));
        }
    };
    let mark = |i: usize, tip: &(&'static str, Vec3, Pos2, f64)| {
        let (name, d, at, depth) = tip;
        let positive = !name.starts_with('-');
        let hot = hovered == Some(i);
        let ink = axis_colour(d, &t).gamma_multiply(0.45 + 0.55 * facing(*depth));
        let scale = 0.86 + 0.14 * facing(*depth);
        if positive {
            let r = 10.0 * scale;
            painter.circle_filled(*at, r + 1.5, t.panel.gamma_multiply(0.55));
            painter.circle_filled(*at, r, if hot { t.accent_soft } else { t.panel });
            painter.circle_stroke(*at, r, Stroke::new(if hot { 2.2 } else { 1.8 }, if hot { t.accent } else { ink }));
            // One fixed label size: a size that changes with the view would rasterize new glyphs every frame.
            painter.text(*at, Align2::CENTER_CENTER, name.to_uppercase(), FontId::proportional(11.0), if hot { t.text } else { ink });
        } else {
            let r = if hot { 9.0 } else { 4.6 * scale };
            painter.circle_filled(*at, r, if hot { t.accent_soft } else { ink.gamma_multiply(0.85) });
            if hot {
                painter.circle_stroke(*at, r, Stroke::new(2.0, t.accent));
                painter.text(*at, Align2::CENTER_CENTER, name.to_uppercase(), FontId::proportional(10.0), t.text);
            }
        }
    };
    for k in 0..3 {
        ring(k, false);
    }
    for (i, tip) in tips.iter().enumerate().filter(|(_, tip)| tip.3 < 0.0) {
        mark(i, tip);
    }
    // Thin spokes from the centre to the positive axes, then the near halves of the rings.
    for (_, d, at, depth) in tips.iter().filter(|tip| !tip.0.starts_with('-')) {
        painter.line_segment([c, *at], Stroke::new(1.3, axis_colour(d, &t).gamma_multiply(0.3 + 0.5 * facing(*depth))));
    }
    for k in 0..3 {
        ring(k, true);
    }
    for (i, tip) in tips.iter().enumerate().filter(|(_, tip)| tip.3 >= 0.0) {
        mark(i, tip);
    }
}
