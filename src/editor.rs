//! Annotation model + rendering.
//!
//! Annotations are stored in **image-pixel coordinates** so the on-screen preview
//! (drawn with egui) and the final exported PNG (rendered with tiny-skia) come
//! from one source of truth and stay WYSIWYG.

use image::RgbaImage;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Line,
    Arrow,
    Rect,
}

#[derive(Clone)]
pub enum Shape {
    Pen(Vec<[f32; 2]>),
    Line([f32; 2], [f32; 2]),
    Arrow([f32; 2], [f32; 2]),
    Rect([f32; 2], [f32; 2]),
}

#[derive(Clone)]
pub struct Annot {
    pub shape: Shape,
    pub color: [u8; 4],
    pub width: f32,
}

/// Arrowhead wings for a segment a->b. Returns the two endpoints of the head.
fn arrowhead(a: [f32; 2], b: [f32; 2], width: f32) -> ([f32; 2], [f32; 2]) {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let (ux, uy) = (dx / len, dy / len);
    let head = (width * 4.0).max(14.0);
    let ang = 0.45_f32; // ~26 degrees
    let (s, c) = (ang.sin(), ang.cos());
    let p1 = [
        b[0] + head * (ux * c - uy * s),
        b[1] + head * (ux * s + uy * c),
    ];
    let p2 = [
        b[0] + head * (ux * c + uy * s),
        b[1] + head * (-ux * s + uy * c),
    ];
    (p1, p2)
}

// ---------------------------------------------------------------------------
// egui preview rendering
// ---------------------------------------------------------------------------

/// Map from image-pixel coords to on-screen coords.
pub struct View {
    pub origin: egui::Pos2, // top-left of image on screen
    pub scale: f32,         // screen px per image px
}

impl View {
    pub fn to_screen(&self, p: [f32; 2]) -> egui::Pos2 {
        egui::pos2(self.origin.x + p[0] * self.scale, self.origin.y + p[1] * self.scale)
    }
}

pub fn draw_egui(painter: &egui::Painter, a: &Annot, view: &View) {
    let col = egui::Color32::from_rgba_unmultiplied(a.color[0], a.color[1], a.color[2], a.color[3]);
    let w = (a.width * view.scale).max(1.0);
    let stroke = egui::Stroke::new(w, col);
    match &a.shape {
        Shape::Pen(pts) => {
            let screen: Vec<egui::Pos2> = pts.iter().map(|p| view.to_screen(*p)).collect();
            if screen.len() >= 2 {
                painter.add(egui::Shape::line(screen, stroke));
            }
        }
        Shape::Line(p, q) => {
            painter.line_segment([view.to_screen(*p), view.to_screen(*q)], stroke);
        }
        Shape::Arrow(p, q) => {
            painter.line_segment([view.to_screen(*p), view.to_screen(*q)], stroke);
            let (h1, h2) = arrowhead(*p, *q, a.width);
            painter.line_segment([view.to_screen(*q), view.to_screen(h1)], stroke);
            painter.line_segment([view.to_screen(*q), view.to_screen(h2)], stroke);
        }
        Shape::Rect(p, q) => {
            let r = egui::Rect::from_two_pos(view.to_screen(*p), view.to_screen(*q));
            painter.rect_stroke(r, 0.0, stroke);
        }
    }
}

// ---------------------------------------------------------------------------
// tiny-skia export
// ---------------------------------------------------------------------------

/// Render the base image with annotations baked in and encode it as PNG bytes.
pub fn render_png(base: &RgbaImage, annots: &[Annot]) -> anyhow::Result<Vec<u8>> {
    use tiny_skia::*;

    let (w, h) = base.dimensions();
    let mut pixmap = Pixmap::new(w, h).ok_or_else(|| anyhow::anyhow!("bad image size"))?;

    // Base image is opaque, so straight RGBA == premultiplied RGBA.
    pixmap.data_mut().copy_from_slice(base.as_raw());

    for a in annots {
        let mut paint = Paint::default();
        paint.set_color_rgba8(a.color[0], a.color[1], a.color[2], a.color[3]);
        paint.anti_alias = true;

        let mut stroke = Stroke::default();
        stroke.width = a.width;
        stroke.line_cap = LineCap::Round;
        stroke.line_join = LineJoin::Round;

        let path = match &a.shape {
            Shape::Pen(pts) => {
                if pts.len() < 2 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(pts[0][0], pts[0][1]);
                for p in &pts[1..] {
                    pb.line_to(p[0], p[1]);
                }
                pb.finish()
            }
            Shape::Line(p, q) => {
                let mut pb = PathBuilder::new();
                pb.move_to(p[0], p[1]);
                pb.line_to(q[0], q[1]);
                pb.finish()
            }
            Shape::Arrow(p, q) => {
                let (h1, h2) = arrowhead(*p, *q, a.width);
                let mut pb = PathBuilder::new();
                pb.move_to(p[0], p[1]);
                pb.line_to(q[0], q[1]);
                pb.move_to(q[0], q[1]);
                pb.line_to(h1[0], h1[1]);
                pb.move_to(q[0], q[1]);
                pb.line_to(h2[0], h2[1]);
                pb.finish()
            }
            Shape::Rect(p, q) => {
                let r = Rect::from_ltrb(p[0].min(q[0]), p[1].min(q[1]), p[0].max(q[0]), p[1].max(q[1]));
                r.map(PathBuilder::from_rect)
            }
        };

        if let Some(path) = path {
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }

    // tiny-skia stores premultiplied RGBA; un-premultiply before re-encoding.
    let mut out = RgbaImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(pixmap.pixels()) {
        let a = src.alpha();
        let (r, g, b) = if a == 0 {
            (0, 0, 0)
        } else {
            (
                ((src.red() as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
                ((src.green() as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
                ((src.blue() as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
            )
        };
        *dst = image::Rgba([r, g, b, a]);
    }

    let mut buf = std::io::Cursor::new(Vec::new());
    out.write_to(&mut buf, image::ImageFormat::Png)?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_png_roundtrips_dimensions() {
        let base = RgbaImage::from_pixel(120, 80, image::Rgba([10, 20, 30, 255]));
        let annots = vec![
            Annot {
                shape: Shape::Arrow([5.0, 5.0], [100.0, 60.0]),
                color: [237, 28, 36, 255],
                width: 4.0,
            },
            Annot {
                shape: Shape::Rect([10.0, 10.0], [60.0, 50.0]),
                color: [0, 162, 232, 255],
                width: 3.0,
            },
            Annot {
                shape: Shape::Pen(vec![[1.0, 1.0], [20.0, 30.0], [40.0, 10.0]]),
                color: [34, 177, 76, 255],
                width: 2.0,
            },
        ];
        let png = render_png(&base, &annots).expect("render");
        // Must be a valid PNG that decodes back to the same size.
        let decoded = image::load_from_memory(&png).expect("decode").to_rgba8();
        assert_eq!(decoded.dimensions(), (120, 80));
        // Somewhere the red arrow must have changed a pixel from the flat background.
        let changed = decoded.pixels().any(|p| p.0 != [10, 20, 30, 255]);
        assert!(changed, "annotations should alter pixels");
    }

    #[test]
    fn arrowhead_points_back_toward_start() {
        let (h1, h2) = arrowhead([0.0, 0.0], [100.0, 0.0], 4.0);
        // For a rightward arrow the wings sit left of the tip (x < 100).
        assert!(h1[0] < 100.0 && h2[0] < 100.0);
    }
}
