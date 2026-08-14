use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::time::Duration;

use crate::app::App;
use tui_music::player::RepeatMode;

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(layout_constraints(f.area().height))
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_search(f, app, chunks[1]);
    draw_list(f, app, chunks[2]);
    draw_spectrum(f, app, chunks[3]);
    draw_footer(f, app, chunks[4]);
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

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    if area.height < 3 || area.width == 0 {
        return;
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
}

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
    let max_h = inner.height as i32;

    let src = &app.smoothed;
    let v: Vec<f32> = if src.is_empty() {
        vec![0.0; cols]
    } else if src.len() == cols {
        src.clone()
    } else {
        (0..cols)
            .map(|i| {
                let p = i as f32 * (src.len() as f32 - 1.0) / (cols as f32 - 1.0).max(1.0);
                let lo = p.floor() as usize;
                let hi = (lo + 1).min(src.len() - 1);
                let t = p - lo as f32;
                src[lo] * (1.0 - t) + src[hi] * t
            })
            .collect()
    };

    let bottom = inner.bottom() - 1;
    for x in inner.x..inner.x + inner.width {
        let cell = &mut f.buffer_mut()[(x, bottom)];
        cell.set_char('─');
        cell.set_style(Style::default().fg(Color::Rgb(60, 62, 80)));
    }

    let bar_thresh = 1.0 / max_h as f32;

    for i in 0..cols {
        let energy = v[i].clamp(0.0, 1.0);
        if energy < bar_thresh {
            continue;
        }
        let h = energy * max_h as f32;
        let full = h.floor() as i32;
        let frac = h - full as f32;

        let freq = i as f32 / (cols as f32 - 1.0).max(1.0);
        let color = spectrum_color(freq);

        let x = inner.x + i as u16;
        let top = inner.y as i32;
        let base = bottom as i32 - 1;

        let mut drawn = 0i32;
        while drawn < full {
            let y = base - drawn;
            if y < top {
                break;
            }
            put_dot(f, x, y as u16, color, false);
            drawn += 1;
        }
        if frac > 0.0 && full >= 1 {
            let y = base - drawn;
            if y >= top {
                put_dot(f, x, y as u16, color, true);
            }
        }
    }
}

fn put_dot(f: &mut Frame, x: u16, y: u16, color: Color, half: bool) {
    let ch = if half { '∙' } else { '·' };
    let style = if half {
        Style::default().fg(dim_color(color, 0.6))
    } else {
        Style::default().fg(color)
    };
    f.buffer_mut()
        .set_string(x, y, ch.encode_utf8(&mut [0u8; 4]), style);
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

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
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
    if inner_h >= 2 {
        lines.push(progress_line);
    }
    if inner_h >= 3 {
        lines.push(help);
    }

    let block = Block::default().borders(Borders::ALL);
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

fn fmt_secs(s: f64) -> String {
    let s = s as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
