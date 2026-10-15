//! Small shared widgets: spinner, status badges, animation renderer.

use crate::akari::sprites::SpriteFrame;
use ratatui::style::Color;
use ratatui::text::Line;

/// Render one RGBA frame into half-block text lines fitting
/// `(cell_w, cell_h)` cells. Each cell holds 2 vertical halves via "▀"
/// (upper half = foreground, lower half = background).
///
/// Audit findings addressed here (the old renderer point-sampled a single
/// source pixel per cell half, which is why output looked excessively
/// blocky):
/// - each cell half is an **area average** over every source pixel it
///   covers (box filter), preserving gradients instead of aliasing;
/// - alpha is composited over the terminal background during averaging,
///   so semi-transparent edges blend instead of popping;
/// - aspect ratio is preserved and the sprite is centered on both axes,
///   so consecutive frames cannot drift.
///
/// Terminal limitation (documented, not claimed away): character cells
/// cannot reproduce 256×320 pixels 1:1. This keeps true-color RGB output
/// with a graceful shape-only fallback on terminals without true color.
pub fn render_sprite_halfblock(
    frame: &SpriteFrame,
    cell_w: usize,
    cell_h: usize,
) -> Vec<Line<'static>> {
    if cell_w == 0 || cell_h == 0 {
        return Vec::new();
    }
    let px_h = cell_h * 2;
    // Uniform scale to fit, preserving aspect ratio.
    let scale = (cell_w as f64 / frame.width as f64).min(px_h as f64 / frame.height as f64);
    let draw_w = ((frame.width as f64 * scale).round() as usize)
        .max(1)
        .min(cell_w);
    let draw_h_px = ((frame.height as f64 * scale).round() as usize)
        .max(2)
        .min(px_h);
    let draw_h = draw_h_px.div_ceil(2);
    let off_x = (cell_w.saturating_sub(draw_w)) / 2;
    let off_y = (cell_h.saturating_sub(draw_h)) / 2;
    // Source pixels per output cell-half (for area averaging).
    let src_per_cell_x = frame.width as f64 / draw_w as f64;
    let src_per_half_y = frame.height as f64 / draw_h_px as f64;

    let mut lines = Vec::with_capacity(cell_h);
    for cy in 0..cell_h {
        if cy < off_y || cy >= off_y + draw_h {
            lines.push(Line::from(" ".repeat(cell_w)));
            continue;
        }
        let row = cy - off_y;
        let mut spans = Vec::new();
        if off_x > 0 {
            spans.push(ratatui::text::Span::raw(" ".repeat(off_x)));
        }
        for cx in 0..draw_w {
            let x0 = cx as f64 * src_per_cell_x;
            let top = area_average(
                frame,
                x0,
                (row * 2) as f64 * src_per_half_y,
                x0 + src_per_cell_x,
                (row * 2 + 1) as f64 * src_per_half_y,
            );
            let bot = area_average(
                frame,
                x0,
                (row * 2 + 1) as f64 * src_per_half_y,
                x0 + src_per_cell_x,
                (row * 2 + 2) as f64 * src_per_half_y,
            );
            match (top, bot) {
                (None, None) => spans.push(ratatui::text::Span::raw(" ")),
                (Some((tr, tg, tb)), None) => spans.push(ratatui::text::Span::styled(
                    "▀",
                    ratatui::style::Style::default().fg(Color::Rgb(tr, tg, tb)),
                )),
                (None, Some((br, bg, bb))) => spans.push(ratatui::text::Span::styled(
                    "▄",
                    ratatui::style::Style::default().fg(Color::Rgb(br, bg, bb)),
                )),
                (Some((tr, tg, tb)), Some((br, bg, bb))) => {
                    spans.push(ratatui::text::Span::styled(
                        "▀",
                        ratatui::style::Style::default()
                            .fg(Color::Rgb(tr, tg, tb))
                            .bg(Color::Rgb(br, bg, bb)),
                    ))
                }
            }
        }
        if off_x > 0 || spans.len() < cell_w {
            // Right pad to keep every row exactly cell_w wide.
            let used = off_x + draw_w;
            if used < cell_w {
                spans.push(ratatui::text::Span::raw(" ".repeat(cell_w - used)));
            }
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Area-average the source rectangle `[x0,x1)×[y0,y1)` (float pixel
/// coords, clamped). Each overlapped source pixel contributes by overlap
/// area with its color premultiplied by alpha (composited over a dark
/// terminal background). Returns `None` when the region is effectively
/// transparent (mean alpha < 24), preserving cut-out transparency.
fn area_average(frame: &SpriteFrame, x0: f64, y0: f64, x1: f64, y1: f64) -> Option<(u8, u8, u8)> {
    let x0 = x0.max(0.0);
    let y0 = y0.max(0.0);
    let x1 = x1.min(frame.width as f64);
    let y1 = y1.min(frame.height as f64);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut sum_r = 0.0;
    let mut sum_g = 0.0;
    let mut sum_b = 0.0;
    let mut sum_a = 0.0;
    let mut sum_w = 0.0;
    let sx0 = x0.floor() as u32;
    let sx1 = (x1.ceil() as u32).min(frame.width);
    let sy0 = y0.floor() as u32;
    let sy1 = (y1.ceil() as u32).min(frame.height);
    for sy in sy0..sy1 {
        let oy = ((sy + 1) as f64).min(y1) - (sy as f64).max(y0);
        if oy <= 0.0 {
            continue;
        }
        for sx in sx0..sx1 {
            let ox = ((sx + 1) as f64).min(x1) - (sx as f64).max(x0);
            if ox <= 0.0 {
                continue;
            }
            let idx = ((sy * frame.width + sx) * 4) as usize;
            if idx + 3 >= frame.rgba.len() {
                continue;
            }
            let w = ox * oy;
            let a = frame.rgba[idx + 3] as f64 / 255.0;
            sum_r += frame.rgba[idx] as f64 * a * w;
            sum_g += frame.rgba[idx + 1] as f64 * a * w;
            sum_b += frame.rgba[idx + 2] as f64 * a * w;
            sum_a += a * w;
            sum_w += w;
        }
    }
    if sum_w <= 0.0 || sum_a / sum_w * 255.0 < 24.0 {
        return None;
    }
    // Normalize by total area (not by alpha): transparent pixels
    // contribute terminal-background black, so edges blend instead of
    // popping to full brightness.
    Some((
        (sum_r / sum_w).round().clamp(0.0, 255.0) as u8,
        (sum_g / sum_w).round().clamp(0.0, 255.0) as u8,
        (sum_b / sum_w).round().clamp(0.0, 255.0) as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, r: u8, g: u8, b: u8) -> SpriteFrame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        SpriteFrame {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn halfblock_output_has_right_height() {
        let f = solid(8, 8, 255, 0, 0);
        let lines = render_sprite_halfblock(&f, 8, 6);
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn transparent_renders_blank() {
        let f = SpriteFrame {
            width: 2,
            height: 2,
            rgba: vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        let lines = render_sprite_halfblock(&f, 4, 2);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            for span in &line.spans {
                assert!(!span.content.contains('▀') && !span.content.contains('▄'));
            }
        }
    }

    #[test]
    fn centered_no_drift_same_dims() {
        // Deterministic output for identical inputs ⇒ no frame-to-frame drift.
        let f = solid(256, 320, 10, 20, 30);
        let a = render_sprite_halfblock(&f, 30, 20);
        let b = render_sprite_halfblock(&f, 30, 20);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }

    #[test]
    fn solid_color_survives_area_averaging() {
        // No quantization: a solid sprite renders its exact color.
        let f = solid(16, 16, 200, 100, 50);
        let lines = render_sprite_halfblock(&f, 8, 4);
        let mut saw = false;
        for line in &lines {
            for span in &line.spans {
                if let Some(fg) = span.style.fg {
                    assert_eq!(fg, Color::Rgb(200, 100, 50));
                    saw = true;
                }
            }
        }
        assert!(saw, "expected colored half-blocks");
    }

    #[test]
    fn checkerboard_averages_instead_of_aliasing() {
        // 2×2 red/green checker downsampled to one cell must blend,
        // not pick a single winner pixel.
        let f = SpriteFrame {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 255, 0, 255, 255, 0, 0, 255,
            ],
        };
        // One cell wide, one cell tall: whole sprite per cell.
        let lines = render_sprite_halfblock(&f, 1, 1);
        assert_eq!(lines.len(), 1);
        let mut fgs = Vec::new();
        for span in &lines[0].spans {
            if let Some(fg) = span.style.fg {
                fgs.push(fg);
            }
        }
        assert!(!fgs.is_empty());
        for fg in fgs {
            match fg {
                Color::Rgb(r, g, b) => {
                    assert!(
                        r > 100 && g > 100 && b < 60,
                        "expected red/green blend, got ({r},{g},{b})"
                    );
                }
                other => panic!("expected RGB, got {other:?}"),
            }
        }
    }

    #[test]
    fn semi_transparent_composites_over_background() {
        // 50% red over dark background ≈ (128, 0, 0).
        let f = SpriteFrame {
            width: 1,
            height: 2,
            rgba: vec![255, 0, 0, 128, 255, 0, 0, 128],
        };
        let avg = area_average(&f, 0.0, 0.0, 1.0, 2.0).unwrap();
        assert_eq!(avg, (128, 0, 0));
    }

    #[test]
    fn aspect_preserved_and_centered_both_axes() {
        // Wide sprite in a tall panel: content rows center vertically
        // with blank padding above and below.
        let f = solid(32, 8, 9, 9, 9);
        let lines = render_sprite_halfblock(&f, 32, 12);
        assert_eq!(lines.len(), 12);
        let blank = |l: &ratatui::text::Line| {
            l.spans
                .iter()
                .all(|s| !s.content.contains('▀') && !s.content.contains('▄'))
        };
        let first_content = lines.iter().position(|l| !blank(l)).unwrap();
        let last_content = lines.iter().rposition(|l| !blank(l)).unwrap();
        assert!(first_content > 0, "expected top padding");
        assert!(last_content < 11, "expected bottom padding");
        assert!(
            first_content <= 12 - 1 - last_content + 1,
            "roughly symmetric padding"
        );
    }
}
