mod app;
mod media;
mod ui;

use std::io;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

#[derive(Parser)]
#[command(name = "tui-music", version, about = "A terminal music player")]
struct Args {
    /// Path to music directory (default: ~/Music)
    #[arg(short, long)]
    music: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let music_dir = args
        .music
        .unwrap_or_else(tui_music::library::default_music_dir);

    // macOS 系统媒体集成需要 com.apple.* 代码签名标识；未签名时提前提示。
    if !media::system_integration_available() {
        eprintln!(
            "警告：macOS 系统媒体集成未启用（控制中心/媒体键不可用）。\n  \
             请先签名二进制：codesign --force -s - --identifier com.apple.tui-music <tui-music 可执行文件路径>"
        );
    }

    let mut app = app::App::new(music_dir)?;
    if !app.playlists.is_empty() && !app.active_tracks().is_empty() {
        app.selected = Some(0);
    }

    let result = with_terminal(|terminal| run(terminal, &mut app));
    // 退出时保存会话状态（音量/循环/随机/歌单/选中曲目）。
    if let Some(p) = tui_music::state::default_path() {
        let _ = tui_music::state::save(&p, &app.snapshot_state());
    }
    result
}

/// Enter raw mode + alternate screen for the duration of `f`, and always
/// restore the terminal afterwards — even if `f` errors or the app panics.
fn with_terminal<F>(f: F) -> anyhow::Result<()>
where
    F: FnOnce(&mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()>,
{
    enable_raw_mode()?;
    let result = (|| {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let r = f(&mut terminal);
        let _ = terminal.show_cursor();
        let _ = execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        r
    })();
    let _ = disable_raw_mode();
    result
}

fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut app::App,
) -> anyhow::Result<()> {
    let tick = Duration::from_millis(33);
    let mut media = media::Bridge::new();
    loop {
        // 每帧回传可交互区域，供鼠标事件命中测试（draw 回调无法直接返回值，走侧信道）。
        let mut li_slot: Option<ui::LayoutInfo> = None;
        terminal.draw(|f| {
            li_slot = Some(ui::draw(f, app));
        })?;
        let li = li_slot.unwrap_or_default();

        // AirPods / 硬件媒体键命令（来自 MediaRemote，跑在独立线程上）。
        while let Some(cmd) = media.poll() {
            handle_remote(app, cmd);
        }
        // 服务主 run loop：MediaRemote 的命令回调依赖它，每帧泵一次（无事件时立即返回）。
        media.pump();

        if event::poll(tick)? {
            while event::poll(Duration::ZERO).unwrap_or(false) {
                match event::read()? {
                    Event::Key(k) => {
                        if k.kind != KeyEventKind::Release {
                            app.handle_key(k);
                        }
                    }
                    Event::Mouse(m) => app.handle_mouse(m, &li),
                    _ => {}
                }
            }
        }

        app.update(terminal.size()?.width.saturating_sub(2).max(16) as usize);

        sync_media(&mut media, app);

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// 把一条远程媒体命令翻译成应用动作。
fn handle_remote(app: &mut app::App, cmd: media::Command) {
    use media::Command as C;
    match cmd {
        C::Play => {
            if app.player.playing {
                return;
            }
            if app.current_track.is_some() {
                if app.player.is_finished() {
                    app.next();
                } else {
                    app.player.resume();
                }
            } else {
                app.play_selected();
            }
        }
        C::Pause => {
            if app.player.playing {
                app.player.pause();
            }
        }
        C::TogglePlayPause => app.toggle_pause(),
        C::Stop => {
            if app.player.playing {
                app.player.pause();
            }
        }
        C::NextTrack => app.next(),
        C::PreviousTrack => app.prev(),
    }
}

/// 把当前播放状态同步给系统「正在播放」。无曲目时发布一个「停止」状态，
/// 让系统卡片不再残留上一首（Bridge 内部有 500ms 节流，稳态只发一次）。
fn sync_media(media: &mut media::Bridge, app: &app::App) {
    let Some(o) = app.current_track else {
        media.sync(false, "", "", 0.0, 0.0);
        return;
    };
    let Some(t) = app.active_tracks().get(o) else {
        media.sync(false, "", "", 0.0, 0.0);
        return;
    };
    media.sync(
        app.player.playing,
        &t.display_name(),
        &t.artist,
        t.duration,
        app.player.position.as_secs_f64(),
    );
}
