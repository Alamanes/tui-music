use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use tui_music::library::{scan, Playlist, Track};
use tui_music::lyrics;
use tui_music::player::{Player, RepeatMode};

use crate::ui::LayoutInfo;

pub struct App {
    pub music_dir: PathBuf,
    pub playlists: Vec<Playlist>,
    pub active_playlist: usize,
    pub display: Vec<usize>,
    pub selected: Option<usize>,
    /// Track identity: index into `active_tracks()` (survives search re-filters).
    pub current_track: Option<usize>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub volume: f32,
    pub player: Player,
    pub smoothed: Vec<f32>,
    pub should_quit: bool,
    pub search: String,
    pub search_active: bool,
    /// 频谱峰值包络（与 smoothed 同长），每帧固定回落，用于下落点绘制。
    pub peaks: Vec<f32>,
    /// Now Playing 全屏面板开关（`i` 切换）。
    pub show_np: bool,
    /// 当前曲目的同步歌词（LRC 解析结果），无则 None。
    pub lyrics: Option<Vec<lyrics::LyricLine>>,
    /// Last non-fatal error, shown in the footer.
    pub error: Option<String>,
    rng_state: u64,
    shuffle_queue: Vec<usize>,
}

impl App {
    pub fn new(music_dir: PathBuf) -> anyhow::Result<Self> {
        let playlists = if music_dir.exists() {
            scan(&music_dir)
        } else {
            Vec::new()
        };
        let active = playlists
            .iter()
            .position(|p| !p.tracks.is_empty())
            .unwrap_or(0);
        let display = playlists
            .get(active)
            .map(|pl| (0..pl.tracks.len()).collect())
            .unwrap_or_default();
        let mut app = Self {
            music_dir,
            playlists,
            active_playlist: active,
            display,
            selected: None,
            current_track: None,
            repeat: RepeatMode::All,
            shuffle: false,
            volume: 0.6,
            player: Player::new()?,
            smoothed: Vec::new(),
            should_quit: false,
            search: String::new(),
            search_active: false,
            peaks: Vec::new(),
            show_np: false,
            lyrics: None,
            error: None,
            rng_state: seed_rng(),
            shuffle_queue: Vec::new(),
        };
        app.restore_state();
        Ok(app)
    }

    /// 从默认路径恢复会话状态（音量/循环/随机/歌单/选中曲目），不自动播放。
    fn restore_state(&mut self) {
        let Some(path) = tui_music::state::default_path() else {
            return;
        };
        let Some(s) = tui_music::state::load(&path) else {
            return;
        };
        self.volume = s.volume.clamp(0.0, 1.0);
        self.player.set_volume(self.volume);
        self.repeat = tui_music::state::repeat_from_str(&s.repeat);
        self.shuffle = s.shuffle;
        if let Some(i) = self.playlists.iter().position(|p| p.name == s.playlist) {
            self.active_playlist = i;
            self.rebuild_display();
            if let Some(ti) = self
                .active_tracks()
                .iter()
                .position(|t| t.path.to_string_lossy() == s.track_path)
            {
                if let Some(v) = self.display.iter().position(|&o| o == ti) {
                    self.selected = Some(v);
                }
            }
        }
    }

    /// 导出当前会话状态用于持久化。
    pub fn snapshot_state(&self) -> tui_music::state::AppState {
        let playlist = self
            .playlists
            .get(self.active_playlist)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let track_path = self
            .selected
            .and_then(|v| self.display.get(v).copied())
            .and_then(|o| self.active_tracks().get(o))
            .map(|t| t.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        tui_music::state::AppState {
            volume: self.volume,
            repeat: tui_music::state::repeat_as_str(&self.repeat).to_string(),
            shuffle: self.shuffle,
            playlist,
            track_path,
        }
    }

    pub fn active_tracks(&self) -> &[Track] {
        self.playlists
            .get(self.active_playlist)
            .map(|p| p.tracks.as_slice())
            .unwrap_or(&[])
    }

    pub fn switch_playlist(&mut self, delta: i32) {
        let n = self.playlists.len();
        if n <= 1 {
            return;
        }
        let cur = self.active_playlist as i32;
        let mut next = cur + delta;
        while next < 0 {
            next += n as i32;
        }
        let next = (next as usize) % n;
        self.active_playlist = next;
        self.player.stop();
        self.current_track = None;
        self.shuffle_queue.clear();
        self.search.clear();
        self.error = None;
        self.rebuild_display();
    }

    fn rebuild_display(&mut self) {
        let tracks = self.active_tracks();
        if self.search.is_empty() {
            self.display = (0..tracks.len()).collect();
        } else {
            let q = self.search.to_lowercase();
            self.display = tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    t.display_name().to_lowercase().contains(&q)
                        || t.subtitle().to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        let n = self.display.len();
        if n == 0 {
            self.selected = None;
        } else if self.selected.map_or(true, |s| s >= n) {
            self.selected = Some(0);
        }
    }

    pub fn enter_search(&mut self) {
        self.search_active = true;
    }

    pub fn exit_search(&mut self, keep: bool) {
        self.search_active = false;
        if !keep {
            self.search.clear();
            self.rebuild_display();
        }
    }

    pub fn search_push(&mut self, c: char) {
        self.search.push(c);
        self.rebuild_display();
    }

    pub fn search_pop(&mut self) {
        self.search.pop();
        self.rebuild_display();
    }

    fn orig_of(&self, view_i: usize) -> Option<usize> {
        self.display.get(view_i).copied()
    }

    pub fn selected_mut(&mut self, delta: i32) {
        let n = self.display.len();
        if n == 0 {
            return;
        }
        let cur = self
            .selected
            .unwrap_or_else(|| if delta > 0 { 0 } else { n - 1 });
        let mut next = cur as i32 + delta;
        while next < 0 {
            next += n as i32;
        }
        let next = (next as usize) % n;
        self.selected = Some(next);
    }

    pub fn play_selected(&mut self) {
        if let Some(v) = self.selected {
            if let Some(o) = self.orig_of(v) {
                self.play_track(o);
            }
        }
    }

    pub fn play_track(&mut self, playlist_track_i: usize) {
        let tracks = self.active_tracks();
        if playlist_track_i >= tracks.len() {
            return;
        }
        let track = &tracks[playlist_track_i];
        let path = track.path.clone();
        let raw_lyrics = track.lyrics.clone();
        if let Err(e) = self.player.play_file(&path) {
            self.error = Some(format!("cannot play: {}", e));
            return;
        }
        self.player.set_volume(self.volume);
        self.current_track = Some(playlist_track_i);
        self.lyrics = raw_lyrics.as_deref().and_then(lyrics::parse_lrc);
        self.error = None;
    }

    /// Display index of the current track, if it's visible in the current filter.
    fn current_view(&self) -> Option<usize> {
        let ct = self.current_track?;
        self.display.iter().position(|&o| o == ct)
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64* — fast, tiny, uniform.
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng_state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn reset_shuffle(&mut self) {
        let n = self.display.len();
        let mut order: Vec<usize> = (0..n).collect();
        let mut i = n;
        while i > 1 {
            i -= 1;
            // multiply-high: uniform in [0, i+1) without modulo bias.
            let j = ((self.next_rand() >> 32) as usize).wrapping_mul(i + 1) >> 32;
            order.swap(i, j);
        }
        // Don't let shuffle start with the currently playing track.
        if let Some(cur) = self.current_view() {
            if order.first() == Some(&cur) && order.len() > 1 {
                order.rotate_left(1);
            }
        }
        self.shuffle_queue = order;
    }

    pub fn next(&mut self) {
        let n = self.display.len();
        if n == 0 {
            return;
        }
        let next_v = if self.shuffle {
            if self.shuffle_queue.len() != n {
                self.reset_shuffle();
            }
            if let Some(r) = self.shuffle_queue.first().copied() {
                self.shuffle_queue.remove(0);
                r
            } else {
                self.current_view().unwrap_or(0)
            }
        } else {
            let cur = self.current_view().unwrap_or(0);
            (cur + 1) % n
        };
        if let Some(o) = self.orig_of(next_v) {
            self.play_track(o);
        }
    }

    pub fn prev(&mut self) {
        let n = self.display.len();
        if n == 0 {
            return;
        }
        let cur = self.current_view().unwrap_or(0);
        let prev_v = if cur == 0 { n - 1 } else { cur - 1 };
        if let Some(o) = self.orig_of(prev_v) {
            self.play_track(o);
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.player.playing {
            self.player.pause();
        } else if let Some(o) = self.current_track {
            if self.player.is_finished() {
                // 自然播完后按空格：重播当前曲目而非跳下一首。
                // 循环 All/One 时 update() 已自动切歌，这里通常只在 repeat Off 到达。
                self.play_track(o);
            } else {
                self.player.resume();
            }
        }
    }

    pub fn cycle_repeat(&mut self) {
        match self.repeat {
            RepeatMode::Off => self.repeat = RepeatMode::All,
            RepeatMode::All => self.repeat = RepeatMode::One,
            RepeatMode::One => self.repeat = RepeatMode::Off,
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.shuffle_queue.clear();
    }

    pub fn change_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
        self.player.set_volume(self.volume);
    }

    /// ±delta 秒快进/快退；无曲目、无时长或已播完时忽略，越界夹紧。
    pub fn seek(&mut self, delta: f64) {
        let Some(o) = self.current_track else { return };
        let Some(t) = self.active_tracks().get(o) else {
            return;
        };
        if t.duration <= 0.0 || self.player.is_finished() {
            return;
        }
        let cur = self.player.position.as_secs_f64();
        let target = (cur + delta).clamp(0.0, (t.duration - 0.2).max(0.0));
        match self.player.seek(Duration::from_secs_f64(target)) {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(format!("seek failed: {e}")),
        }
    }

    /// 定位到绝对秒数（进度条点击用），越界夹紧。
    pub fn seek_absolute(&mut self, sec: f64) {
        let Some(o) = self.current_track else { return };
        let Some(t) = self.active_tracks().get(o) else {
            return;
        };
        if t.duration <= 0.0 {
            return;
        }
        let target = sec.clamp(0.0, (t.duration - 0.2).max(0.0));
        if let Err(e) = self.player.seek(Duration::from_secs_f64(target)) {
            self.error = Some(format!("seek failed: {e}"));
        } else {
            self.error = None;
        }
    }

    /// 当前播放位置对应的歌词行下标（最后一句 time <= pos）。
    pub fn current_lyric_idx(&self) -> Option<usize> {
        let ls = self.lyrics.as_ref()?;
        let pos = self.player.position.as_secs_f64();
        let mut idx = None;
        for (i, l) in ls.iter().enumerate() {
            if l.time <= pos + 0.05 {
                idx = Some(i);
            } else {
                break;
            }
        }
        idx
    }

    /// 鼠标交互：列表区点击播放 / 滚轮移动选择；进度条点击定位；
    /// 其余区域（头部 / 频谱 / 状态栏 / Now Playing）滚轮调节音量。
    pub fn handle_mouse(&mut self, m: MouseEvent, li: &LayoutInfo) {
        let (x, y) = (m.column as i32, m.row as i32);
        let contains = |r: Rect| {
            r.width > 0
                && r.height > 0
                && x >= r.x as i32
                && x < (r.x + r.width) as i32
                && y >= r.y as i32
                && y < (r.y + r.height) as i32
        };
        match m.kind {
            MouseEventKind::ScrollUp => {
                if contains(li.list) {
                    self.selected_mut(-1);
                } else if contains(li.np)
                    || contains(li.header)
                    || contains(li.spectrum)
                    || contains(li.footer)
                {
                    self.change_volume(0.05);
                }
            }
            MouseEventKind::ScrollDown => {
                if contains(li.list) {
                    self.selected_mut(1);
                } else if contains(li.np)
                    || contains(li.header)
                    || contains(li.spectrum)
                    || contains(li.footer)
                {
                    self.change_volume(-0.05);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // 进度条：按横向比例定位
                if let Some(pr) = li.progress {
                    if contains(pr) {
                        let frac = (x - pr.x as i32) as f64 / pr.width.max(1) as f64;
                        if let Some(o) = self.current_track {
                            if let Some(t) = self.active_tracks().get(o) {
                                if t.duration > 0.0 {
                                    self.seek_absolute(frac * t.duration);
                                }
                            }
                        }
                        return;
                    }
                }
                // 列表：点击行播放对应曲目
                if contains(li.list) && li.list.height > 2 {
                    let row = y - li.list.y as i32 - 1;
                    if row < 0 {
                        return;
                    }
                    let idx = li.list_offset + row as usize;
                    if let Some(o) = self.display.get(idx).copied() {
                        self.selected = Some(idx);
                        self.play_track(o);
                    }
                }
            }
            _ => {}
        }
    }

    fn smooth(&mut self, raw: Vec<f32>) {
        if self.smoothed.len() != raw.len() {
            self.smoothed = raw.clone();
            return;
        }
        const ATTACK: f32 = 0.7;
        const RELEASE: f32 = 0.25;
        for (i, &r) in raw.iter().enumerate() {
            let s = self.smoothed[i];
            let coeff = if r > s { ATTACK } else { RELEASE };
            self.smoothed[i] = s + (r - s) * coeff;
        }
    }

    pub fn update(&mut self, num_bars: usize) {
        self.player.update_position();

        if let Some(o) = self.current_track {
            let tracks = self.active_tracks();
            let d = tracks[o].duration;
            let mut finished = self.player.is_finished();
            if d > 0.0 && self.player.position.as_secs_f64() >= d && self.player.is_finished() {
                finished = true;
            }
            if finished && self.player.playing {
                self.player.playing = false;
                match self.repeat {
                    RepeatMode::One => {
                        self.play_track(o);
                    }
                    RepeatMode::All => {
                        self.next();
                    }
                    RepeatMode::Off => {
                        // repeat Off：仅当当前曲目之后还有曲目时才前进；
                        // 当前曲目被搜索过滤隐藏或已是最后一首时停在原地。
                        if self
                            .current_view()
                            .is_some_and(|v| v + 1 < self.display.len())
                        {
                            self.next();
                        }
                    }
                }
            }
        }

        let samples = self.player.viz.latest(tui_music::viz::FFT_N);
        let raw = tui_music::viz::compute(&samples, num_bars.max(16));
        self.smooth(raw);

        // 峰值包络：固定速度回落，被当前值顶起时不落后于实际电平。
        const PEAK_FALL: f32 = 0.015;
        if self.peaks.len() != self.smoothed.len() {
            self.peaks = self.smoothed.clone();
        } else {
            for (p, &s) in self.peaks.iter_mut().zip(self.smoothed.iter()) {
                *p = (*p - PEAK_FALL).max(s).max(0.0);
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.search_active {
            // 忽略 Ctrl/Alt 组合键（如 Ctrl+C），避免把控制字符敲进搜索框。
            if key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return;
            }
            match key.code {
                KeyCode::Esc => self.exit_search(false),
                KeyCode::Enter => self.exit_search(true),
                KeyCode::Backspace => self.search_pop(),
                KeyCode::Up => self.selected_mut(-1),
                KeyCode::Down => self.selected_mut(1),
                KeyCode::Char(c) => self.search_push(c),
                _ => {}
            }
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (ctrl, key.code) {
            (false, KeyCode::Char('q') | KeyCode::Esc) => self.should_quit = true,
            (false, KeyCode::Char('/')) | (false, KeyCode::Char('f')) => self.enter_search(),
            (false, KeyCode::Char('[')) => self.switch_playlist(-1),
            (false, KeyCode::Char(']')) => self.switch_playlist(1),
            (false, KeyCode::Up | KeyCode::Char('k')) => self.selected_mut(-1),
            (false, KeyCode::Down | KeyCode::Char('j')) => self.selected_mut(1),
            (false, KeyCode::Enter) => self.play_selected(),
            (false, KeyCode::Left | KeyCode::Char('h')) => self.seek(-5.0),
            (false, KeyCode::Right | KeyCode::Char('l')) => self.seek(5.0),
            (false, KeyCode::Char(' ')) => self.toggle_pause(),
            (false, KeyCode::Char('n')) | (false, KeyCode::Tab) => self.next(),
            (false, KeyCode::Char('p')) | (false, KeyCode::BackTab) => self.prev(),
            (false, KeyCode::Char('r')) => self.cycle_repeat(),
            (false, KeyCode::Char('s')) => self.toggle_shuffle(),
            (false, KeyCode::Char('i')) => {
                self.show_np = !self.show_np;
            }
            (false, KeyCode::Char('=')) | (false, KeyCode::Char('+')) => self.change_volume(0.05),
            (false, KeyCode::Char('-')) => self.change_volume(-0.05),
            _ => {}
        }
    }
}

fn seed_rng() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let pid = std::process::id() as u64;
    nanos ^ pid.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tui_music::library::{Playlist, Track};

    /// Write a tiny valid mono 16-bit PCM WAV file (0.5s of a sine at 8kHz).
    fn write_wav(path: &Path, freq: f32) {
        let rate = 8000u32;
        let n = (rate / 2) as usize; // 0.5s
        let mut data = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = (i as f32 / rate as f32 * freq * 2.0 * std::f32::consts::PI).sin();
            let v = (s * i16::MAX as f32) as i16;
            data.extend_from_slice(&v.to_le_bytes());
        }
        let data_len = data.len() as u32;
        let mut f = Vec::new();
        f.extend_from_slice(b"RIFF");
        f.extend_from_slice(&(36 + data_len).to_le_bytes());
        f.extend_from_slice(b"WAVE");
        f.extend_from_slice(b"fmt ");
        f.extend_from_slice(&16u32.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes()); // PCM
        f.extend_from_slice(&1u16.to_le_bytes()); // mono
        f.extend_from_slice(&rate.to_le_bytes());
        f.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        f.extend_from_slice(&2u16.to_le_bytes()); // block align
        f.extend_from_slice(&16u16.to_le_bytes()); // bits
        f.extend_from_slice(b"data");
        f.extend_from_slice(&data_len.to_le_bytes());
        f.extend_from_slice(&data);
        fs::write(path, f).unwrap();
    }

    fn playlist(dir: &Path, n: usize) -> Playlist {
        Playlist {
            name: "test".into(),
            tracks: (0..n)
                .map(|i| {
                    let p = dir.join(format!("t{}.wav", i));
                    write_wav(&p, 440.0 + i as f32 * 100.0);
                    Track {
                        path: p,
                        title: format!("t{}", i),
                        artist: format!("a{}", i),
                        duration: 0.5,
                        lyrics: None,
                    }
                })
                .collect(),
        }
    }

    fn app_with(n: usize) -> App {
        let dir = std::env::temp_dir().join(format!("tui_music_test_{}_{}", std::process::id(), n));
        fs::create_dir_all(&dir).unwrap();
        let mut app = App {
            music_dir: dir.clone(),
            playlists: vec![playlist(&dir, n)],
            active_playlist: 0,
            display: (0..n).collect(),
            selected: None,
            current_track: None,
            repeat: RepeatMode::All,
            shuffle: false,
            volume: 0.6,
            player: Player::new().unwrap(),
            smoothed: Vec::new(),
            should_quit: false,
            search: String::new(),
            search_active: false,
            peaks: Vec::new(),
            show_np: false,
            lyrics: None,
            error: None,
            rng_state: 1,
            shuffle_queue: Vec::new(),
        };
        if n > 0 {
            app.selected = Some(0);
        }
        app
    }

    #[test]
    fn display_index_survives_search_refilter() {
        let mut app = app_with(4);
        app.play_selected(); // plays track 0
        assert_eq!(app.current_track, Some(0));

        // Type a search that hides the current track.
        app.enter_search();
        app.search_push('3'); // only track 3 matches
        assert_eq!(app.display, vec![3]);
        assert_eq!(app.current_view(), None);
        // current_track identity is preserved even though it's not visible.
        assert_eq!(app.current_track, Some(0));

        // Exiting search keeps the filter; identity is preserved even when hidden.
        app.exit_search(true);
        assert_eq!(app.current_track, Some(0));
        assert_eq!(app.current_view(), None);

        // Clearing the filter restores the correct view mapping.
        app.exit_search(false);
        assert_eq!(app.current_view(), Some(0));
    }

    #[test]
    fn next_wraps_within_filtered_list() {
        let mut app = app_with(5);
        app.play_selected();
        app.next();
        assert_eq!(app.current_track, Some(1));
        app.next();
        assert_eq!(app.current_track, Some(2));
    }

    #[test]
    fn shuffle_uses_full_permutation_no_mod_bias() {
        let mut app = app_with(100);
        app.shuffle = true;
        app.reset_shuffle();
        let mut q = app.shuffle_queue.clone();
        q.sort_unstable();
        assert_eq!(q, (0..100).collect::<Vec<_>>(), "must be a permutation");
    }

    #[test]
    fn shuffle_skips_current_as_first() {
        let mut app = app_with(10);
        app.selected = Some(4);
        app.play_selected();
        assert_eq!(app.current_track, Some(4));
        app.shuffle = true;
        app.next();
        let got = app.current_track.unwrap();
        assert_ne!(
            got, 4,
            "shuffle must not immediately repeat the current track"
        );
    }

    #[test]
    fn corrupt_track_is_not_fatal() {
        let mut app = app_with(2);
        app.playlists[0].tracks[0].path = PathBuf::from("/tmp/definitely_missing.mp3");
        app.selected = Some(0);
        app.play_selected();
        assert!(app.error.is_some(), "error should be surfaced, not panic");
        assert_eq!(app.current_track, None);
        app.should_quit = false; // still alive
    }

    #[test]
    fn peaks_fall_and_stick() {
        let mut app = app_with(1);
        app.smoothed = vec![0.3; 32];
        app.peaks = vec![0.9; 32];
        app.update(32);
        // 空闲时 viz 为零：smoothed 向 0 衰减，峰值固定回落 0.015 后贴住 smoothed。
        // smoothed: 0.3 + (0 - 0.3) * 0.25 = 0.225；peak: max(0.9 - 0.015, 0.225) = 0.885
        assert!(app.peaks.iter().all(|&p| (p - 0.885).abs() < 1e-4));
    }

    #[test]
    fn mouse_click_in_list_plays_track() {
        let mut app = app_with(3);
        let li = LayoutInfo {
            list: Rect::new(0, 5, 40, 10),
            ..Default::default()
        };
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 6,
                modifiers: KeyModifiers::NONE,
            },
            &li,
        );
        assert_eq!(app.selected, Some(0));
        assert_eq!(app.current_track, Some(0));
        // 下一行点击应播放第二首
        app.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 7,
                modifiers: KeyModifiers::NONE,
            },
            &li,
        );
        assert_eq!(app.current_track, Some(1));
    }

    #[test]
    fn np_toggle_and_lyric_lookup() {
        let mut app = app_with(2);
        assert!(!app.show_np);
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert!(app.show_np);
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        assert!(!app.show_np);

        // 无歌词时 current_lyric_idx 为 None
        app.play_selected();
        assert!(app.current_lyric_idx().is_none());
    }

    #[test]
    fn search_ignores_ctrl_and_alt_combos() {
        let mut app = app_with(2);
        app.enter_search();
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::ALT,
        ));
        assert!(app.search.is_empty(), "组合键不得进入搜索框");
        app.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        ));
        assert_eq!(app.search, "a");
    }

    #[test]
    fn space_after_end_replays_current_track() {
        let mut app = app_with(3);
        app.selected = Some(2);
        app.play_selected();
        assert_eq!(app.current_track, Some(2));
        app.player.stop(); // sink 清空 → is_finished，模拟自然播完
        app.toggle_pause(); // 空格应重播当前曲目而非跳下一首
        assert_eq!(app.current_track, Some(2));
        assert!(app.player.playing);
    }
}
