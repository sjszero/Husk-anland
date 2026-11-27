//! Audit an Android `screencap` raw frame without decoding PNG or adding a
//! dependency. The command is deliberately narrow: it answers whether the
//! centre of the frame is an all-black/uniform surface or has a large missing
//! quadrant, which are useful checks for host-captured X11/desktop frames. It does not try to recognise a desktop or a particular theme.

use std::collections::HashSet;
use std::fs;

const SAMPLE_LIMIT: usize = 240_000;

#[derive(Clone, Copy, Default)]
struct Bucket {
    count: u64,
    sum: u64,
    sum_sq: u64,
    near_black: u64,
    max: u8,
}

impl Bucket {
    fn add(&mut self, luma: u8) {
        let v = luma as u64;
        self.count += 1;
        self.sum += v;
        self.sum_sq += v * v;
        self.max = self.max.max(luma);
        if luma <= 4 {
            self.near_black += 1;
        }
    }

    fn mean(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.sum as f64 / self.count as f64
        }
    }

    fn stddev(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let n = self.count as f64;
        let mean = self.sum as f64 / n;
        let variance = (self.sum_sq as f64 / n - mean * mean).max(0.0);
        variance.sqrt()
    }

    fn near_black_ratio(&self) -> f64 {
        if self.count == 0 {
            1.0
        } else {
            self.near_black as f64 / self.count as f64
        }
    }
}

struct Report {
    width: u32,
    height: u32,
    header_bytes: usize,
    samples: u64,
    center: Bucket,
    quadrants: [Bucket; 4],
    quantized_colors: usize,
}

fn parse_header(data: &[u8]) -> Result<(u32, u32, usize), String> {
    if data.len() < 16 {
        return Err(format!(
            "too short for screencap header: {} bytes",
            data.len()
        ));
    }
    let width = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let height = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let format = u32::from_le_bytes(data[8..12].try_into().unwrap());
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return Err(format!("invalid dimensions {width}x{height}"));
    }
    // Android PixelFormat: RGBA_8888=1, RGBX_8888=2, BGRA_8888=5.
    if !matches!(format, 1 | 2 | 5) {
        return Err(format!(
            "unsupported PixelFormat={format}; raw RGBA/RGBX/BGRA is required"
        ));
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "pixel byte count overflows usize".to_string())?;
    if data.len() < pixels {
        return Err(format!(
            "truncated frame: pixels need {pixels} bytes, file has {}",
            data.len()
        ));
    }
    let header = data.len() - pixels;
    // Android 13 emits a 16-byte header (width, height, format, dataspace).
    // Older releases emitted 12; newer releases may add metadata. Accept a
    // small bounded header so we still reject PNG or unrelated files.
    if !(12..=64).contains(&header) {
        return Err(format!(
            "unexpected screencap header length {header}; frame is not raw RGBA"
        ));
    }
    Ok((width, height, header))
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((77u32 * r as u32 + 150 * g as u32 + 29 * b as u32) >> 8) as u8
}

fn analyze(data: &[u8]) -> Result<Report, String> {
    let (width, height, header_bytes) = parse_header(data)?;
    let w = width as usize;
    let h = height as usize;
    let pixels = &data[header_bytes..];

    // Ignore the outer 15% on every side: status/navigation bars and a small
    // desktop margin should not make a black centre pass. The crop is split
    // into four quadrants so a missing half can be detected independently.
    let x0 = w * 15 / 100;
    let x1 = (w * 85 / 100).max(x0 + 1).min(w);
    let y0 = h * 15 / 100;
    let y1 = (h * 85 / 100).max(y0 + 1).min(h);
    let cw = x1 - x0;
    let ch = y1 - y0;
    let mut step = 1usize;
    while (cw / step).saturating_mul(ch / step) > SAMPLE_LIMIT {
        step += 1;
    }

    let format = u32::from_le_bytes(data[8..12].try_into().unwrap());
    let mut center = Bucket::default();
    let mut quadrants = [Bucket::default(); 4];
    let mut colors = HashSet::new();
    let mut samples = 0u64;
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let off = (y * w + x) * 4;
            let p = &pixels[off..off + 4];
            let (r, g, b) = match format {
                5 => (p[2], p[1], p[0]),
                _ => (p[0], p[1], p[2]),
            };
            let l = luma(r, g, b);
            center.add(l);
            let qx = usize::from(x >= x0 + cw / 2);
            let qy = usize::from(y >= y0 + ch / 2);
            quadrants[qy * 2 + qx].add(l);
            colors.insert(((r as u32 >> 4) << 8) | ((g as u32 >> 4) << 4) | (b as u32 >> 4));
            samples += 1;
            x += step;
        }
        y += step;
    }

    Ok(Report {
        width,
        height,
        header_bytes,
        samples,
        center,
        quadrants,
        quantized_colors: colors.len(),
    })
}

fn verdict(report: &Report, strict: bool) -> (bool, Option<String>) {
    if report.samples == 0 {
        return (false, Some("no sampled pixels".into()));
    }
    let c = &report.center;
    if c.near_black_ratio() >= 0.995 && c.mean() <= 2.0 && c.max <= 4 {
        return (false, Some("centre crop is all black".into()));
    }
    if strict {
        let mut black = 0usize;
        let mut visible = 0usize;
        for q in &report.quadrants {
            if q.near_black_ratio() >= 0.995 && q.mean() <= 2.0 && q.max <= 4 {
                black += 1;
            } else if q.mean() > 8.0 {
                visible += 1;
            }
        }
        // A desktop with one quadrant genuinely black is allowed (dark
        // wallpaper/terminal); a large missing region next to rendered content
        // is not. Require at least two mostly black quadrants and at least one
        // clearly rendered opposite/side quadrant.
        if black >= 2 && visible >= 1 {
            return (
                false,
                Some(format!(
                    "large missing region: {black}/4 centre quadrants are all black"
                )),
            );
        }
    }
    (true, None)
}

pub fn run(args: &[String]) -> i32 {
    let mut strict = false;
    let mut path: Option<&str> = None;
    for arg in args {
        match arg.as_str() {
            "--strict" => strict = true,
            "-h" | "--help" => {
                eprintln!("usage: husk-tools screenshot-audit [--strict] <raw-screencap>");
                return 0;
            }
            other if other.starts_with('-') => {
                eprintln!("screenshot-audit: unknown option {other}");
                return 2;
            }
            other => {
                if path.replace(other).is_some() {
                    eprintln!("screenshot-audit: exactly one raw frame is required");
                    return 2;
                }
            }
        }
    }
    let Some(path) = path else {
        eprintln!("usage: husk-tools screenshot-audit [--strict] <raw-screencap>");
        return 2;
    };
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("screenshot-audit: read {path}: {e}");
            return 2;
        }
    };
    let report = match analyze(&data) {
        Ok(report) => report,
        Err(e) => {
            eprintln!("screenshot-audit: FAIL {e}");
            return 1;
        }
    };
    let (ok, why) = verdict(&report, strict);
    let c = &report.center;
    let qmeans = report
        .quadrants
        .iter()
        .map(|q| format!("{:.1}", q.mean()))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "screenshot-audit: {} width={} height={} header={} samples={} mean={:.2} stddev={:.2} near_black={:.4} min_sample_max={} colors={} quadrant_mean={}",
        if ok { "PASS" } else { "FAIL" },
        report.width,
        report.height,
        report.header_bytes,
        report.samples,
        c.mean(),
        c.stddev(),
        c.near_black_ratio(),
        c.max,
        report.quantized_colors,
        qmeans
    );
    if let Some(reason) = why {
        eprintln!("screenshot-audit: {reason}");
    }
    if ok {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for y in 0..h {
            for x in 0..w {
                out.extend_from_slice(&pixel(x, y));
            }
        }
        out
    }

    #[test]
    fn black_frame_fails_even_without_strict() {
        let data = frame(32, 32, |_, _| [0, 0, 0, 255]);
        let report = analyze(&data).unwrap();
        assert!(!verdict(&report, false).0);
    }

    #[test]
    fn uniform_visible_frame_passes() {
        let data = frame(32, 32, |_, _| [180, 180, 180, 255]);
        let report = analyze(&data).unwrap();
        assert!(verdict(&report, true).0);
    }

    #[test]
    fn missing_right_half_fails_strict() {
        let data = frame(40, 40, |x, _| {
            if x < 20 {
                [180, 140, 90, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        let report = analyze(&data).unwrap();
        assert!(!verdict(&report, true).0);
        assert!(verdict(&report, false).0);
    }

    #[test]
    fn png_or_truncated_data_is_rejected() {
        assert!(analyze(b"\x89PNG\r\n\x1a\n").is_err());
        let mut data = frame(8, 8, |_, _| [20, 20, 20, 255]);
        // A 4-byte truncation is indistinguishable from a legitimate 12-byte
        // legacy header; trim beyond the accepted header range instead.
        data.truncate(data.len() - 20);
        assert!(analyze(&data).is_err());
    }

    #[test]
    fn twelve_byte_header_is_accepted() {
        let w = 8u32;
        let h = 8u32;
        let mut data = Vec::new();
        data.extend_from_slice(&w.to_le_bytes());
        data.extend_from_slice(&h.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend(std::iter::repeat([30, 90, 170, 255]).take(64).flatten());
        let report = analyze(&data).unwrap();
        assert_eq!(report.header_bytes, 12);
        assert!(verdict(&report, true).0);
    }
}
