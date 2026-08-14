use anyhow::Result;
use rodio::source::Source;
use rodio::{Decoder, OutputStream, Sink};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// O(1) circular buffer of recent PCM samples (mono mix) for visualization.
#[derive(Clone)]
pub struct VizBuffer {
    inner: Arc<Mutex<VizRing>>,
}

struct VizRing {
    buf: Vec<f32>,
    head: usize,
    len: usize,
}

impl VizRing {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0.0; capacity],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, s: f32) {
        let cap = self.buf.len();
        if self.len < cap {
            self.buf[(self.head + self.len) % cap] = s;
            self.len += 1;
        } else {
            self.buf[self.head] = s;
            self.head = (self.head + 1) % cap;
        }
    }

    fn latest(&self, n: usize) -> Vec<f32> {
        let take = n.min(self.len);
        let mut out = Vec::with_capacity(take);
        for k in 0..take {
            out.push(self.buf[(self.head + self.len - take + k) % self.buf.len()]);
        }
        out
    }
}

impl VizBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VizRing::new(capacity))),
        }
    }

    /// Push a batch of mono samples under a single lock.
    pub fn push_batch(&self, samples: &[f32]) {
        // 锁中毒（另一线程 panic）不应连带杀死音频线程
        let mut r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for &s in samples {
            r.push(s);
        }
    }

    pub fn latest(&self, n: usize) -> Vec<f32> {
        // 锁中毒（另一线程 panic）不应连带杀死音频线程
        let r = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = r.latest(n);
        if out.len() < n {
            out.resize(n, 0.0);
        }
        out
    }
}

/// Samples to accumulate before pushing into the ring buffer (one lock per batch).
const PUSH_BATCH: usize = 256;

/// A Source wrapper that down-mixes to mono and mirrors samples into a VizBuffer.
struct MonoTee<S: Source<Item = f32>> {
    inner: S,
    viz: VizBuffer,
    pending: Vec<f32>,
}

impl<S: Source<Item = f32>> MonoTee<S> {
    fn flush(&mut self) {
        if !self.pending.is_empty() {
            self.viz.push_batch(&self.pending);
            self.pending.clear();
        }
    }
}

impl<S: Source<Item = f32>> Iterator for MonoTee<S> {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let ch = u16::max(self.inner.channels(), 1) as usize;
        let mut sum = 0.0f32;
        let mut n = 0usize;
        for _ in 0..ch {
            if let Some(s) = self.inner.next() {
                sum += s;
                n += 1;
            } else {
                break;
            }
        }
        if n == 0 {
            self.flush();
            return None;
        }
        let mono = sum / n as f32;
        self.pending.push(mono);
        if self.pending.len() >= PUSH_BATCH {
            self.flush();
        }
        Some(mono)
    }
}

impl<S: Source<Item = f32>> Source for MonoTee<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        1
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

pub enum RepeatMode {
    Off,
    One,
    All,
}

pub struct Player {
    _stream: OutputStream,
    sink: Sink,
    pub viz: VizBuffer,
    pub playing: bool,
    pub position: Duration,
}

impl Player {
    pub fn new() -> Result<Self> {
        let (stream, handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&handle)?;
        sink.pause();
        Ok(Self {
            _stream: stream,
            sink,
            viz: VizBuffer::new(4096),
            playing: false,
            position: Duration::ZERO,
        })
    }

    pub fn play_file(&mut self, path: &Path) -> Result<()> {
        let file = std::fs::File::open(path)?;
        let dec = Decoder::new(file)?;
        let tee = MonoTee {
            inner: dec.convert_samples(),
            viz: self.viz.clone(),
            pending: Vec::with_capacity(PUSH_BATCH),
        };
        self.sink.clear();
        self.sink.append(tee);
        self.sink.play();
        self.playing = true;
        self.position = Duration::ZERO;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.sink.stop();
        self.playing = false;
        self.position = Duration::ZERO;
    }

    pub fn pause(&mut self) {
        self.sink.pause();
        self.playing = false;
    }

    pub fn resume(&mut self) {
        self.sink.play();
        self.playing = true;
    }

    pub fn is_finished(&self) -> bool {
        self.sink.empty()
    }

    /// Real playback position from the audio backend (accurate across pause/buffer).
    pub fn update_position(&mut self) {
        self.position = self.sink.get_pos();
    }

    /// Seek 到绝对位置。失败时返回错误，由调用方展示在 footer。
    pub fn seek(&self, target: Duration) -> anyhow::Result<()> {
        self.sink
            .try_seek(target)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn set_volume(&self, v: f32) {
        self.sink.set_volume(v);
    }
}
