//! Minimal H.264 parameter-set synthesis for the VA-API system bridge.
//!
//! The VA decode API gives the driver parsed picture and slice parameters, not
//! the original SPS/PPS byte stream. MediaCodec needs Annex-B CSD, so this
//! module reconstructs a conservative SPS/PPS pair from the fields that are
//! actually present in `VAPictureParameterBufferH264` and the first slice
//! parameter buffer. It is intentionally independent of any player.

use super::va_bindings::{
    VAProfile, VAProfile_VAProfileH264Baseline, VAProfile_VAProfileH264ConstrainedBaseline,
    VAProfile_VAProfileH264High, VAProfile_VAProfileH264Main,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct H264Csd {
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

pub fn build_csd(
    profile: VAProfile,
    picture: &[u8],
    slice: Option<&[u8]>,
    visible_width: u32,
    visible_height: u32,
) -> Result<H264Csd, String> {
    // VAPictureParameterBufferH264 is 672 bytes in VA-API 1.20. Reject a
    // short or malformed buffer instead of reading past the mapped VA buffer.
    if picture.len() < 640 {
        return Err(format!(
            "H264 picture parameter buffer too short: {} bytes",
            picture.len()
        ));
    }

    let width_mbs_minus1 = read_u16(picture, 612)?;
    let height_mbs_minus1 = read_u16(picture, 614)?;
    let bit_depth_luma_minus8 = read_u8(picture, 616)?;
    let bit_depth_chroma_minus8 = read_u8(picture, 617)?;
    let num_ref_frames = read_u8(picture, 618)?.max(1);
    let seq_fields = read_u32(picture, 620)?;
    let num_slice_groups_minus1 = read_u8(picture, 624)?;
    if num_slice_groups_minus1 != 0 {
        return Err(format!(
            "H264 FMO is not supported: num_slice_groups_minus1={num_slice_groups_minus1}"
        ));
    }
    let pic_init_qp_minus26 = read_i8(picture, 628)?;
    let pic_init_qs_minus26 = read_i8(picture, 629)?;
    let chroma_qp_index_offset = read_i8(picture, 630)?;
    let second_chroma_qp_index_offset = read_i8(picture, 631)?;
    let pic_fields = read_u32(picture, 632)?;

    let chroma_format_idc = seq_fields & 0x3;
    let separate_colour_plane_flag = (seq_fields >> 2) & 1;
    let gaps_in_frame_num_value_allowed_flag = (seq_fields >> 3) & 1;
    let frame_mbs_only_flag = (seq_fields >> 4) & 1;
    let mb_adaptive_frame_field_flag = (seq_fields >> 5) & 1;
    let direct_8x8_inference_flag = (seq_fields >> 6) & 1;
    let log2_max_frame_num_minus4 = (seq_fields >> 8) & 0xf;
    let pic_order_cnt_type = (seq_fields >> 12) & 0x3;
    let log2_max_pic_order_cnt_lsb_minus4 = (seq_fields >> 14) & 0xf;
    let delta_pic_order_always_zero_flag = (seq_fields >> 18) & 1;

    let profile_idc = profile_idc(profile, chroma_format_idc, bit_depth_luma_minus8);
    let high_profile = matches!(profile_idc, 100 | 110 | 122 | 244);
    let level_idc = level_idc_for(visible_width, visible_height);

    let (default_l0, default_l1) = match slice {
        Some(data) if data.len() >= 20 => (data[18], data[19]),
        _ => (num_ref_frames.saturating_sub(1), 0),
    };

    let mut sps = BitWriter::new(128);
    sps.put_bits(8, 0x67); // nal_ref_idc=3, nal_unit_type=7
    sps.put_bits(8, profile_idc as u32);
    sps.put_bits(8, 0); // constraint_set flags and reserved bits
    sps.put_bits(8, level_idc as u32);
    sps.ue(0); // seq_parameter_set_id

    if high_profile {
        sps.ue(chroma_format_idc);
        if chroma_format_idc == 3 {
            sps.put_bit(separate_colour_plane_flag != 0);
        }
        sps.ue(bit_depth_luma_minus8 as u32);
        sps.ue(bit_depth_chroma_minus8 as u32);
        sps.put_bit(false); // qpprime_y_zero_transform_bypass_flag
        sps.put_bit(false); // seq_scaling_matrix_present_flag
    }

    sps.ue(log2_max_frame_num_minus4);
    sps.ue(pic_order_cnt_type);
    if pic_order_cnt_type == 0 {
        sps.ue(log2_max_pic_order_cnt_lsb_minus4);
    } else if pic_order_cnt_type == 1 {
        // VA-API's H.264 picture buffer does not carry the offset table. A
        // zero-length cycle is valid and keeps the synthetic SPS minimal.
        sps.put_bit(delta_pic_order_always_zero_flag != 0);
        sps.se(0); // offset_for_non_ref_pic
        sps.se(0); // offset_for_top_to_bottom_field
        sps.ue(0); // num_ref_frames_in_pic_order_cnt_cycle
    }

    sps.ue(num_ref_frames as u32);
    sps.put_bit(gaps_in_frame_num_value_allowed_flag != 0);
    sps.ue(width_mbs_minus1 as u32);
    sps.ue(height_mbs_minus1 as u32);
    sps.put_bit(frame_mbs_only_flag != 0);
    if frame_mbs_only_flag == 0 {
        sps.put_bit(mb_adaptive_frame_field_flag != 0);
    }
    sps.put_bit(direct_8x8_inference_flag != 0);

    let coded_width = (width_mbs_minus1 as u32 + 1) * 16;
    let coded_height =
        (height_mbs_minus1 as u32 + 1) * 16 * if frame_mbs_only_flag != 0 { 1 } else { 2 };
    let crop_unit_x = if chroma_format_idc == 0 { 1 } else { 2 };
    let crop_unit_y = if chroma_format_idc == 0 {
        if frame_mbs_only_flag != 0 {
            1
        } else {
            2
        }
    } else if frame_mbs_only_flag != 0 {
        2
    } else {
        4
    };
    let crop_right = crop_value(coded_width, visible_width, crop_unit_x);
    let crop_bottom = crop_value(coded_height, visible_height, crop_unit_y);
    let cropped = crop_right != 0 || crop_bottom != 0;
    sps.put_bit(cropped);
    if cropped {
        sps.ue(0); // frame_crop_left_offset
        sps.ue(crop_right);
        sps.ue(0); // frame_crop_top_offset
        sps.ue(crop_bottom);
    }

    // Keep a bitstream-restriction VUI. Some Android decoders otherwise
    // choose an output window that is too small for streams with B frames.
    sps.put_bit(true); // vui_parameters_present_flag
    sps.put_bit(false); // aspect_ratio_info_present_flag
    sps.put_bit(false); // overscan_info_present_flag
    sps.put_bit(false); // video_signal_type_present_flag
    sps.put_bit(false); // chroma_loc_info_present_flag
    sps.put_bit(false); // timing_info_present_flag
    sps.put_bit(false); // nal_hrd_parameters_present_flag
    sps.put_bit(false); // vcl_hrd_parameters_present_flag
    sps.put_bit(false); // pic_struct_present_flag
    sps.put_bit(true); // bitstream_restriction_flag
    sps.put_bit(true); // motion_vectors_over_pic_boundaries_flag
    sps.ue(0); // max_bytes_per_pic_denom
    sps.ue(0); // max_bits_per_mb_denom
    sps.ue(11); // log2_max_mv_length_horizontal
    sps.ue(11); // log2_max_mv_length_vertical
    sps.ue(if num_ref_frames > 2 { 2 } else { 0 }); // max_num_reorder_frames
    sps.ue(num_ref_frames as u32); // max_dec_frame_buffering
    sps.finish();

    let mut pps = BitWriter::new(64);
    pps.put_bits(8, 0x68); // nal_ref_idc=3, nal_unit_type=8
    pps.ue(0); // pic_parameter_set_id
    pps.ue(0); // seq_parameter_set_id
    pps.put_bit((pic_fields & 1) != 0); // entropy_coding_mode_flag
    pps.put_bit(((pic_fields >> 7) & 1) != 0); // bottom_field_pic_order_in_frame_present_flag
    pps.ue(num_slice_groups_minus1 as u32);
    pps.ue(default_l0 as u32);
    pps.ue(default_l1 as u32);
    pps.put_bit(((pic_fields >> 1) & 1) != 0); // weighted_pred_flag
    pps.put_bits(2, (pic_fields >> 2) & 0x3); // weighted_bipred_idc
    pps.se(pic_init_qp_minus26 as i32);
    pps.se(pic_init_qs_minus26 as i32);
    pps.se(chroma_qp_index_offset as i32);
    pps.put_bit(((pic_fields >> 8) & 1) != 0); // deblocking_filter_control_present_flag
    pps.put_bit(((pic_fields >> 6) & 1) != 0); // constrained_intra_pred_flag
    pps.put_bit(((pic_fields >> 9) & 1) != 0); // redundant_pic_cnt_present_flag
    if high_profile {
        pps.put_bit(((pic_fields >> 4) & 1) != 0); // transform_8x8_mode_flag
        pps.put_bit(false); // pic_scaling_matrix_present_flag
        pps.se(second_chroma_qp_index_offset as i32);
    }
    pps.finish();

    Ok(H264Csd {
        sps: with_start_code(sps.into_bytes()),
        pps: with_start_code(pps.into_bytes()),
    })
}

fn with_start_code(nal: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len() + 4);
    out.extend_from_slice(&[0, 0, 0, 1]);
    out.extend_from_slice(&nal);
    out
}

fn crop_value(coded: u32, visible: u32, unit: u32) -> u32 {
    if visible != 0 && coded > visible && (coded - visible) % unit == 0 {
        (coded - visible) / unit
    } else {
        0
    }
}

fn profile_idc(profile: VAProfile, chroma_format_idc: u32, bit_depth_luma_minus8: u8) -> u8 {
    match profile {
        VAProfile_VAProfileH264Baseline | VAProfile_VAProfileH264ConstrainedBaseline => 66,
        VAProfile_VAProfileH264Main => 77,
        VAProfile_VAProfileH264High => 100,
        _ if bit_depth_luma_minus8 != 0 || chroma_format_idc > 1 => 100,
        _ => 77,
    }
}

fn level_idc_for(width: u32, height: u32) -> u8 {
    let pixels = width.saturating_mul(height);
    if pixels > 1920 * 1080 {
        51
    } else if pixels > 1280 * 720 {
        40
    } else {
        30
    }
}

fn read_u8(data: &[u8], offset: usize) -> Result<u8, String> {
    data.get(offset)
        .copied()
        .ok_or_else(|| format!("missing byte at offset {offset}"))
}

fn read_i8(data: &[u8], offset: usize) -> Result<i8, String> {
    Ok(read_u8(data, offset)? as i8)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| format!("missing u16 at offset {offset}"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| format!("missing u32 at offset {offset}"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

struct BitWriter {
    bytes: Vec<u8>,
    current: u32,
    bits: u8,
    zeros: u8,
}

impl BitWriter {
    fn new(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
            current: 0,
            bits: 0,
            zeros: 0,
        }
    }

    fn put_bit(&mut self, bit: bool) {
        self.current = (self.current << 1) | u32::from(bit);
        self.bits += 1;
        if self.bits == 8 {
            let byte = self.current as u8;
            self.write_ebsp_byte(byte);
            self.current = 0;
            self.bits = 0;
        }
    }

    fn put_bits(&mut self, count: u8, value: u32) {
        for shift in (0..count).rev() {
            self.put_bit(((value >> shift) & 1) != 0);
        }
    }

    fn ue(&mut self, value: u32) {
        let value = value.saturating_add(1);
        let leading = 31 - value.leading_zeros();
        self.put_bits(leading as u8, 0);
        self.put_bits(leading as u8 + 1, value);
    }

    fn se(&mut self, value: i32) {
        let mapped = if value <= 0 {
            (-(value as i64) as u32).saturating_mul(2)
        } else {
            (value as u32).saturating_mul(2).saturating_sub(1)
        };
        self.ue(mapped);
    }

    fn finish(&mut self) {
        self.put_bit(true);
        while self.bits != 0 {
            self.put_bit(false);
        }
    }

    fn write_ebsp_byte(&mut self, byte: u8) {
        if self.zeros >= 2 && byte <= 3 {
            self.bytes.push(3);
            self.zeros = 0;
        }
        self.bytes.push(byte);
        if byte == 0 {
            self.zeros = self.zeros.saturating_add(1);
        } else {
            self.zeros = 0;
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture_1080p() -> Vec<u8> {
        let mut picture = vec![0u8; 672];
        picture[612..614].copy_from_slice(&119u16.to_le_bytes());
        picture[614..616].copy_from_slice(&67u16.to_le_bytes());
        picture[618] = 4;
        picture[620..624].copy_from_slice(&(1u32 | (1u32 << 4)).to_le_bytes());
        picture
    }

    #[test]
    fn builds_high_profile_sps_and_pps() {
        let csd = build_csd(
            VAProfile_VAProfileH264High,
            &picture_1080p(),
            None,
            1920,
            1080,
        )
        .expect("valid H264 picture parameters");
        assert_eq!(&csd.sps[..5], &[0, 0, 0, 1, 0x67]);
        assert_eq!(csd.sps[5], 100);
        assert_eq!(&csd.pps[..5], &[0, 0, 0, 1, 0x68]);
        assert!(!has_emulation_prevention_violation(&csd.sps));
        assert!(!has_emulation_prevention_violation(&csd.pps));
    }

    #[test]
    fn rejects_short_picture_buffer() {
        assert!(build_csd(VAProfile_VAProfileH264High, &[0; 16], None, 1920, 1080).is_err());
    }

    fn has_emulation_prevention_violation(data: &[u8]) -> bool {
        data.get(4..)
            .unwrap_or_default()
            .windows(3)
            .any(|w| w[0] == 0 && w[1] == 0 && w[2] <= 3)
    }
}
