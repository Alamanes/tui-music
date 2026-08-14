use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::time::Duration;

use crate::app::App;
use tui_music::player::RepeatMode;

/// 每帧渲染后回传的可交互区域，供鼠标事件命中测试。
#[derive(Debug, Default, Clone, Copy)]
pub struct LayoutInfo {
    pub header: Rect,
    pub list: Rect,
    pub spectrum: Rect,
    pub footer: Rect,
    /// Now Playing 面板内层区域（面板模式下有效）。
    pub np: Rect,
    /// 进度条（可点击定位）所在行，宽度即进度条长度。
    pub progress: Option<Rect>,
    /// 列表当前滚动偏移（顶部可见项的下标）。
    pub list_offset: usize,
}

pub fn draw(f: &mut Frame, app: &App) -> LayoutInfo {
    if app.show_np {
        return draw_np(f, app, f.area());
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(layout_constraints(f.area().height))
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_search(f, app, chunks[1]);
    let list_offset = draw_list(f, app, chunks[2]);
    draw_spectrum(f, app, chunks[3]);
    let progress = draw_footer(f, app, chunks[4]);
    LayoutInfo {
        header: chunks[0],
        list: chunks[2],
        spectrum: chunks[3],
        footer: chunks[4],
        np: Rect::default(),
        progress,
        list_offset,
    }
}

fn layout_constraints(h: u16) -> Vec<Constraint> {
    if h >= 32 {
        return vec![
            Constraint::Length(3),  // header（含歌单标签）
            Constraint::Length(3),  // search
            Constraint::Min(6),     // list
            Constraint::Length(16), // spectrum
            Constraint::Length(4),  // footer
        ];
    }
    if h >= 14 {
        let fixed = 10u16;
        let rem = h.saturating_sub(fixed);
        let spec = (rem as f32 * 0.4).ceil() as u16;
        let spec = spec.max(2);
        let list = rem.saturating_sub(spec).max(2);
        return vec![
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(list),
            Constraint::Length(spec),
            Constraint::Length(4),
        ];
    }
    let footer_h = h.min(3).max(1);
    let list_h = h.saturating_sub(footer_h).max(1);
    vec![
        Constraint::Length(0),
        Constraint::Length(0),
        Constraint::Length(list_h),
        Constraint::Length(0),
        Constraint::Length(footer_h),
    ]
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    if area.height < 3 || area.width == 0 {
        return;
    }
    let title = Span::styled(
        " tui-music ",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    );
    let inner_w = area.width.saturating_sub(2);
    let dir = if inner_w >= 35 {
        Span::raw(format!("  dir: {}", app.music_dir.display()))
    } else {
        Span::raw("")
    };
    let title_line = Line::from(vec![title, dir]);
    let block = Block::default().borders(Borders::ALL).title(title_line);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || app.playlists.is_empty() {
        return;
    }
    let mut spans: Vec<Span> = Vec::new();
    for (i, pl) in app.playlists.iter().enumerate() {
        let is_active = i == app.active_playlist;
        let style = if is_active {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let marker = if is_active { "▶" } else { " " };
        spans.push(Span::styled(
            format!("[{} {} ({})] ", marker, pl.name, pl.tracks.len()),
            style,
        ));
    }
    let tabs_line = Line::from(spans);
    f.render_widget(Paragraph::new(tabs_line), inner);
}

fn draw_search(f: &mut Frame, app: &App, area: Rect) {
    if area.height < 3 || area.width == 0 {
        return;
    }
    let label = Span::styled(" search ", Style::default().fg(Color::Yellow));
    let query = Span::raw(format!("  {}", app.search));
    let cursor = if app.search_active {
        Span::styled("_", Style::default().add_modifier(Modifier::RAPID_BLINK))
    } else {
        Span::raw("")
    };
    let inner_w = area.width.saturating_sub(2);
    let mut parts = vec![label, query, cursor];
    if inner_w >= 35 {
        let active_total = app.active_tracks().len();
        parts.push(Span::raw(format!(
            "   [{} / {}]",
            app.display.len(),
            active_total
        )));
    }
    let line = Line::from(parts);
    let block = Block::default().borders(Borders::ALL);
    let p = Paragraph::new(line).block(block);
    f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) -> usize {
    if area.height < 3 || area.width == 0 {
        return 0;
    }
    let pl_name = app
        .playlists
        .get(app.active_playlist)
        .map(|p| p.name.as_str())
        .unwrap_or("???");
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" {} ({}) ", pl_name, app.display.len()),
        Style::default().fg(Color::Cyan),
    ));
    let items: Vec<ListItem> = app
        .display
        .iter()
        .map(|&orig| {
            let tracks = app.active_tracks();
            let t = &tracks[orig];
            let is_cur = app.current_track == Some(orig);
            let marker = if is_cur { ">" } else { " " };
            let name = t.display_name();
            let style = if is_cur {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker.to_string(), Style::default().fg(Color::Green)),
                Span::styled(name, style),
                Span::raw("  "),
                Span::styled(t.subtitle(), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::REVERSED),
    );
    let mut state = ListState::default();
    state.select(app.selected);
    f.render_stateful_widget(list, area, &mut state);
    state.offset()
}

/// 经典镜像频谱：中轴上下对称的渐变色块（半块分辨率 ▀/▄/█）
/// + 峰值下落点，宽度填满。
fn draw_spectrum(f: &mut Frame, app: &App, area: Rect) {
    if area.height < 2 || area.width == 0 {
        return;
    }
    let use_border = area.height >= 5;
    let inner = if use_border {
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        f.render_widget(block, area);
        inner
    } else {
        area
    };

    let cols = inner.width as usize;
    if cols == 0 {
        return;
    }
    let bars = resample(&app.smoothed, cols);
    let peaks = resample(&app.peaks, cols);

    let h = inner.height as i32;
    let top = inner.y as i32;
    let bottom = top + h - 1;
    let mid = top + h / 2;
    let rows_up = (mid - top).max(1);
    let rows_dn = (bottom - mid).max(0);

    // 中轴基准线
    for x in inner.x..inner.x + inner.width {
        f.buffer_mut()[(x, mid as u16)]
            .set_char('─')
            .set_style(Style::default().fg(Color::Rgb(60, 62, 80)));
    }

    for i in 0..cols {
        let e = bars[i].clamp(0.0, 1.0);
        let pk = peaks[i].clamp(0.0, 1.0);
        if e <= 0.0 && pk <= 0.0 {
            continue;
        }
        let freq = i as f32 / (cols.saturating_sub(1)).max(1) as f32;
        let color = spectrum_color(freq);
        let x = inner.x + i as u16;

        // 上半（从中轴向顶，半块分辨率）
        let units = e * (rows_up * 2) as f32;
        for r in 0..rows_up {
            let y = mid - 1 - r;
            if y < top {
                break;
            }
            let fade = 1.0 - 0.45 * r as f32 / rows_up as f32;
            if units >= (2 * r + 2) as f32 {
                set_cell(f, x, y, '█', dim_color(color, fade));
            } else if units >= (2 * r + 1) as f32 {
                set_cell(f, x, y, '▀', dim_color(color, fade));
            } else {
                break;
            }
        }

        // 下半（从中轴向底）
        let units = e * (rows_dn * 2) as f32;
        for r in 0..rows_dn {
            let y = mid + 1 + r;
            if y > bottom {
                break;
            }
            let fade = 1.0 - 0.45 * r as f32 / rows_dn.max(1) as f32;
            if units >= (2 * r + 2) as f32 {
                set_cell(f, x, y, '█', dim_color(color, fade));
            } else if units >= (2 * r + 1) as f32 {
                set_cell(f, x, y, '▄', dim_color(color, fade));
            } else {
                break;
            }
        }

        // 峰值下落点（只画在上半区，避开已有柱体）
        if pk > 0.0 {
            let pr = ((pk * rows_up as f32).round() as i32).min(rows_up);
            let y = mid - pr;
            if y >= top && y < mid {
                let sym = f.buffer_mut()[(x, y as u16)].symbol();
                if sym == " " || sym == "─" {
                    set_cell(f, x, y, '·', dim_color(color, 0.85));
                }
            }
        }
    }
}

/// 把频段序列重采样为 `cols` 列（线性插值，空序列补零）。
fn resample(src: &[f32], cols: usize) -> Vec<f32> {
    if src.is_empty() {
        return vec![0.0; cols];
    }
    if src.len() == cols {
        return src.to_vec();
    }
    (0..cols)
        .map(|i| {
            let p = i as f32 * (src.len() as f32 - 1.0) / (cols as f32 - 1.0).max(1.0);
            let lo = p.floor() as usize;
            let hi = (lo + 1).min(src.len() - 1);
            let t = p - lo as f32;
            src[lo] * (1.0 - t) + src[hi] * t
        })
        .collect()
}

fn set_cell(f: &mut Frame, x: u16, y: i32, ch: char, color: Color) {
    f.buffer_mut().set_string(
        x,
        y.max(0) as u16,
        ch.encode_utf8(&mut [0u8; 4]),
        Style::default().fg(color),
    );
}

fn spectrum_color(freq: f32) -> Color {
    if freq < 0.33 {
        blend(
            Color::Rgb(120, 220, 240),
            Color::Rgb(170, 140, 250),
            freq / 0.33,
        )
    } else if freq < 0.66 {
        blend(
            Color::Rgb(170, 140, 250),
            Color::Rgb(250, 130, 200),
            (freq - 0.33) / 0.33,
        )
    } else {
        blend(
            Color::Rgb(250, 130, 200),
            Color::Rgb(250, 210, 160),
            (freq - 0.66) / 0.34,
        )
    }
}

fn dim_color(c: Color, factor: f32) -> Color {
    let (r, g, b) = to_rgb(c);
    Color::Rgb(
        (r as f32 * factor) as u8,
        (g as f32 * factor) as u8,
        (b as f32 * factor) as u8,
    )
}

fn blend(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (ar, ag, ab) = to_rgb(a);
    let (br, bg, bb) = to_rgb(b);
    let r = (ar as f32 * (1.0 - t) + br as f32 * t) as u8;
    let g = (ag as f32 * (1.0 - t) + bg as f32 * t) as u8;
    let bch = (ab as f32 * (1.0 - t) + bb as f32 * t) as u8;
    Color::Rgb(r, g, bch)
}

fn to_rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Cyan => (0, 220, 220),
        Color::Blue => (60, 120, 230),
        Color::Magenta => (220, 80, 200),
        Color::Yellow => (240, 210, 90),
        Color::White => (230, 230, 230),
        _ => (150, 150, 150),
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) -> Option<Rect> {
    if area.height == 0 || area.width == 0 {
        return None;
    }
    let rep = match app.repeat {
        RepeatMode::Off => "Off",
        RepeatMode::One => "One",
        RepeatMode::All => "List",
    };
    let st = if app.player.playing {
        "Playing"
    } else {
        "Paused"
    };
    let shuf = if app.shuffle { "On" } else { "Off" };
    let pos = fmt_dur(app.player.position);

    let (cur_dur, name) = if let Some(o) = app.current_track {
        let tracks = app.active_tracks();
        if let Some(t) = tracks.get(o) {
            (t.duration, t.display_name())
        } else {
            (0.0, "None".to_string())
        }
    } else {
        (0.0, "None".to_string())
    };

    let pct = if cur_dur > 0.0 {
        app.player.position.as_secs_f64() / cur_dur
    } else {
        0.0
    };
    let pct = pct.clamp(0.0, 1.0);

    let inner_w = area.width.saturating_sub(2);
    let status = if inner_w >= 70 {
        Line::from(vec![
            Span::styled(" state ", Style::default().fg(Color::DarkGray)),
            Span::raw(": "),
            Span::styled(st.to_string(), Style::default().fg(Color::Green)),
            Span::raw("   repeat: "),
            Span::styled(rep.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw("   shuffle: "),
            Span::styled(shuf.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw(format!("   volume: {}%", (app.volume * 100.0) as i32)),
            Span::raw(format!("   pos: {} /", pos)),
            Span::raw(format!(" {}", fmt_secs(cur_dur))),
            Span::raw(format!("   now: {}", name)),
        ])
    } else if inner_w >= 45 {
        Line::from(vec![
            Span::styled(" state ", Style::default().fg(Color::DarkGray)),
            Span::raw(": "),
            Span::styled(st.to_string(), Style::default().fg(Color::Green)),
            Span::raw("   rpt: "),
            Span::styled(rep.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw("   shf: "),
            Span::styled(shuf.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw(format!("   vol:{}%", (app.volume * 100.0) as i32)),
            Span::raw(format!("   {}/", pos)),
            Span::raw(fmt_secs(cur_dur)),
            Span::raw(format!("   {}", name)),
        ])
    } else {
        Line::from(vec![
            Span::styled(st.to_string(), Style::default().fg(Color::Green)),
            Span::raw(" | rpt:"),
            Span::styled(rep.to_string(), Style::default().fg(Color::Cyan)),
            Span::raw(format!(" | {}/", pos)),
            Span::raw(fmt_secs(cur_dur)),
            Span::raw(format!(" | {}", name)),
        ])
    };

    let bar_w = area.width.saturating_sub(2) as usize;
    let filled = (pct * bar_w as f64) as usize;
    let progress = format!("[{}{}]", "=".repeat(filled), "-".repeat(bar_w - filled));
    let progress_line = Line::from(Span::styled(progress, Style::default().fg(Color::Magenta)));

    let help = Line::from(vec![
        Span::styled(
            " f/find  j/k move  enter play  space pause  n/p next/prev  r repeat  s shuffle  +/- vol  [/] playlist  q quit ",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let inner_h = area.height.saturating_sub(2);
    let mut lines = Vec::new();
    if let Some(err) = &app.error {
        if inner_h >= 1 {
            lines.push(Line::from(Span::styled(
                format!(" ERR: {}", err),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        }
    }
    if inner_h >= 1 + lines.len() as u16 {
        lines.push(status);
    }
    let mut progress_rect = None;
    if inner_h >= 2 {
        // 进度条所在行（回传给鼠标命中测试）；宽度 = inner 宽。
        progress_rect = Some(Rect::new(
            area.x + 1,
            area.y + 1 + lines.len() as u16,
            bar_w as u16,
            1,
        ));
        lines.push(progress_line);
    }
    if inner_h >= 3 {
        lines.push(help);
    }

    let block = Block::default().borders(Borders::ALL);
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
    progress_rect
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

fn fmt_secs(s: f64) -> String {
    let s = s as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Now Playing 全屏面板：大标题 + 同步歌词 + 镜像频谱 + 可点击进度条。
fn draw_np(f: &mut Frame, app: &App, area: Rect) -> LayoutInfo {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(Span::styled(
            " ♪ Now Playing ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut info = LayoutInfo {
        np: inner,
        ..Default::default()
    };
    if inner.width < 12 || inner.height < 10 {
        return info;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(4),    // 标题区
            Constraint::Length(7), // 歌词窗口
            Constraint::Length(8), // 频谱
            Constraint::Length(1), // 进度条
            Constraint::Length(1), // 提示行
        ])
        .split(inner);

    draw_np_title(f, app, chunks[0]);
    draw_lyrics(f, app, chunks[1]);
    draw_spectrum(f, app, chunks[2]);
    info.spectrum = chunks[2];
    info.progress = draw_np_progress(f, app, chunks[3]);

    let hint = Line::from(Span::styled(
        " i 返回列表 · 空格 播放/暂停 · n/p 切歌 · h/l 快退/快进 · 滚轮 音量 · 点击进度条 定位 ",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(hint).alignment(Alignment::Center), chunks[4]);
    info
}

fn draw_np_title(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let (title, artist, pl, idx, total, dur) = if let Some(o) = app.current_track {
        if let Some(t) = app.active_tracks().get(o) {
            (
                t.display_name(),
                t.subtitle(),
                app.playlists
                    .get(app.active_playlist)
                    .map(|p| p.name.clone())
                    .unwrap_or_default(),
                o + 1,
                app.active_tracks().len(),
                t.duration,
            )
        } else {
            (
                "未在播放".to_string(),
                String::new(),
                String::new(),
                0,
                0,
                0.0,
            )
        }
    } else {
        (
            "未在播放".to_string(),
            String::new(),
            String::new(),
            0,
            0,
            0.0,
        )
    };

    let mut lines = vec![
        Line::from(Span::styled("♪", Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(artist, Style::default().fg(Color::Magenta))),
    ];
    if area.height >= 5 {
        let state = if app.player.playing { "▶" } else { "⏸" };
        lines.push(Line::from(Span::styled(
            format!(
                "  {} · {} · 第 {}/{} 首 · {}  ",
                state,
                pl,
                idx,
                total,
                fmt_secs(dur)
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

/// 歌词窗口：当前行高亮居中，上下各带 2 行上下文，无歌词时显示占位符。
fn draw_lyrics(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    if let (Some(ls), Some(cur)) = (app.lyrics.as_ref(), app.current_lyric_idx()) {
        let start = cur.saturating_sub(2);
        let end = (cur + 2).min(ls.len() - 1);
        let n = end - start + 1;
        let pad = (area.height as usize).saturating_sub(n) / 2;
        for _ in 0..pad {
            lines.push(Line::from(""));
        }
        for i in start..=end {
            let style = if i == cur {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            lines.push(Line::from(Span::styled(ls[i].text.clone(), style)));
        }
    } else if app.lyrics.is_some() {
        // 有同步歌词但还没到第一句
        lines.push(Line::from(Span::styled(
            "…",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "· · ·",
            Style::default().fg(Color::Rgb(90, 90, 110)),
        )));
    }
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

/// Now Playing 大进度条：时间标签 + 比例条，回传可点击区域。
fn draw_np_progress(f: &mut Frame, app: &App, area: Rect) -> Option<Rect> {
    if area.width < 14 || area.height == 0 {
        return None;
    }
    let cur_dur = app
        .current_track
        .and_then(|o| app.active_tracks().get(o))
        .map(|t| t.duration)
        .unwrap_or(0.0);
    let pct = if cur_dur > 0.0 {
        (app.player.position.as_secs_f64() / cur_dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label_w = 5u16;
    let bar_w = area.width.saturating_sub(label_w * 2 + 2) as usize;
    let filled = ((pct * bar_w as f64) as usize).min(bar_w);
    let line = Line::from(vec![
        Span::styled(
            format!("{:>5}", fmt_dur(app.player.position)),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" "),
        Span::styled("━".repeat(filled), Style::default().fg(Color::Cyan)),
        Span::styled(
            "─".repeat(bar_w - filled),
            Style::default().fg(Color::Rgb(90, 90, 110)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<5}", fmt_secs(cur_dur)),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
    Some(Rect::new(area.x + label_w + 1, area.y, bar_w as u16, 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tui_music::library::{Playlist, Track};

    fn np_app() -> App {
        let mut app = App::new("/tmp".into()).unwrap();
        app.playlists = vec![Playlist {
            name: "歌单".into(),
            tracks: vec![Track {
                path: "/tmp/x.mp3".into(),
                title: "Song".into(),
                artist: "Artist".into(),
                duration: 200.0,
                lyrics: None,
            }],
        }];
        app.active_playlist = 0;
        app.display = vec![0];
        app.selected = Some(0);
        app.current_track = Some(0);
        app.show_np = true;
        app.smoothed = vec![0.5; 32];
        app.peaks = vec![0.7; 32];
        app.player.position = Duration::from_secs(100);
        app
    }

    #[test]
    fn np_panel_renders_filled_progress_and_spectrum() {
        let app = np_app();
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut li_slot = None;
        term.draw(|f| {
            li_slot = Some(super::draw(f, &app));
        })
        .unwrap();
        let li = li_slot.expect("layout info");
        let content: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol().to_string())
            .collect();
        // 进度条：一半位置应有填充块
        assert!(content.contains('━'), "progress bar must have filled cells");
        assert!(li.progress.is_some(), "progress rect must be reported");
        // 频谱：镜像块与峰值点必须出现
        assert!(content.contains('█'), "spectrum must have block cells");
        assert!(content.contains('·'), "spectrum must have peak dots");
        assert!(content.contains('▀') || content.contains('▄'));
        // 歌词占位符（无歌词）
        assert!(content.contains("· · ·"));
    }
}
