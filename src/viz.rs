use rustfft::{num_complex::Complex, FftPlanner};
use std::cell::RefCell;

pub const FFT_N: usize = 2048;

thread_local! {
    /// Reused across frames so plans are computed once, not every 33ms.
    static FFT_PLANNER: RefCell<FftPlanner<f32>> = RefCell::new(FftPlanner::new());
}

/// Compute `num_bars` raw spectrum magnitudes in [0.0, 1.0].
/// Gamma-compressed so typical audio only fills the lower portion; only peaks reach the top.
/// Quadratic-frequency binning concentrates resolution near DC (low frequencies).
pub fn compute(samples: &[f32], num_bars: usize) -> Vec<f32> {
    let num_bars = num_bars.max(1);
    let mut input = [Complex::new(0.0, 0.0); FFT_N];
    for (i, &s) in samples.iter().take(FFT_N).enumerate() {
        let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / FFT_N as f32).cos());
        input[i] = Complex::new(s * w, 0.0);
    }
    FFT_PLANNER.with(|p| {
        let fft = p.borrow_mut().plan_fft_forward(FFT_N);
        fft.process(&mut input);
    });

    // mag typically in [0, 1]; normalize then gamma > 1 so low/medium stays low,
    // only genuine peaks approach 1.0.
    // 第一遍：计算每个频段的原始magnitude
    let max_k = (FFT_N / 2) as f32;
    let mut mags = vec![0.0f32; num_bars];
    let mut peak = 1e-6f32;
    for b in 0..num_bars {
        let f0 = (b as f32 / num_bars as f32).powi(2);
        let f1 = ((b + 1) as f32 / num_bars as f32).powi(2);
        let lo = (f0 * max_k) as usize;
        let hi = (((f1 * max_k) as usize) + 1).min(FFT_N / 2).max(lo + 1);
        let mut mag = 0.0f32;
        for k in lo..hi {
            mag = mag.max(input[k].norm());
        }
        mags[b] = mag;
        if mag > peak {
            peak = mag;
        }
    }
    // 第二遍：按本帧峰值归一化到 [0,1]，再用 gamma>1 压缩
    const FLOOR_RATIO: f32 = 0.03; // 数值低于峰值的3%视为无声
    const GAMMA: f32 = 1.2; // 接近线性，变化幅度跟原始信号紧
    const HEADROOM: f32 = 1.0; // 峰值可到顶
    let mut out = vec![0.0f32; num_bars];
    for b in 0..num_bars {
        let r = mags[b] / peak; // [0, 1]
        let m = (r - FLOOR_RATIO).max(0.0) / (1.0 - FLOOR_RATIO);
        out[b] = (m.powf(GAMMA) * HEADROOM).clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq_hz: f32, rate: u32) -> Vec<f32> {
        let n = FFT_N;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq_hz * i as f32 / rate as f32).sin())
            .collect()
    }

    #[test]
    fn silence_is_zero() {
        let out = compute(&vec![0.0; FFT_N], 64);
        assert!(out.iter().all(|&v| v == 0.0));
        assert_eq!(out.len(), 64);
    }

    #[test]
    fn output_is_normalized() {
        let out = compute(&sine(440.0, 44100), 128);
        assert!(out.iter().all(|&v| (0.0..=1.0).contains(&v)));
        let peak = out.iter().cloned().fold(0.0f32, f32::max);
        assert!(
            peak > 0.9,
            "a sustained full-scale tone should reach the top"
        );
    }

    #[test]
    fn low_tone_peaks_in_low_bucket() {
        let out = compute(&sine(100.0, 44100), 64);
        let peak_idx = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!(
            peak_idx < 8,
            "100Hz energy should sit near the low end, got {}",
            peak_idx
        );
    }

    #[test]
    fn short_input_is_ok() {
        let out = compute(&[0.5; 16], 32);
        assert_eq!(out.len(), 32);
        assert!(out.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
}
