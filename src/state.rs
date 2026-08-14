//! 会话状态持久化：退出时保存音量 / 循环 / 随机 / 歌单与选中曲目，
//! 下次启动恢复（不自动播放）。存放于 ~/.config/tui-music/state.conf，
//! 使用简单 key=value 行格式，避免引入序列化依赖。

use std::path::{Path, PathBuf};

use crate::player::RepeatMode;

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub volume: f32,
    /// "off" | "one" | "all"
    pub repeat: String,
    pub shuffle: bool,
    /// 歌单名（如「歌曲架」）
    pub playlist: String,
    /// 选中曲目的文件路径
    pub track_path: String,
}

/// 状态文件默认路径：~/.config/tui-music/state.conf。
pub fn default_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(|h| PathBuf::from(h).join(r".config\tui-music\state.conf"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/tui-music/state.conf"))
    }
}

pub fn repeat_as_str(r: &RepeatMode) -> &'static str {
    match r {
        RepeatMode::Off => "off",
        RepeatMode::One => "one",
        RepeatMode::All => "all",
    }
}

pub fn repeat_from_str(s: &str) -> RepeatMode {
    match s {
        "off" => RepeatMode::Off,
        "one" => RepeatMode::One,
        _ => RepeatMode::All,
    }
}

impl AppState {
    fn to_lines(&self) -> String {
        format!(
            "volume={:.3}\nrepeat={}\nshuffle={}\nplaylist={}\ntrack={}\n",
            self.volume, self.repeat, self.shuffle, self.playlist, self.track_path
        )
    }

    /// 解析 key=value 行；任一必需键缺失返回 None（整份状态视为无效）。
    fn from_lines(s: &str) -> Option<Self> {
        let mut volume = None;
        let mut repeat = None;
        let mut shuffle = None;
        let mut playlist = None;
        let mut track = None;
        for line in s.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // split_once：值里允许出现 '='（如文件路径）
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k {
                "volume" => volume = v.parse::<f32>().ok(),
                "repeat" => repeat = Some(v.to_string()),
                "shuffle" => shuffle = v.parse::<bool>().ok(),
                "playlist" => playlist = Some(v.to_string()),
                "track" => track = Some(v.to_string()),
                _ => {}
            }
        }
        Some(AppState {
            volume: volume?,
            repeat: repeat?,
            shuffle: shuffle?,
            playlist: playlist?,
            track_path: track?,
        })
    }
}

/// 保存状态；自动创建父目录。
pub fn save(path: &Path, s: &AppState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, s.to_lines())
}

pub fn load(path: &Path) -> Option<AppState> {
    let text = std::fs::read_to_string(path).ok()?;
    AppState::from_lines(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AppState {
        AppState {
            volume: 0.65,
            repeat: "one".into(),
            shuffle: true,
            playlist: "歌曲架".into(),
            track_path: "/tmp/music/a=b 張三.mp3".into(),
        }
    }

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("tui_music_state_{}", std::process::id()));
        let path = dir.join("state.conf");
        let s = sample();
        save(&path, &s).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, s);
        // volume 序列化为三位小数，容差比较
        assert!((loaded.volume - 0.65).abs() < 1e-3);
    }

    #[test]
    fn missing_key_is_invalid() {
        let s = "volume=0.5\nshuffle=true";
        assert!(AppState::from_lines(s).is_none());
    }

    #[test]
    fn value_may_contain_equals() {
        let s = "volume=0.5\nrepeat=all\nshuffle=false\nplaylist=p\ntrack=/a/b=c.mp3\n";
        let st = AppState::from_lines(s).unwrap();
        assert_eq!(st.track_path, "/a/b=c.mp3");
    }

    #[test]
    fn garbage_lines_ignored() {
        let s = "junk\nvolume=0.5\nrepeat=off\nshuffle=false\nplaylist=p\ntrack=t\nnoise=1\n";
        let st = AppState::from_lines(s).unwrap();
        assert_eq!(st.repeat, "off");
    }
}
