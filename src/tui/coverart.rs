//! Render album art with terminal half blocks.

use anyhow::{Result, bail};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

pub fn render_cover(
    data: &[u8],
    width: u16,
    height: u16,
) -> Result<Vec<Line<'static>>> {
    if width == 0 || height == 0 {
        bail!("cover pane is too small");
    }
    let image = image::load_from_memory(data)?;
    let width = width as u32;
    let height = height as u32;
    let target_h = height * 2;
    let scale = f64::min(
        width as f64 / image.width() as f64,
        target_h as f64 / image.height() as f64,
    );
    let dest_w =
        ((image.width() as f64 * scale).round() as u32).clamp(1, width);
    let mut dest_h =
        ((image.height() as f64 * scale).round() as u32).clamp(1, target_h);
    if dest_h % 2 != 0 {
        dest_h = dest_h.saturating_add(1).min(target_h);
    }
    // Box-filter the original pixel format before converting, so large RGB
    // covers never allocate a full-size RGBA copy.
    let resized = image.thumbnail_exact(dest_w, dest_h).to_rgba8();
    let rows = dest_h.div_ceil(2);
    let start_row = (height - rows) / 2;
    let start_col = (width - dest_w) / 2;

    let mut lines = Vec::with_capacity(height as usize);
    for row in 0..height {
        if row < start_row || row >= start_row + rows {
            lines.push(Line::from(" ".repeat(width as usize)));
            continue;
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        if start_col > 0 {
            spans.push(Span::raw(" ".repeat(start_col as usize)));
        }
        let y = (row - start_row) * 2;
        for x in 0..dest_w {
            let top = resized.get_pixel(x, y);
            let bottom = resized.get_pixel(x, (y + 1).min(dest_h - 1));
            let top = Color::Rgb(top[0], top[1], top[2]);
            let bottom = Color::Rgb(bottom[0], bottom[1], bottom[2]);
            spans.push(Span::styled("▀", Style::default().fg(top).bg(bottom)));
        }
        lines.push(Line::from(spans));
    }
    Ok(lines)
}
