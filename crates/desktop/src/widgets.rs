//! Small drawing helpers. The message list paints rows directly (no nested
//! layouts) and only for visible rows, which keeps scrolling at full frame rate.

use std::sync::Arc;

use apark_core::MsgRow;
use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{
    self, text::LayoutJob, Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Sense, Stroke, TextFormat, Ui,
};

pub const ROW_HEIGHT: f32 = 68.0;

pub fn account_color(email: &str) -> Color32 {
    // FNV-1a so similar addresses still land on different hues.
    let h = email.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193));
    let hue = (h % 3600) as f32 / 3600.0;
    egui::ecolor::Hsva::new(hue, 0.55, 0.85, 1.0).into()
}

pub fn short_time(ts: i64) -> String {
    let Some(t) = Local.timestamp_opt(ts, 0).single() else { return String::new() };
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if (now.date_naive() - t.date_naive()).num_days() == 1 {
        "昨天".into()
    } else if t.year() == now.year() {
        format!("{}月{}日", t.month(), t.day())
    } else {
        t.format("%Y/%m/%d").to_string()
    }
}

pub fn full_time(ts: i64) -> String {
    Local.timestamp_opt(ts, 0).single().map(|t| t.format("%Y年%m月%d日 %H:%M").to_string()).unwrap_or_default()
}

pub fn human_size(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

/// One line of text, cut with an ellipsis at `max_w`.
pub fn line(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap.max_width = max_w.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    p.layout_job(job)
}

/// Draw one message row; returns the click response.
pub fn message_row(ui: &mut Ui, m: &MsgRow, selected: bool, show_account: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let v = ui.visuals();
    let p = ui.painter();
    if selected {
        p.rect_filled(rect.shrink2(egui::vec2(4.0, 1.0)), 6.0, v.selection.bg_fill.gamma_multiply(0.55));
    } else if resp.hovered() {
        p.rect_filled(rect.shrink2(egui::vec2(4.0, 1.0)), 6.0, v.widgets.hovered.weak_bg_fill);
    }
    let strong = v.strong_text_color();
    let normal = v.text_color();
    let weak = v.weak_text_color();
    let left = rect.left() + 26.0;
    let right = rect.right() - 12.0;

    if show_account {
        p.rect_filled(
            Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.top() + 10.0), egui::vec2(3.0, ROW_HEIGHT - 20.0)),
            1.5,
            account_color(&m.account),
        );
    }
    if !m.seen {
        p.circle_filled(Pos2::new(rect.left() + 17.0, rect.top() + 18.0), 4.0, Color32::from_rgb(10, 132, 255));
    }

    let date = line(p, &short_time(m.date), FontId::proportional(12.0), weak, 90.0);
    let date_w = date.size().x;
    p.galley(Pos2::new(right - date_w, rect.top() + 10.0), date, weak);

    let sender_font = FontId::proportional(14.5);
    let sender = line(p, m.sender(), sender_font, if m.seen { normal } else { strong }, right - left - date_w - 10.0);
    p.galley(Pos2::new(left, rect.top() + 8.0), sender, strong);

    let subject = if m.subject.is_empty() { "（无主题）" } else { m.subject.as_str() };
    let star_w = if m.flagged { 18.0 } else { 0.0 };
    let subj = line(p, subject, FontId::proportional(13.5), if m.seen { normal } else { strong }, right - left - star_w);
    p.galley(Pos2::new(left, rect.top() + 28.0), subj, normal);
    if m.flagged {
        p.text(Pos2::new(right, rect.top() + 28.0), Align2::RIGHT_TOP, "★", FontId::proportional(13.0), Color32::from_rgb(255, 179, 0));
    }

    if !m.snippet.is_empty() {
        let snip = line(p, &m.snippet, FontId::proportional(12.5), weak, right - left);
        p.galley(Pos2::new(left, rect.top() + 47.0), snip, weak);
    }
    p.hline(
        rect.x_range().shrink(12.0),
        rect.bottom() - 0.5,
        Stroke::new(0.5, v.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.6)),
    );
    resp
}

/// Round avatar with the sender's initial.
pub fn avatar(ui: &mut Ui, name: &str, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let p = ui.painter();
    p.circle_filled(rect.center(), size / 2.0, color);
    let initial: String = name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    p.text(rect.center(), Align2::CENTER_CENTER, initial, FontId::proportional(size * 0.45), Color32::WHITE);
}

/// App icon: blue rounded square with a white envelope, drawn procedurally.
pub fn icon(size: u32) -> egui::IconData {
    let s = size as f32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let radius = s * 0.22;
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded-square mask.
            let cx = fx.clamp(radius, s - radius);
            let cy = fy.clamp(radius, s - radius);
            if (fx - cx).hypot(fy - cy) > radius {
                continue;
            }
            let t = fy / s;
            let mut c = [(20.0 + 20.0 * t) as u8, (120.0 - 30.0 * t) as u8, (255.0 - 40.0 * t) as u8, 255u8];
            // Envelope.
            let (l, r, top, bot) = (s * 0.2, s * 0.8, s * 0.3, s * 0.7);
            let th = (s * 0.045).max(1.0);
            if fx >= l && fx <= r && fy >= top && fy <= bot {
                let border = fx - l < th || r - fx < th || fy - top < th || bot - fy < th;
                // The flap: two lines from top corners meeting at the centre.
                let mid = s * 0.5;
                let flap_y = top + (fx - l).min(r - fx) * ((s * 0.52 - top) / (mid - l));
                let flap = (fy - flap_y).abs() < th * 1.1 && fy <= s * 0.53 + th;
                if border || flap {
                    c = [255, 255, 255, 255];
                }
            }
            let i = ((y * size + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&c);
        }
    }
    egui::IconData { rgba, width: size, height: size }
}
