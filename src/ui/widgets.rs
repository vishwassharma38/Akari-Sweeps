//! Small shared widgets: spinner, status badges, animation renderer.

use crate::akari::sprites::SpriteFrame;
use ratatui::style::Color;
use ratatui::text::Line;

/// Render one RGBA frame into half-block text lines fitting
/// `(cell_w, cell_h)` cells. Each cell holds 2 vertical pixels via "▀".
/// Transparent pixels (< 24 alpha) fall back to the terminal background
/// (rendered as dark). Aspect is preserved and the sprite is centered so
/// consecutive frames do not drift.
pub fn render_sprite_halfblock(
    frame: &SpriteFrame,
    cell_w: usize,
    cell_h: usize,
) -> Vec<Line<'static>> {
    if cell_w == 0 || cell_h == 0 {
        return Vec::new();
    }
    let px_h = cell_h * 2;
    // Uniform scale to fit, preserving 256:320 aspect.
    let scale = (cell_w as f64 / frame.width as f64)
        .min(px_h as f64 / frame.height as f64)
        .min(1.0);
    // Never upscale tiny panels beyond 1x; allow modest downscale only.
    // (Panels narrower than the sprite show a centered crop-free shrink.)
    let draw_w = ((frame.width as f64 * scale).round() as usize)
        .max(1)
        .min(cell_w);
    let draw_h_px = ((frame.height as f64 * scale).round() as usize)
        .max(2)
        .min(px_h);
    let draw_h = draw_h_px.div_ceil(2);
    let off_x = (cell_w.saturating_sub(draw_w)) / 2;
    // Offsets map output cells back to source pixels (nearest neighbor).
    let mut lines = Vec::with_capacity(cell_h);
    for cy in 0..cell_h {
        let mut spans = Vec::new();
        // Left padding for centering.
        if off_x > 0 {
            spans.push(ratatui::text::Span::raw(" ".repeat(off_x)));
        }
        if cy < draw_h {
            for cx in 0..draw_w {
                let sx = ((cx as f64 * frame.width as f64) / draw_w as f64) as u32;
                let sy_top =
                    ((cy as usize * 2) as f64 * frame.height as f64 / draw_h_px as f64) as u32;
                let sy_bot = (((cy as usize * 2) + 1) as f64 * frame.height as f64
                    / draw_h_px as f64) as u32;
                let top = sample(frame, sx.min(frame.width - 1), sy_top.min(frame.height - 1));
                let bot = sample(frame, sx.min(frame.width - 1), sy_bot.min(frame.height - 1));
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
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Sample one pixel; `None` when transparent (alpha < 24).
fn sample(frame: &SpriteFrame, x: u32, y: u32) -> Option<(u8, u8, u8)> {
    let idx = ((y * frame.width + x) * 4) as usize;
    if idx + 3 >= frame.rgba.len() {
        return None;
    }
    let a = frame.rgba[idx + 3];
    if a < 24 {
        return None;
    }
    let r = frame.rgba[idx];
    let g = frame.rgba[idx + 1];
    let b = frame.rgba[idx + 2];
    if a < 255 {
        // Blend over near-black terminal background.
        let t = a as f32 / 255.0;
        let blend = |c: u8| (c as f32 * t).round() as u8;
        return Some((blend(r), blend(g), blend(b)));
    }
    Some((r, g, b))
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
    }

    #[test]
    fn centered_no_drift_same_dims() {
        let f = solid(256, 320, 10, 20, 30);
        let a = render_sprite_halfblock(&f, 30, 20);
        let b = render_sprite_halfblock(&f, 30, 20);
        assert_eq!(a.len(), b.len());
    }
}
