use std::path::PathBuf;
use tui_music::library::{is_audio, scan};

#[test]
fn detects_audio_extensions() {
    assert!(is_audio(&PathBuf::from("foo.mp3")));
    assert!(is_audio(&PathBuf::from("BAR.FLAC")));
    assert!(!is_audio(&PathBuf::from("notes.txt")));
    assert!(!is_audio(&PathBuf::from("noext")));
}

#[test]
fn scans_generated_sample_dir() {
    let dir = PathBuf::from("/tmp/tmtest/music");
    if !dir.exists() {
        return;
    }
    let playlists = scan(&dir);
    assert!(!playlists.is_empty());
    let total: usize = playlists.iter().map(|p| p.tracks.len()).sum();
    assert!(total >= 2);
    let has_mp3 = playlists.iter().any(|p| {
        p.tracks
            .iter()
            .any(|t| t.path.extension().unwrap_or_default() == "mp3")
    });
    assert!(has_mp3);
}
