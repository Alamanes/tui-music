use lofty::file::TaggedFileExt;
use lofty::prelude::{Accessor, AudioFile};
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub duration: f64,
    /// 原始歌词文本（通常为 LRC 格式），无标签时为 None。
    pub lyrics: Option<String>,
}

#[derive(Clone)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<Track>,
}

impl Track {
    pub fn display_name(&self) -> String {
        if self.title.is_empty() {
            self.path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "?".to_string())
        } else {
            self.title.clone()
        }
    }

    pub fn subtitle(&self) -> String {
        if self.artist.is_empty() {
            self.path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            self.artist.clone()
        }
    }
}

const EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "vorbis", "m4a", "aac", "opus", "aiff", "aif", "alac",
];

pub fn is_audio(p: &Path) -> bool {
    let e = match p.extension() {
        Some(e) => e.to_string_lossy().to_lowercase(),
        None => return false,
    };
    EXTENSIONS.contains(&e.as_str())
}

const PLAYLIST_MARKER: &str = "歌单";
const DEFAULT_PLAYLIST: &str = "歌曲架";

fn playlist_name(dir_name: &str) -> String {
    let stripped = dir_name.strip_suffix(PLAYLIST_MARKER).unwrap_or(dir_name);
    let stripped = stripped.trim();
    if stripped.is_empty() {
        dir_name.to_string()
    } else {
        stripped.to_string()
    }
}

pub fn scan(root: &Path) -> Vec<Playlist> {
    let mut playlist_dirs: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if name.contains(PLAYLIST_MARKER) {
                playlist_dirs.push((playlist_name(&name), p));
            }
        }
    }
    playlist_dirs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut playlist_tracks: Vec<Vec<Track>> = vec![Vec::new(); playlist_dirs.len()];
    let mut shelf_tracks: Vec<Track> = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if !p.is_file() || !is_audio(p) {
            continue;
        }

        let mut assigned = false;
        for (i, (_, dir)) in playlist_dirs.iter().enumerate() {
            if p.starts_with(dir) {
                let track = load_or_fallback(p);
                playlist_tracks[i].push(track);
                assigned = true;
                break;
            }
        }
        if !assigned {
            let track = load_or_fallback(p);
            shelf_tracks.push(track);
        }
    }

    for tracks in playlist_tracks.iter_mut() {
        tracks.sort_by(|a, b| {
            a.display_name()
                .to_lowercase()
                .cmp(&b.display_name().to_lowercase())
        });
    }
    shelf_tracks.sort_by(|a, b| {
        a.display_name()
            .to_lowercase()
            .cmp(&b.display_name().to_lowercase())
    });

    let shelf_empty = shelf_tracks.is_empty();

    let mut playlists: Vec<Playlist> = Vec::new();

    let shelf = Playlist {
        name: DEFAULT_PLAYLIST.to_string(),
        tracks: shelf_tracks,
    };
    playlists.push(shelf);

    for (i, (name, _)) in playlist_dirs.iter().enumerate() {
        if playlist_tracks[i].is_empty() {
            continue;
        }
        playlists.push(Playlist {
            name: name.clone(),
            tracks: std::mem::take(&mut playlist_tracks[i]),
        });
    }

    if shelf_empty && playlists.len() == 1 {
        playlists.clear();
    }

    playlists
}

fn load_or_fallback(p: &Path) -> Track {
    match load(p) {
        Ok(t) => t,
        Err(_) => Track {
            path: p.to_path_buf(),
            title: String::new(),
            artist: String::new(),
            duration: 0.0,
            lyrics: None,
        },
    }
}

fn load(p: &Path) -> anyhow::Result<Track> {
    let tagged = Probe::open(p)?.read()?;
    let tags = tagged.primary_tag();
    let (title, artist) = if let Some(t) = tags {
        (
            t.title().map(|s| s.to_string()).unwrap_or_default(),
            t.artist().map(|s| s.to_string()).unwrap_or_default(),
        )
    } else {
        (String::new(), String::new())
    };
    let duration = tagged.properties().duration().as_secs_f64();
    let lyrics = tags
        .and_then(|t| t.get_string(&ItemKey::Lyrics))
        .map(|s| s.to_string());
    Ok(Track {
        path: p.to_path_buf(),
        title,
        artist,
        duration,
        lyrics,
    })
}

pub fn default_music_dir() -> PathBuf {
    if let Some(dir) = dirs_fallback() {
        return dir;
    }
    PathBuf::from(".")
}

fn dirs_fallback() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join("Music"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Music"))
    }
}
