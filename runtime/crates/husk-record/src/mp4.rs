//! Minimal MP4 (ISO/IEC 14496-12) muxer for one H.264 track.
//!
//! Why not GStreamer: `filesrc`/`filesink` open their files through
//! libc-internal `open`, which no GOT patch reaches (the documented gap in
//! docs/design/libc-interpose.md) — measured: `filesink location=/tmp/x.png`
//! fails with ENOENT inside the guest, with and without any preload. Recording
//! must not depend on that, so the container is written here.
//!
//! Non-fragmented layout: ftyp | mdat | moov. Samples are Annex-B access units
//! converted to length-prefixed (AVCC) form; SPS/PPS go into avcC.

struct Sample {
    avcc: Vec<u8>,
    key: bool,
}

fn box_(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + payload.len());
    v.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(typ);
    v.extend_from_slice(payload);
    v
}

fn full_box(typ: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + payload.len());
    p.push(version);
    p.extend_from_slice(&flags.to_be_bytes()[1..]);
    p.extend_from_slice(payload);
    box_(typ, &p)
}

/// Split an Annex-B buffer into NAL payloads (start codes removed).
fn nals(d: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut start: Option<usize> = None;
    while i + 3 <= d.len() {
        let sc3 = d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1;
        let sc4 = i + 4 <= d.len() && d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 0 && d[i + 3] == 1;
        if sc3 || sc4 {
            if let Some(s) = start {
                let e = if s == i { i } else { i };
                if e > s {
                    out.push(&d[s..e]);
                }
            }
            i += if sc3 { 3 } else { 4 };
            start = Some(i);
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        if d.len() > s {
            out.push(&d[s..]);
        }
    }
    // trim trailing zero padding
    if let Some(last) = out.last_mut() {
        let mut n = last.len();
        while n > 0 && last[n - 1] == 0 {
            n -= 1;
        }
        *last = &last[..n];
    }
    out
}

fn avcc_sample(annexb: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(annexb.len());
    for nal in nals(annexb) {
        v.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        v.extend_from_slice(nal);
    }
    v
}

fn avcc_config(sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let profile = sps.get(1).copied().unwrap_or(0x64);
    let compat = sps.get(2).copied().unwrap_or(0);
    let level = sps.get(3).copied().unwrap_or(0x1e);
    let mut p = vec![1u8, profile, compat, level, 0xfc | 3, 0xe0 | 1];
    p.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    p.extend_from_slice(sps);
    p.push(1);
    p.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    p.extend_from_slice(pps);
    // High profiles carry chroma/bit-depth fields.
    if matches!(profile, 100 | 110 | 122 | 144) {
        p.extend_from_slice(&[0xfc | 1, 0xf8, 0xf8, 0]);
        p.push(0);
    }
    box_(b"avcC", &p)
}

fn avc1(w: u16, h: u16, avcc: &[u8]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&[0u8; 6]);
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&[0u8; 16]);
    p.extend_from_slice(&w.to_be_bytes());
    p.extend_from_slice(&h.to_be_bytes());
    p.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    p.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes());
    p.extend_from_slice(&1u16.to_be_bytes());
    let mut name = [0u8; 32];
    let label = b"husk";
    name[0] = label.len() as u8;
    name[1..1 + label.len()].copy_from_slice(label);
    p.extend_from_slice(&name);
    p.extend_from_slice(&0x0018u16.to_be_bytes());
    p.extend_from_slice(&0xffffu16.to_be_bytes());
    p.extend_from_slice(avcc);
    box_(b"avc1", &p)
}

/// `annexb` are the access units in capture order (CSD excluded), `key` marks IDR.
pub fn write(
    path: &str,
    w: u32,
    h: u32,
    fps: f64,
    annexb: &[Vec<u8>],
    key: &[bool],
    sps: &[u8],
    pps: &[u8],
) -> std::io::Result<u64> {
    let n = annexb.len();
    let timescale: u32 = ((fps * 1000.0).round() as u32).max(1);
    let delta: u32 = 1000;
    let duration: u32 = (n as u64 * delta as u64) as u32;

    let samples: Vec<Sample> = annexb
        .iter()
        .enumerate()
        .map(|(i, a)| Sample {
            avcc: avcc_sample(a),
            key: key.get(i).copied().unwrap_or(false),
        })
        .collect();

    // ftyp
    let mut ftyp_p = Vec::new();
    ftyp_p.extend_from_slice(b"isom");
    ftyp_p.extend_from_slice(&512u32.to_be_bytes());
    for b in [b"isom", b"iso2", b"avc1", b"mp41"] {
        ftyp_p.extend_from_slice(b);
    }
    let ftyp = box_(b"ftyp", &ftyp_p);

    // mdat
    let mdat_data_len: usize = samples.iter().map(|s| s.avcc.len()).sum();
    let mut mdat = Vec::with_capacity(mdat_data_len + 8);
    mdat.extend_from_slice(&((mdat_data_len + 8) as u32).to_be_bytes());
    mdat.extend_from_slice(b"mdat");
    let mdat_payload = mdat.len(); // header length
    for s in &samples {
        mdat.extend_from_slice(&s.avcc);
    }

    // moov
    let mut mvhd_p = Vec::new();
    mvhd_p.extend_from_slice(&[0u8; 8]); // creation + modification
    mvhd_p.extend_from_slice(&timescale.to_be_bytes());
    mvhd_p.extend_from_slice(&duration.to_be_bytes());
    mvhd_p.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate 1.0
    mvhd_p.extend_from_slice(&0x0100u16.to_be_bytes()); // volume 1.0
    mvhd_p.extend_from_slice(&[0u8; 10]);
    mvhd_p.extend_from_slice(&[
        0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // unity matrix
        0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0,
        0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    mvhd_p.extend_from_slice(&[0u8; 24]); // pre_defined
    mvhd_p.extend_from_slice(&2u32.to_be_bytes()); // next track id
    let mvhd = full_box(b"mvhd", 0, 0, &mvhd_p);

    let mut tkhd_p = Vec::new();
    tkhd_p.extend_from_slice(&[0u8; 8]);
    tkhd_p.extend_from_slice(&1u32.to_be_bytes()); // track id
    tkhd_p.extend_from_slice(&0u32.to_be_bytes());
    tkhd_p.extend_from_slice(&duration.to_be_bytes());
    tkhd_p.extend_from_slice(&[0u8; 8]);
    tkhd_p.extend_from_slice(&0u16.to_be_bytes()); // layer
    tkhd_p.extend_from_slice(&0u16.to_be_bytes()); // alternate group
    tkhd_p.extend_from_slice(&0u16.to_be_bytes()); // volume (video = 0)
    tkhd_p.extend_from_slice(&0u16.to_be_bytes());
    tkhd_p.extend_from_slice(&[
        0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0,
        0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ]);
    tkhd_p.extend_from_slice(&((w as u32) << 16).to_be_bytes());
    tkhd_p.extend_from_slice(&((h as u32) << 16).to_be_bytes());
    let tkhd = full_box(b"tkhd", 0, 7, &tkhd_p); // enabled | in movie | in preview

    let mut mdhd_p = Vec::new();
    mdhd_p.extend_from_slice(&[0u8; 8]);
    mdhd_p.extend_from_slice(&timescale.to_be_bytes());
    mdhd_p.extend_from_slice(&duration.to_be_bytes());
    mdhd_p.extend_from_slice(&0x55c4u16.to_be_bytes()); // language "und"
    mdhd_p.extend_from_slice(&0u16.to_be_bytes());
    let mdhd = full_box(b"mdhd", 0, 0, &mdhd_p);

    let mut hdlr_p = Vec::new();
    hdlr_p.extend_from_slice(&0u32.to_be_bytes());
    hdlr_p.extend_from_slice(b"vide");
    hdlr_p.extend_from_slice(&[0u8; 12]);
    hdlr_p.extend_from_slice(b"VideoHandler\0");
    let hdlr = full_box(b"hdlr", 0, 0, &hdlr_p);

    let mut vmhd_p = Vec::new();
    vmhd_p.extend_from_slice(&0u16.to_be_bytes()); // graphics mode copy
    vmhd_p.extend_from_slice(&[0u8; 6]); // opcolor
    let vmhd = full_box(b"vmhd", 0, 1, &vmhd_p);

    let url = full_box(b"url ", 0, 1, &[]);
    let dref = full_box(b"dref", 0, 0, &{
        let mut p = 1u32.to_be_bytes().to_vec();
        p.extend_from_slice(&url);
        p
    });
    let dinf = box_(b"dinf", &dref);

    let stsd = full_box(b"stsd", 0, 0, &{
        let mut p = 1u32.to_be_bytes().to_vec();
        p.extend_from_slice(&avc1(w as u16, h as u16, &avcc_config(sps, pps)));
        p
    });

    let stts = full_box(b"stts", 0, 0, &{
        let mut p = 1u32.to_be_bytes().to_vec(); // one entry
        p.extend_from_slice(&(n as u32).to_be_bytes());
        p.extend_from_slice(&delta.to_be_bytes());
        p
    });

    let keys: Vec<u32> = key
        .iter()
        .enumerate()
        .filter(|(_, k)| **k)
        .map(|(i, _)| (i + 1) as u32)
        .collect();
    let stss = full_box(b"stss", 0, 0, &{
        let mut p = (keys.len() as u32).to_be_bytes().to_vec();
        for k in &keys {
            p.extend_from_slice(&k.to_be_bytes());
        }
        p
    });

    let stsc = full_box(b"stsc", 0, 0, &{
        let mut p = 1u32.to_be_bytes().to_vec();
        p.extend_from_slice(&1u32.to_be_bytes()); // first chunk
        p.extend_from_slice(&(n as u32).to_be_bytes()); // samples per chunk
        p.extend_from_slice(&1u32.to_be_bytes()); // sample description index
        p
    });

    let stsz = full_box(b"stsz", 0, 0, &{
        let mut p = 0u32.to_be_bytes().to_vec(); // constant size 0 = table
        p.extend_from_slice(&(n as u32).to_be_bytes());
        for s in &samples {
            p.extend_from_slice(&(s.avcc.len() as u32).to_be_bytes());
        }
        p
    });

    let chunk_offset = (ftyp.len() + mdat_payload) as u32;
    let stco = full_box(b"stco", 0, 0, &{
        let mut p = 1u32.to_be_bytes().to_vec();
        p.extend_from_slice(&chunk_offset.to_be_bytes());
        p
    });

    let stbl = box_(
        b"stbl",
        &[stsd, stts, stss, stsc, stsz, stco].concat(),
    );
    let minf = box_(b"minf", &[vmhd, dinf, stbl].concat());
    let mdia = box_(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = box_(b"trak", &[tkhd, mdia].concat());
    let moov = box_(b"moov", &[mvhd, trak].concat());

    let mut out = Vec::with_capacity(ftyp.len() + mdat.len() + moov.len());
    out.extend_from_slice(&ftyp);
    out.extend_from_slice(&mdat);
    out.extend_from_slice(&moov);
    std::fs::write(path, &out)?;
    Ok(out.len() as u64)
}

/// Pull SPS and PPS out of a CSD chunk (Annex-B).
pub fn param_sets(csd: &[u8], sps: &mut Vec<u8>, pps: &mut Vec<u8>) {
    for nal in nals(csd) {
        match nal.first().map(|b| b & 0x1f) {
            Some(7) if sps.is_empty() => *sps = nal.to_vec(),
            Some(8) if pps.is_empty() => *pps = nal.to_vec(),
            _ => {}
        }
    }
}
