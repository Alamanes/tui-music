//! LRC 歌词解析：优先读取与音频同目录的同名 `.lrc` 文件，其次使用
//! 标签内嵌歌词（USLT/LYRICS 常见为 LRC 文本），解析为按时间排序的
//! 行序列，供 Now Playing 面板同步滚动。
//! 无时间戳的纯文本歌词不视为同步歌词（返回 None）。

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    /// 距曲目开始的秒数
    pub time: f64,
    pub text: String,
}

/// 读取与音频同目录的同名 `.lrc` 文件并解析；文件不存在或不是
/// 有效同步歌词时返回 None。
pub fn load_lrc_file(track: &Path) -> Option<Vec<LyricLine>> {
    let lrc = track.with_extension("lrc");
    let text = std::fs::read_to_string(lrc).ok()?;
    parse_lrc(&text)
}

/// 解析 LRC 文本；没有任何 `[mm:ss.xx]` 时间戳时返回 None。
pub fn parse_lrc(raw: &str) -> Option<Vec<LyricLine>> {
    let mut out: Vec<LyricLine> = Vec::new();
    for line in raw.lines() {
        let mut rest = line.trim();
        if rest.is_empty() {
            continue;
        }
        // 一行可以带多个时间戳：[00:12.00][00:45.30]重叠的蓝天
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else {
                break;
            };
            let Some(t) = parse_time(&stripped[..close]) else {
                break;
            };
            times.push(t);
            rest = &stripped[close + 1..];
        }
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        for t in times {
            out.push(LyricLine {
                time: t,
                text: text.to_string(),
            });
        }
    }
    if out.is_empty() {
        return None;
    }
    out.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Some(out)
}

/// `mm:ss.xx` / `m:ss` → 秒；不是时间格式（如 `ti:标题`）返回 None。
fn parse_time(s: &str) -> Option<f64> {
    let (m, sec) = s.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let sec: f64 = sec.trim().parse().ok()?;
    if !(0.0..60.0).contains(&sec) {
        return None;
    }
    Some(m * 60.0 + sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synced_lrc() {
        let raw = "[ti:测试]\n[00:01.50]第一句\n[01:02.00]第二句\n";
        let ls = parse_lrc(raw).unwrap();
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].time, 1.5);
        assert_eq!(ls[0].text, "第一句");
        assert_eq!(ls[1].time, 62.0);
    }

    #[test]
    fn multiple_timestamps_expand() {
        let raw = "[00:10.00][00:40.00]副歌\n";
        let ls = parse_lrc(raw).unwrap();
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].time, 10.0);
        assert_eq!(ls[1].time, 40.0);
        assert_eq!(ls[0].text, ls[1].text);
    }

    #[test]
    fn unsynced_returns_none() {
        assert!(parse_lrc("没有时间戳的歌词\n第二行\n").is_none());
        assert!(parse_lrc("").is_none());
    }

    #[test]
    fn metadata_tags_are_ignored() {
        let raw = "[ar:歌手]\n[00:05.00]正文\n";
        let ls = parse_lrc(raw).unwrap();
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].text, "正文");
    }

    #[test]
    fn lrc_file_sibling_loading() {
        let dir = std::env::temp_dir().join(format!("tui_music_lrc_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("歌 手.mp3");
        // 无同名 .lrc → None
        assert!(load_lrc_file(&audio).is_none());
        // 写入有效同步歌词 → Some
        std::fs::write(
            dir.join("歌 手.lrc"),
            "[00:01.00]第一句\n[00:02.00]第二句\n",
        )
        .unwrap();
        let ls = load_lrc_file(&audio).unwrap();
        assert_eq!(ls.len(), 2);
        assert_eq!(ls[0].time, 1.0);
        assert_eq!(ls[1].text, "第二句");
        // 无效（无时间戳）→ None
        std::fs::write(dir.join("歌 手.lrc"), "没有时间戳\n").unwrap();
        assert!(load_lrc_file(&audio).is_none());
    }
}
