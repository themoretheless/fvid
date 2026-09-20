//! Optional native FFmpeg-library adapter. No subprocess execution in production.
#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    unnecessary_transmutes,
    clippy::all
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/av.rs"));
}
mod budget;
mod edit;
mod input_policy;
mod plan;
pub use budget::{
    estimate_path_controlled_bytes, parse_max_memory_mib, parse_max_rss_mib,
};
pub use edit::{concat, parse_time, trim};
pub use input_policy::with_standalone_inputs;
pub use plan::{
    MediaPlan, PlanStep, PlanStream, plan_burn_subtitles, plan_concat, plan_decode_audio,
    plan_loudness, plan_loudnorm, plan_merge_audio, plan_mix_audio, plan_overlay, plan_remux,
    plan_transcode_lossless, plan_trim, plan_trim_pcm, plan_xfade,
};
mod audio;
mod audio_layout;
mod audio_mix;
mod decode;
mod play;
mod filter;
#[cfg(feature = "cuda-hw")]
mod hw_cuda;
mod loudness;
mod lossless;
mod pcm;
mod subtitle;
mod wav;
mod xfade;
pub use loudness::{
    DEFAULT_LOUDNORM_ARGS, LoudnessStats, LoudnormStats, apply_loudnorm, apply_loudnorm_dual,
    measure_loudness, validate_loudnorm_args,
};
pub use audio::{
    AudioDecodeStats, AudioDecodeTransform, decode_audio, decode_audio_interval,
    decode_audio_transformed,
};
pub use audio_mix::{
    MergeAudioStats, MixAudioOptions, MixAudioStats, MixDuration, merge_audio, mix_audio,
};
#[cfg(feature = "cuda-hw")]
pub use decode::decode_video_cuda;
pub use decode::{DecodeStats, DecodeTransform, decode_video, decode_video_transformed};
pub use play::{
    AbLoop, PlayOptions, PlayStats, SubtitleCue, ab_mark, ab_restart_us, active_subtitle,
    advance_rate_phase, audio_output_devices, clamp_rate_milli, cycle_track, encode_bmp,
    find_audio_device, is_playback_url, load_subtitle_file, parse_srt, parse_subtitle_clock,
    parse_subtitle_text, plain_subtitle, play, play_paths, playlist_step, scale_elapsed_us,
    snapshot_path, subtitle_window, chapter_step, cycle_repeat, playback_continue,
    subtitle_clock_us, subtitle_delay_us, audio_delay_frames, step_audio_skew,
    PlaybackContinue, RepeatMode, AspectMode, cycle_aspect, aspect_label, frame_aspect,
    fit_aspect, center_crop, display_ratio,     zoom_step, zoom_label, zoom_size, ZOOM_MIN_MILLI,
    ZOOM_MAX_MILLI, clamp_pan_px, pan_step_px, zoom_pan_rect, Bookmark, insert_bookmark, bookmark_step, shuffled_indices, order_step,
    is_hls_playlist, parse_playlist_text, expand_play_inputs, clamp_volume_milli,
    clamp_adjust_milli, adjust_pixel, flip_uv, RotateMode, cycle_rotate, rotate_label,
    rotate_pixel, rotate_size, ToneState, tone_step, EQ_BAND_COUNT, EQ_BAND_HZ, GraphicEqState,
    graphic_eq_step, eq_unity_gains, average_rgb_pixel, deinterlace_blend_rgb, cycle_output_device,
    format_play_stats, AudioChannelMode, cycle_audio_channel, audio_channel_label,
    apply_audio_channel, clamp_subtitle_margin, subtitle_margin_px, SnapshotFormat,
    cycle_snapshot_format, snapshot_format_ext, encode_png, snapshot_path_with_ext,
    snapshot_path_in_dir, format_snapshot_dir_osd, NETWORK_CACHE_DEFAULT_MS,
    NETWORK_CACHE_MIN_MS, NETWORK_CACHE_MAX_MS, clamp_network_cache_ms,
    format_network_cache_osd, format_hotkeys_help_osd,
    BitmapSubtitle, palette_rgba_pixel, pal8_to_rgba, blend_rgba_over_rgb, blit_bitmap_subtitle,
    active_bitmap_subtitle, parse_play_clock, EqPreset, EQ_PRESET_COUNT, eq_preset_label,
    cycle_eq_preset, eq_preset_db, eq_preset_gains, eq_db_to_milli, clamp_eq_milli,
    seek_step_us, SEEK_COARSE_US, SEEK_FINE_US, media_display_title, DeinterlaceMode,
    cycle_deinterlace, deinterlace_label, deinterlace_bob_rgb, apply_deinterlace_rgb,
    format_volume_osd, VOLUME_MAX_MILLI, VOLUME_STEP_MILLI, RATE_STEP_MILLI, volume_step_milli,
    rate_step_milli, format_play_clock, format_rate_osd, position_us_from_digit, media_fraction,
    media_us_from_fraction, FrameStep, frame_step_target_us, BALANCE_CENTER_MILLI,
    BALANCE_MIN_MILLI, BALANCE_MAX_MILLI, BALANCE_STEP_MILLI, clamp_balance_milli,
    balance_step_milli, apply_audio_balance, format_balance_osd, WIDTH_UNITY_MILLI,
    WIDTH_MIN_MILLI, WIDTH_MAX_MILLI, WIDTH_STEP_MILLI, clamp_width_milli, width_step_milli,
    apply_stereo_width, format_width_osd, compress_sample, apply_compressor, format_compressor_osd,
    CROSSFEED_MAX_MILLI, CROSSFEED_STEP_MILLI, clamp_crossfeed_milli, crossfeed_step_milli,
    apply_crossfeed, format_crossfeed_osd, clamp_subtitle_scale_milli,
    subtitle_scale_step_milli, subtitle_font_px, SUBTITLE_SCALE_MIN_MILLI, SUBTITLE_SCALE_MAX_MILLI,
    SUBTITLE_SCALE_UNITY_MILLI, SUBTITLE_SCALE_STEP_MILLI, cycle_eq_bypass, format_eq_bypass_osd,
    reset_av_delays, format_delay_osd, VOLUME_WHEEL_STEP_MILLI, volume_from_wheel, clamp_seek_us,
    format_ab_osd, format_repeat_osd, format_shuffle_osd, format_subtitle_scale_osd,
    format_jump_osd, format_window_title, format_window_title_with_position, PositionDisplay,
    cycle_position_display, remaining_media_us, format_position_osd, normalizer_peak_step,
    normalizer_gain_milli, apply_normalizer_sample, format_normalizer_osd, TONE_UNITY_MILLI,
    TONE_STEP_MILLI, tone_gain_step_milli,
    reset_tone_gains, apply_tone_frame, format_tone_osd, PlayRenderOptions, render_play_pixels,
    reset_video_adjust, adjust_step_milli, reset_zoom_pan, format_adjust_osd, format_zoom_osd,
    format_aspect_osd, format_crop_osd, format_deinterlace_osd, eq_band_step_milli,
    set_eq_gains_from_preset, format_eq_preset_osd, PAN_STEP_PX, format_pan_osd, initial_seek_us,
    gamma_channel, apply_gamma_pixel, format_audio_channel_osd, initial_stop_us,
    should_stop_playback, RATE_WHEEL_STEP_MILLI, rate_from_wheel, stop_playback_us,
    format_stop_osd, format_rotate_osd, format_flip_osd, seek_end_us, chapter_index,
    format_chapter_osd, format_pause_osd, clear_bookmarks, format_bookmark_osd,
    format_bookmarks_export, parse_bookmarks_export,
    format_playlist_osd, format_playlist_m3u, soft_clip_sample, PLAYBACK_PROTOCOLS,
    SLEEP_TIMER_STEPS_MIN, cycle_sleep_timer_min, sleep_deadline_secs, sleep_timer_fired,
    format_sleep_osd, FOV_MIN_MILLI, FOV_MAX_MILLI, FOV_DEFAULT_MILLI, FOV_STEP_MILLI,
    YAW_STEP_MILLI, PITCH_STEP_MILLI, PITCH_MIN_MILLI, PITCH_MAX_MILLI, clamp_yaw_milli,
    clamp_pitch_milli, clamp_fov_milli, yaw_step_milli, pitch_step_milli, fov_step_milli,
    ROLL_STEP_MILLI, clamp_roll_milli, roll_step_milli, format_spherical_osd,
    format_spherical_osd_ex, sample_equirect_pixel, project_equirect_view,
    project_equirect_view_ex, HdrTonemap,
    COLOR_TRC_SMPTE2084, COLOR_TRC_HLG, is_hdr_transfer, cycle_hdr_tonemap, hdr_tonemap_label,
    format_hdr_tonemap_osd, tonemap_channel, apply_hdr_tonemap_pixel, auto_hdr_tonemap,
    pq_eotf, hlg_eotf, expand_hdr_channel, PlayStereo3D, cycle_play_stereo3d, play_stereo3d_label,
    format_play_stereo3d_osd, apply_play_stereo3d, parse_hdr_tonemap, parse_play_stereo3d,
    parse_degrees_milli, format_media_info_osd, random_seek_us, cycle_integer_zoom,
    format_integer_zoom_osd, detect_equirect_aspect, fit_window_to_video, format_fit_window_osd,
    SUBTITLE_OPACITY_MIN_MILLI, SUBTITLE_OPACITY_MAX_MILLI, SUBTITLE_OPACITY_UNITY_MILLI,
    SUBTITLE_OPACITY_STEP_MILLI, clamp_subtitle_opacity_milli, subtitle_opacity_step_milli,
    format_subtitle_opacity_osd, subtitle_opacity_u8, format_track_osd, should_quit_at_end,
    SubtitlePosition, cycle_subtitle_position, subtitle_position_label,
    format_subtitle_position_osd, subtitle_block_top_y, OSD_TIMEOUT_DEFAULT_MS,
    OSD_TIMEOUT_MIN_MS, OSD_TIMEOUT_MAX_MS, clamp_osd_timeout_ms, osd_should_clear,
    MOUSE_HIDE_DEFAULT_MS, mouse_should_hide, audio_peak_milli, format_vu_osd,
    vu_bar_fills, cycle_rate_preset_milli, format_rate_preset_osd, MarqueePosition,
    cycle_marquee_position, marquee_position_label, format_marquee_osd, marquee_block_top_y,
    format_title_osd, DropFrameMode, cycle_drop_frame, drop_frame_label, format_drop_frame_osd,
    should_drop_late_frame, cycle_show_osd, format_show_osd, network_cache_delay_us,
    DisplayEffect, cycle_display_effect, display_effect_label, format_display_effect_osd,
    apply_display_effect_pixel, prefer_track_index, CONTROLS_AUTOHIDE_DEFAULT_MS,
    CONTROLS_AUTOHIDE_MIN_MS, CONTROLS_AUTOHIDE_MAX_MS, clamp_controls_autohide_ms,
    controls_should_hide, format_resume_positions, parse_resume_positions, resume_seek_us,
    HTTP_RECONNECT_DEFAULT, HTTP_RECONNECT_MAX, clamp_http_reconnect, format_http_reconnect_osd,
    http_should_reconnect, scaletempo_duration_us, HDR_NITS_DEFAULT, HDR_NITS_MIN, HDR_NITS_MAX,
    clamp_hdr_nits, format_hdr_nits_osd, scale_hdr_display_channel, CacheDomain,
    cycle_cache_domain, cache_domain_label, default_cache_ms, clamp_cache_ms, format_cache_osd,
    secondary_subtitle_delay_us, format_secondary_subtitle_delay_osd, snapshot_path_with_prefix,
    format_snapshot_prefix_osd, next_snapshot_index, EQ_PREAMP_DEFAULT_MILLI, EQ_PREAMP_MIN_MILLI,
    EQ_PREAMP_MAX_MILLI, EQ_PREAMP_STEP_MILLI, clamp_eq_preamp_milli, eq_preamp_step_milli,
    apply_eq_preamp_sample, format_eq_preamp_osd, SPATIALIZER_DEFAULT_MILLI, SPATIALIZER_MIN_MILLI,
    SPATIALIZER_MAX_MILLI, SPATIALIZER_STEP_MILLI, clamp_spatializer_milli, spatializer_step_milli,
    apply_spatializer, format_spatializer_osd, GAPLESS_THRESHOLD_DEFAULT_US, gapless_should_prefetch,
    format_gapless_osd, CROSSFADE_DEFAULT_MS, CROSSFADE_MIN_MS, CROSSFADE_MAX_MS, CROSSFADE_STEP_MS,
    clamp_crossfade_ms, crossfade_step_ms, crossfade_gain_pair, format_crossfade_osd,
    REPLAYGAIN_UNITY_MILLI, REPLAYGAIN_MIN_MILLI, REPLAYGAIN_MAX_MILLI, clamp_replaygain_milli,
    replaygain_milli_from_db_milli, apply_replaygain_sample, format_replaygain_osd,
    buffer_health_pct, format_buffer_health_osd, snap_seek_to_keyframe, format_keyframe_seek_osd,
    SubtitleColor, cycle_subtitle_color, subtitle_color_label, subtitle_color_rgba,
    format_subtitle_color_osd, prefer_forced_subtitle_index, format_forced_subtitle_osd,
    momentary_lufs_from_peak_milli, format_loudness_osd, spectrum_bar_fills, format_spectrum_osd,
    PlaylistSort, cycle_playlist_sort, playlist_sort_label, sort_playlist_paths,
    format_playlist_sort_osd, format_bookmark_label, RECENT_PLAY_MAX, push_recent_path,
    format_recent_osd, deinterlace_linear_rgb, deinterlace_mean_rgb, SeekJump, cycle_seek_jump,
    format_seek_jump_osd, seek_step_us_ex, VideoPostFx, cycle_video_post_fx, video_post_fx_label,
    format_video_post_fx_osd, apply_video_post_fx, downmix_surround_to_stereo, format_downmix_osd,
    format_scaletempo_osd, format_minimal_interface_osd, AUDIO_PITCH_UNITY_MILLI,
    AUDIO_PITCH_MIN_MILLI, AUDIO_PITCH_MAX_MILLI, AUDIO_PITCH_STEP_MILLI, clamp_audio_pitch_milli,
    audio_pitch_step_milli, apply_audio_pitch_sample_index, format_audio_pitch_osd,
    VisualizationMode, cycle_visualization, visualization_label,
    format_visualization_osd, scope_samples_u8, audio_bargraph_fills, format_bargraph_osd,
    apply_motion_blur_rgb, format_motion_blur_osd, apply_video_post_fx_with_prev,
    IMAGE_DURATION_DEFAULT_SECS, IMAGE_DURATION_MIN_SECS, IMAGE_DURATION_MAX_SECS,
    clamp_image_duration_secs, image_duration_us, format_image_duration_osd,
    ClosedCaptionChannel, cycle_closed_caption, closed_caption_label, format_closed_caption_osd,
    CropPixels, clamp_crop_pixels, crop_output_size, format_crop_pixels_osd, audio_desync_us,
    audio_desync_ms_from_us, format_audio_desync_osd, format_wallpaper_osd, prefer_program_index,
    format_program_osd, SubtitleEncoding, cycle_subtitle_encoding, subtitle_encoding_label,
    format_subtitle_encoding_osd, TELETEXT_PAGE_DEFAULT, TELETEXT_PAGE_MIN, TELETEXT_PAGE_MAX,
    clamp_teletext_page, teletext_page_step, format_teletext_osd, format_aspect_lock_osd,
    locked_window_size, format_snapshot_sequential_name, format_snapshot_sequential_osd,
    format_hw_decode_osd, format_media_fingerprint_osd, LogoPosition, cycle_logo_position,
    logo_position_label, clamp_logo_opacity_milli, format_logo_osd, logo_anchor_xy, mosaic_tile_rect,
    format_mosaic_osd, clamp_param_eq_milli, apply_param_eq_sample, format_param_eq_osd,
    clamp_amplifier_milli, apply_amplifier_sample, format_amplifier_osd, format_record_path,
    format_record_osd, format_http_auth_osd, ProxyMode, cycle_proxy_mode, proxy_mode_label,
    format_proxy_osd, prefer_stream_quality_index, format_stream_quality_osd,
    prefer_external_subtitle_path, format_external_subtitle_osd, SphericalProjection,
    cycle_spherical_projection, spherical_projection_label, format_spherical_projection_osd,
    dual_fisheye_to_equirect, sample_cubemap_pixel, project_little_planet, project_spherical_view,
    parse_spherical_projection,
    clamp_hdr_maxcll, format_hdr_metadata_osd, COLOR_PRIMARIES_BT709, COLOR_PRIMARIES_BT2020,
    color_primaries_label, format_color_primaries_osd, silence_skip_target_us,
    format_silence_skip_osd, cycle_video_track, format_video_track_osd,
    prefer_hearing_impaired_subtitle_index, format_hearing_impaired_osd, skip_marker_target_us,
    format_skip_marker_osd, filter_playlist_paths, format_playlist_filter_osd, FAVORITES_MAX,
    toggle_favorite, format_favorite_osd, format_remote_control_osd, REMOTE_CONTROL_DEFAULT_PORT,
    format_bitperfect_osd, should_toggle_fullscreen_on_click, scrub_preview_us,
    format_scrub_preview_osd, apply_night_mode_sample, format_night_mode_osd,
    frame_rate_milli_from_duration_us, format_frame_rate_osd, cycle_angle, format_angle_osd,
    seek_from_wheel, queue_insert, format_queue_osd, format_forced_only_osd,
    clamp_exclusive_latency_ms, format_exclusive_latency_osd, chapter_thumbnail_times,
    format_chapter_thumbs_osd, IPD_DEFAULT_MILLI, IPD_MIN_MILLI, IPD_MAX_MILLI, IPD_STEP_MILLI,
    clamp_ipd_milli, ipd_step_milli, format_ipd_osd, cardboard_eye_yaw_offset_milli, VrDisplayMode,
    cycle_vr_display, vr_display_label, format_vr_display_osd, AmbisonicMode, cycle_ambisonic,
    ambisonic_label, format_ambisonic_osd, parse_webvtt_timestamp, format_webvtt_timestamp,
    CastProtocol, cycle_cast_protocol, cast_protocol_label, format_cast_osd, media_library_entries,
    format_media_library_osd, format_chapter_art_osd, SmilClip, parse_smil_clip_line,
    format_smil_clip_osd, format_named_bookmark_osd, format_hdr_mastering_osd, hdr_gamut_warning,
    format_hdr_gamut_osd, cardboard_eye_rect, cardboard_view_yaw_milli, LyricLine, active_lyric_line,
    format_lyric_osd, ab_slot_store, ab_slot_load, format_ab_slot_osd,     format_play_stats_csv, DEBAND_DEFAULT_MILLI, DEBAND_MAX_MILLI, clamp_deband_milli,
    apply_deband_pixel, format_deband_osd, parse_hdr_maxcll_maxfall, format_dolby_vision_osd,
    format_snapshot_with_osd, image_loop_remaining, format_image_loop_osd, EdlClip, parse_edl_line,
    format_edl_clip_osd, pip_rect, format_pip_osd, thumbnail_seek_us, format_thumbnail_seek_osd,
    seek_from_drag_px, format_drag_seek_osd, CropBoxMilli, clamp_crop_box_milli, crop_box_to_pixels,
    format_crop_box_osd, format_horizon_lock_osd, locked_pitch_milli, estimate_frame_peak_milli,
    suggest_hdr_nits_from_peak, format_hdr_peak_osd, format_stream_rendition_osd,
    SphericalStereoLayout, cycle_spherical_stereo, spherical_stereo_label, format_spherical_stereo_osd,
    spherical_stereo_uv_rect, recenter_spherical_view, format_recenter_osd, compass_heading_deg,
    format_compass_osd, barrel_distort_uv_milli, format_barrel_osd, TONEMAP_STRENGTH_DEFAULT_MILLI,
    clamp_tonemap_strength_milli, blend_tonemap_channel, format_tonemap_strength_osd,
    apply_hdr_highlight_desat_pixel, format_hdr_highlight_desat_osd, parse_hdr_mastering_nits,
    COLOR_TEMP_DAYLIGHT_K, clamp_color_temp_kelvin, apply_white_balance_pixel, format_color_temp_osd,
    detect_letterbox_bars, format_letterbox_osd, playlist_edge_fade_gain_milli,
    format_playlist_fade_osd, audio_duck_gain_milli, format_audio_duck_osd, waveform_column_fills,
    format_waveform_osd, format_hdr10_plus_osd, hlg_ootf_channel, apply_hlg_ootf_pixel,
    format_hlg_ootf_osd, FOV_PRESET_MILLI, cycle_fov_preset_milli, format_fov_preset_osd,
    parse_spherical_stereo, format_hdr_headroom_osd, format_timecode_osd, format_chapter_list_export,
    prefer_album_art_path, format_album_art_osd, apply_box_denoise_pixel, format_box_denoise_osd,
    apply_dialogue_enhance_sample, format_dialogue_enhance_osd, vectorscope_quadrant_counts,
    format_vectorscope_osd, yaw_from_swipe_px, pitch_from_swipe_px, gyro_look_delta_milli,
    format_gyro_osd, vr_vignette_gain_milli, apply_vr_vignette_pixel, format_vr_vignette_osd,
    eac_face_uv_from_dir, format_eac_face_osd, apply_hdr_black_lift_channel, format_hdr_black_lift_osd,
    apply_unsharp_pixel, format_unsharp_osd, format_icecast_metadata_osd, MediaKeyAction,
    media_key_label, format_media_key_osd, parse_m3u_extinf_title, thumbnail_grid_rect,
    format_thumbnail_grid_osd, format_clipboard_snapshot_osd, timeshift_lag_us, format_live_edge_osd,
    instant_replay_us, format_instant_replay_osd, phase_correlation_milli, format_phase_correlation_osd,
    true_peak_milli, format_true_peak_osd, format_atmos_layout_osd, format_content_rating_osd,
    ass_override_margin_px, format_ass_override_osd, network_bandwidth_bps, format_network_bandwidth_osd,
    multi_room_sync_target_us, format_watch_party_osd, hdr_sdr_ratio_milli, format_hdr_sdr_ratio_osd,
    accelerometer_horizon_pitch_milli, format_auto_horizon_osd, sample_eac_pixel,
    chromatic_aberration_uv_milli, format_chromatic_aberration_osd, AmbisonicChannelOrder,
    cycle_ambisonic_order, ambisonic_order_label, format_ambisonic_order_osd,
    apply_soft_limiter_sample, format_soft_limiter_osd, apply_echo_sample, format_echo_osd,
    apply_lowpass_1pole, apply_highpass_1pole, format_tone_filter_osd,
    short_term_lufs_from_peaks, loudness_range_l_milli, format_loudness_range_osd,
    anaglyph_dubois, format_anaglyph_dubois_osd, apply_delogo_rect, format_delogo_osd,
    prefer_opensubtitles_path, filter_playlist_by_extension, format_smart_playlist_osd,
    detect_bpm_from_onset_gaps_ms, format_bpm_osd, haas_delay_samples, format_haas_osd,
    atempo_duration_us, format_atempo_osd, apply_chorus_sample, format_chorus_osd,
    apply_reverb_sample, format_reverb_osd, format_ass_force_style, format_ass_force_style_osd,
    bt2446_tonemap_channel, apply_bt2446_tonemap_pixel, format_bt2446_osd, SphericalHotspot,
    spherical_hotspot_hit, format_spherical_hotspot_osd, parse_webvtt_region_id,
    thumbnail_cache_key, integrated_lufs_from_short_term, format_integrated_lufs_osd,
    project_panini_view, project_cylindrical_view, brown_conrady_uv_milli, format_lens_calibration_osd,
    ictcp_intensity_milli, format_ictcp_osd, prefer_abr_rendition_index, format_abr_osd,
    storyboard_tile_index, format_storyboard_osd, watch_progress_milli, format_continue_watching_osd,
    up_next_should_start, format_up_next_osd, format_scrobble_line, gaze_dwell_triggered,
    format_gaze_dwell_osd, controller_ray_hit, parse_ttml_clock, clamp_dvr_playhead_us,
    format_dvr_window_osd, pq_oetf, hlg_oetf, maxrgb_tonemap_pixel, format_maxrgb_osd,
    project_mercator_view, dual_fisheye_tb_to_equirect, clamp_live_latency_ms, format_live_latency_osd,
    buffer_health_ratio_milli, format_epg_program_osd, format_cea708_service_osd,
    sample_octahedral_pixel, project_octahedral_view, project_equisolid_view,
    PAPER_WHITE_DEFAULT_NITS, clamp_paper_white_nits, format_paper_white_osd,
    scale_sdr_overlay_to_paper_white, format_st2094_l1_osd, StereoPacking, cycle_stereo_packing,
    stereo_packing_label, format_stereo_packing_osd, row_interleaved_eye_pixel,
    format_row_interleaved_osd, passthrough_blend_milli, format_passthrough_osd,
    skip_segment_target_us, format_skip_segment_osd, format_binge_mode_osd,
};
pub use filter::{PadRect, RotateAngle, TransposeMode, overlay_cli_vf, subtitles_cli_vf, xfade_cli_vf};
pub use xfade::{xfade_filter_complex, xfade_video};
use ffi::*;
#[cfg(feature = "cuda-hw")]
pub use hw_cuda::{HwFilterOptions, HwFilterStats, hw_filter};
pub use lossless::{
    CropRect, EncoderSettings, LosslessStats, LosslessTransform, OverlaySpec, ScaleSize, XfadeSpec,
    crop_lossless, transcode, transcode_lossless,
};
pub use pcm::{PcmTrimStats, trim_pcm};
pub use subtitle::{
    SubtitleCodec, SubtitleConvertOptions, SubtitleConvertStats, burn_subtitles, convert_subtitles,
    overlay_video,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    path::{Path, PathBuf},
    ptr, slice,
};
pub type Result<T> = std::result::Result<T, String>;
const NOPTS: i64 = i64::MIN;
const EOF: i32 = -541478725;
const MAX_STREAMS: usize = 64;
fn cstring(text: &str) -> Result<CString> {
    CString::new(text).map_err(|_| "embedded NUL".into())
}
fn path_string(path: &Path) -> Result<CString> {
    cstring(path.to_str().ok_or("path must be UTF-8")?)
}
/// Prefer hard link then unlink temp; fall back to rename when hard links fail.
fn publish_file(temporary: &Path, destination: &Path) -> Result<()> {
    match std::fs::hard_link(temporary, destination) {
        Ok(()) => {
            let _ = std::fs::remove_file(temporary);
            Ok(())
        }
        Err(hard_link_err) => std::fs::rename(temporary, destination).map_err(|rename_err| {
            format!("publish output (hard_link: {hard_link_err}; rename: {rename_err})")
        }),
    }
}
fn check(code: i32, operation: &str) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    let mut buffer = [0i8; 256];
    // SAFETY: The error formatter writes at most the supplied buffer length.
    unsafe {
        av_strerror(code, buffer.as_mut_ptr(), buffer.len());
    }
    // SAFETY: buffer starts zeroed; av_strerror always terminates within its bound.
    let detail = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy();
    Err(format!("{operation}: {detail} ({code})"))
}
fn string(pointer: *const std::ffi::c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: Internal callers pass live FFmpeg-owned, NUL-terminated strings.
    unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() }
}
struct Input(*mut AVFormatContext);
impl Input {
    fn open(path: &Path) -> Result<Self> {
        Self::open_with_stream_info(path, true)
    }
    /// MP4-family headers carry complete codec parameters and stream timing.
    /// Decode-only/filter paths can avoid a redundant packet probe at startup.
    fn open_fast(path: &Path) -> Result<Self> {
        let header_complete = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                value.eq_ignore_ascii_case("mp4")
                    || value.eq_ignore_ascii_case("mov")
                    || value.eq_ignore_ascii_case("m4v")
                    || value.eq_ignore_ascii_case("m4a")
                    || value.eq_ignore_ascii_case("srt")
            });
        Self::open_with_stream_info(path, !header_complete)
    }
    /// Local file, or a playback URL (`http`, `https`, `rtsp`, `rtmp`, `udp`, …).
    fn open_playback(location: &str) -> Result<Self> {
        if is_playback_url(location) {
            return Self::open_url(location);
        }
        let path = Path::new(location);
        if !path.is_file() {
            return Err(
                "media input must be an existing local file or a supported playback URL".into(),
            );
        }
        Self::open_fast(path)
    }
    fn open_url(location: &str) -> Result<Self> {
        let location = cstring(location)?;
        let mut input = Self(unsafe { avformat_alloc_context() });
        if input.0.is_null() {
            return Err("input context allocation failed".into());
        }
        unsafe {
            (*input.0).max_streams = MAX_STREAMS as i32;
            (*input.0).probesize = 5 * 1024 * 1024;
            (*input.0).max_analyze_duration = 5_000_000;
            let mut options = ptr::null_mut();
            let whitelist = cstring(play::PLAYBACK_PROTOCOLS)?;
            let code = av_dict_set(
                &mut options,
                c"protocol_whitelist".as_ptr(),
                whitelist.as_ptr(),
                0,
            );
            if code < 0 {
                av_dict_free(&mut options);
                check(code, "set playback protocol policy")?;
            }
            let code = avformat_open_input(
                &mut input.0,
                location.as_ptr(),
                ptr::null(),
                &mut options,
            );
            av_dict_free(&mut options);
            check(code, "open input")?;
            check(
                avformat_find_stream_info(input.0, ptr::null_mut()),
                "read stream information",
            )?;
        }
        if unsafe { (*input.0).nb_streams as usize } > MAX_STREAMS {
            return Err("too many streams; maximum 64".into());
        }
        Ok(input)
    }
    fn chapter_starts_us(&self, origin_us: i64) -> Vec<i64> {
        unsafe {
            let count = (*self.0).nb_chapters as usize;
            let mut starts = Vec::with_capacity(count.min(256));
            for index in 0..count.min(256) {
                let chapter = &*(*(*self.0).chapters.add(index));
                if chapter.time_base.num <= 0 || chapter.time_base.den <= 0 || chapter.start < 0 {
                    continue;
                }
                let absolute = av_rescale_q(
                    chapter.start,
                    chapter.time_base,
                    AVRational {
                        num: 1,
                        den: 1_000_000,
                    },
                );
                starts.push(absolute.saturating_sub(origin_us).max(0));
            }
            starts.sort_unstable();
            starts.dedup();
            starts
        }
    }
    fn open_with_stream_info(path: &Path, find_stream_info: bool) -> Result<Self> {
        if !path.is_file() {
            return Err("media input must be an existing local file".into());
        }
        let path = path_string(path)?;
        // SAFETY: Fresh context, immediately guarded. FFmpeg owns/frees its fields.
        let mut input = Self(unsafe { avformat_alloc_context() });
        if input.0.is_null() {
            return Err("input context allocation failed".into());
        }
        // SAFETY: Context and option dictionary are uniquely owned; strings live
        // for the calls. The dictionary is released on success and failure.
        unsafe {
            (*input.0).max_streams = MAX_STREAMS as i32;
            (*input.0).probesize = 5 * 1024 * 1024;
            (*input.0).max_analyze_duration = 5_000_000;
            let mut options = ptr::null_mut();
            check(
                av_dict_set(
                    &mut options,
                    c"protocol_whitelist".as_ptr(),
                    c"file".as_ptr(),
                    0,
                ),
                "set local input policy",
            )?;
            if input_policy::active() {
                let code = av_dict_set(&mut options, c"format_whitelist".as_ptr(),
                    c"mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,wav,mp3,flac,ogg,aac,avi,mpeg,mpegts,asf,flv,srt,yuv4mpegpipe,png_pipe,jpeg_pipe".as_ptr(), 0);
                if code < 0 {
                    av_dict_free(&mut options);
                    check(code, "set standalone input policy")?;
                }
            }
            let code = avformat_open_input(&mut input.0, path.as_ptr(), ptr::null(), &mut options);
            av_dict_free(&mut options);
            check(code, "open input")?;
        }
        // SAFETY: Successful open creates a live input context, owned by this guard.
        if find_stream_info {
            check(
                unsafe { avformat_find_stream_info(input.0, ptr::null_mut()) },
                "read stream information",
            )?;
        }
        // SAFETY: The live context owns its stream table until Input is dropped.
        if unsafe { (*input.0).nb_streams as usize } > MAX_STREAMS {
            return Err("too many streams; maximum 64".into());
        }
        Ok(input)
    }
    fn streams(&self) -> &[*mut AVStream] {
        // SAFETY: Input owns the stream pointer array for the duration of this borrow.
        unsafe {
            if (*self.0).nb_streams == 0 {
                &[]
            } else {
                slice::from_raw_parts((*self.0).streams, (*self.0).nb_streams as usize)
            }
        }
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        // SAFETY: This guard uniquely owns the context returned by open_input.
        unsafe {
            avformat_close_input(&mut self.0);
        }
    }
}
struct Packet(*mut AVPacket);
impl Packet {
    fn new() -> Result<Self> {
        // SAFETY: FFmpeg allocates a fresh empty packet; null is checked.
        let p = unsafe { av_packet_alloc() };
        if p.is_null() {
            Err("packet allocation failed".into())
        } else {
            Ok(Self(p))
        }
    }
    fn read(&mut self, input: &mut Input) -> Result<bool> {
        // SAFETY: Both resources are exclusively borrowed and valid. Unref permits reuse.
        unsafe {
            av_packet_unref(self.0);
            let code = av_read_frame(input.0, self.0);
            if code == EOF {
                Ok(false)
            } else {
                check(code, "read packet")?;
                Ok(true)
            }
        }
    }
}
impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: Unique ownership; av_packet_free releases payload references as well.
        unsafe {
            av_packet_free(&mut self.0);
        }
    }
}
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StreamInfo {
    pub index: usize,
    pub media_type: String,
    pub codec: String,
    pub time_base: [i32; 2],
    pub start: Option<i64>,
    pub duration: Option<i64>,
    pub bit_rate: Option<i64>,
    pub average_frame_rate: [i32; 2],
    pub profile: Option<String>,
    pub level: Option<i32>,
    pub disposition: i32,
    pub metadata: BTreeMap<String, String>,
    pub width: i32,
    pub height: i32,
    pub pixel_format: i32,
    pub sample_rate: i32,
    pub channels: i32,
    pub video_delay: i32,
    pub extradata_bytes: usize,
}
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChapterInfo {
    pub id: i64,
    pub time_base: [i32; 2],
    pub start: i64,
    pub end: i64,
    pub metadata: BTreeMap<String, String>,
}
#[derive(Serialize, Debug)]
pub struct MediaInfo {
    pub path: PathBuf,
    pub format: String,
    pub start_us: Option<i64>,
    pub duration_us: Option<i64>,
    pub bit_rate: Option<i64>,
    pub metadata: BTreeMap<String, String>,
    pub chapters: Vec<ChapterInfo>,
    pub streams: Vec<StreamInfo>,
}
unsafe fn dictionary(dictionary: *mut AVDictionary) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    let mut entry = ptr::null();
    loop {
        entry = unsafe {
            av_dict_get(
                dictionary,
                c"".as_ptr(),
                entry,
                AV_DICT_IGNORE_SUFFIX as i32,
            )
        };
        if entry.is_null() {
            break;
        }
        unsafe {
            values.insert(string((*entry).key), string((*entry).value));
        }
    }
    values
}
pub fn probe(path: &Path) -> Result<MediaInfo> {
    let input = Input::open(path)?;
    // SAFETY: Input owns all contexts, stream parameters and strings for this block.
    unsafe {
        let mut streams = Vec::new();
        for (index, stream) in input.streams().iter().enumerate() {
            let s = &**stream;
            let p = &*s.codecpar;
            streams.push(StreamInfo {
                index,
                media_type: string(av_get_media_type_string(p.codec_type)),
                codec: string(avcodec_get_name(p.codec_id)),
                time_base: [s.time_base.num, s.time_base.den],
                start: (s.start_time != NOPTS).then_some(s.start_time),
                duration: (s.duration != NOPTS).then_some(s.duration),
                bit_rate: (p.bit_rate > 0).then_some(p.bit_rate),
                average_frame_rate: [s.avg_frame_rate.num, s.avg_frame_rate.den],
                profile: {
                    let name = avcodec_profile_name(p.codec_id, p.profile);
                    (!name.is_null()).then(|| string(name))
                },
                level: (p.level >= 0).then_some(p.level),
                disposition: s.disposition,
                metadata: dictionary(s.metadata),
                width: p.width,
                height: p.height,
                pixel_format: p.format,
                sample_rate: p.sample_rate,
                channels: p.ch_layout.nb_channels,
                video_delay: p.video_delay,
                extradata_bytes: p.extradata_size.max(0) as usize,
            });
        }
        let mut chapters = Vec::with_capacity((*input.0).nb_chapters as usize);
        for index in 0..(*input.0).nb_chapters as usize {
            let chapter = &**(*input.0).chapters.add(index);
            chapters.push(ChapterInfo {
                id: chapter.id,
                time_base: [chapter.time_base.num, chapter.time_base.den],
                start: chapter.start,
                end: chapter.end,
                metadata: dictionary(chapter.metadata),
            });
        }
        Ok(MediaInfo {
            path: path.into(),
            format: string((*(*input.0).iformat).name),
            start_us: ((*input.0).start_time != NOPTS).then_some((*input.0).start_time),
            duration_us: ((*input.0).duration != NOPTS).then_some((*input.0).duration),
            bit_rate: ((*input.0).bit_rate > 0).then_some((*input.0).bit_rate),
            metadata: dictionary((*input.0).metadata),
            chapters,
            streams,
        })
    }
}
#[derive(Serialize)]
pub struct Capabilities {
    pub library_version: String,
    pub demuxers: Vec<String>,
    pub muxers: Vec<String>,
    pub decoders: Vec<String>,
    pub encoders: Vec<String>,
    pub filters: Vec<String>,
}
pub fn capabilities() -> Capabilities {
    // SAFETY: Iterators use library-owned static descriptors, each with its own opaque cursor.
    unsafe {
        let mut result = Capabilities {
            library_version: string(av_version_info()),
            demuxers: vec![],
            muxers: vec![],
            decoders: vec![],
            encoders: vec![],
            filters: vec![],
        };
        let mut opaque = ptr::null_mut();
        loop {
            let p = av_demuxer_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            result.demuxers.push(string((*p).name));
        }
        opaque = ptr::null_mut();
        loop {
            let p = av_muxer_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            result.muxers.push(string((*p).name));
        }
        opaque = ptr::null_mut();
        loop {
            let p = av_codec_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            if av_codec_is_decoder(p) != 0 {
                result.decoders.push(string((*p).name));
            }
            if av_codec_is_encoder(p) != 0 {
                result.encoders.push(string((*p).name));
            }
        }
        opaque = ptr::null_mut();
        loop {
            let p = av_filter_iterate(&mut opaque);
            if p.is_null() {
                break;
            }
            result.filters.push(string((*p).name));
        }
        result
    }
}

#[derive(Clone, Debug)]
pub struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl CancelFlag {
    pub fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
impl Default for CancelFlag {
    fn default() -> Self {
        Self::new()
    }
}

/// Cooperative progress sample between packets (and a final `done` event).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgressEvent {
    pub packets: u64,
    pub payload_bytes: u64,
    pub done: bool,
}

#[derive(Clone)]
pub struct ProgressHook(std::sync::Arc<dyn Fn(ProgressEvent) + Send + Sync>);
impl ProgressHook {
    pub fn new<F>(hook: F) -> Self
    where
        F: Fn(ProgressEvent) + Send + Sync + 'static,
    {
        Self(std::sync::Arc::new(hook))
    }
    pub fn emit(&self, event: ProgressEvent) {
        (self.0)(event);
    }
}
impl std::fmt::Debug for ProgressHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressHook(..)")
    }
}

#[derive(Clone, Debug)]
pub struct CopyOptions {
    /// Empty selects every stream; otherwise indices are preserved in this order.
    pub streams: Vec<usize>,
    pub max_packet_bytes: usize,
    /// Optional hard stop after this many muxed packets (native budget slice).
    pub max_packets: Option<u64>,
    /// Optional admission limit on estimated decoder DPB + Fvid scratch bytes.
    /// Not a promise of peak OS RSS from libav alone.
    pub max_controlled_bytes: Option<usize>,
    /// Optional process RSS limit probed every 256 packets (and at start when set).
    pub max_rss_bytes: Option<u64>,
    /// Cooperative cancellation checked between packets.
    pub cancel: Option<CancelFlag>,
    /// Optional progress hook sampled every 256 packets and once with `done=true`.
    pub progress: Option<ProgressHook>,
    /// Container-level tags to set after copying source metadata (`KEY=VALUE`).
    pub metadata_set: Vec<(String, String)>,
    /// Container-level tag keys to delete after the copy.
    pub metadata_delete: Vec<String>,
    /// Source-stream tags to set: `(stream_index, key, value)`.
    pub stream_metadata_set: Vec<(usize, String, String)>,
    /// Source-stream tags to delete: `(stream_index, key)`.
    pub stream_metadata_delete: Vec<(usize, String)>,
}
impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            streams: vec![],
            max_packet_bytes: 64 * 1024 * 1024,
            max_packets: None,
            max_controlled_bytes: None,
            max_rss_bytes: None,
            cancel: None,
            progress: None,
            metadata_set: vec![],
            metadata_delete: vec![],
            stream_metadata_set: vec![],
            stream_metadata_delete: vec![],
        }
    }
}

pub(crate) use budget::{check_budget, check_budget_with_bytes, emit_progress_done};

fn validate_metadata_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 128 || key.contains('=') || key.contains('\0') {
        return Err("metadata key must be 1..=128 bytes without '=' or NUL".into());
    }
    Ok(())
}

fn validate_metadata_value(value: &str) -> Result<()> {
    if value.len() > 4096 || value.contains('\0') {
        return Err("metadata value must be <=4096 bytes without NUL".into());
    }
    Ok(())
}

unsafe fn dict_set(dict: *mut *mut AVDictionary, key: &str, value: Option<&str>) -> Result<()> {
    validate_metadata_key(key)?;
    if let Some(value) = value {
        validate_metadata_value(value)?;
    }
    let key = cstring(key)?;
    let owned;
    let value_ptr = if let Some(value) = value {
        owned = cstring(value)?;
        owned.as_ptr()
    } else {
        ptr::null()
    };
    check(
        unsafe { av_dict_set(dict, key.as_ptr(), value_ptr, 0) },
        "set metadata entry",
    )
}

unsafe fn apply_container_metadata(
    dict: *mut *mut AVDictionary,
    options: &CopyOptions,
) -> Result<()> {
    if options.metadata_set.len() + options.metadata_delete.len() > 64 {
        return Err("at most 64 container metadata mutations".into());
    }
    for key in &options.metadata_delete {
        unsafe { dict_set(dict, key, None)? };
    }
    for (key, value) in &options.metadata_set {
        unsafe { dict_set(dict, key, Some(value))? };
    }
    Ok(())
}

unsafe fn apply_stream_metadata(
    dict: *mut *mut AVDictionary,
    source_index: usize,
    options: &CopyOptions,
) -> Result<()> {
    let deletes = options
        .stream_metadata_delete
        .iter()
        .filter(|(i, _)| *i == source_index);
    let sets = options
        .stream_metadata_set
        .iter()
        .filter(|(i, _, _)| *i == source_index);
    if deletes.clone().count() + sets.clone().count() > 64 {
        return Err("at most 64 stream metadata mutations per stream".into());
    }
    for (_, key) in deletes {
        unsafe { dict_set(dict, key, None)? };
    }
    for (_, key, value) in sets {
        unsafe { dict_set(dict, key, Some(value))? };
    }
    Ok(())
}
#[derive(Serialize, Default, Debug)]
pub struct CopyStats {
    pub packets: u64,
    pub payload_bytes: u64,
    pub segments: usize,
    pub backend: &'static str,
    pub fvid_payload_copies: u64,
}
struct Output {
    context: *mut AVFormatContext,
    temporary: Option<PathBuf>,
    destination: PathBuf,
    strict_timing: bool,
    completed: bool,
    /// When false, use `av_write_frame` (single-stream encode; avoids interleave buffer).
    interleave: bool,
}
impl Output {
    fn new(destination: &Path, source: &Input, selected: &[usize]) -> Result<Self> {
        Self::with_video_mode(destination, source, selected, &[], true, None)
    }
    /// Streamcopy edits already validate every packet before publication and are
    /// benchmarked against FFmpeg's direct output path. Avoid the staged hard-link.
    fn new_direct(
        destination: &Path,
        source: &Input,
        selected: &[usize],
        options: Option<&CopyOptions>,
    ) -> Result<Self> {
        Self::with_video_mode(destination, source, selected, &[], false, options)
    }
    fn with_video(
        destination: &Path,
        source: &Input,
        selected: &[usize],
        video: Option<(usize, *const AVCodecParameters, AVRational)>,
    ) -> Result<Self> {
        let overrides = video.map(|v| vec![v]).unwrap_or_default();
        Self::with_overrides(destination, source, selected, &overrides, true, None)
    }
    fn with_video_direct(
        destination: &Path,
        source: &Input,
        selected: &[usize],
        video: Option<(usize, *const AVCodecParameters, AVRational)>,
    ) -> Result<Self> {
        let overrides = video.map(|v| vec![v]).unwrap_or_default();
        Self::with_overrides(destination, source, selected, &overrides, false, None)
    }
    fn with_overrides(
        destination: &Path,
        source: &Input,
        selected: &[usize],
        overrides: &[(usize, *const AVCodecParameters, AVRational)],
        staged: bool,
        options: Option<&CopyOptions>,
    ) -> Result<Self> {
        Self::with_video_mode(destination, source, selected, overrides, staged, options)
    }
    fn with_video_mode(
        destination: &Path,
        source: &Input,
        selected: &[usize],
        overrides: &[(usize, *const AVCodecParameters, AVRational)],
        staged: bool,
        options: Option<&CopyOptions>,
    ) -> Result<Self> {
        if destination.symlink_metadata().is_ok() {
            return Err("output already exists".into());
        }
        let directory = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = if staged {
            let mut temporary = None;
            for attempt in 0..100 {
                let path =
                    directory.join(format!(".fvid-media-{}-{attempt}.tmp", std::process::id()));
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(_) => {
                        temporary = Some(path);
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(e.to_string()),
                }
            }
            Some(temporary.ok_or("cannot reserve temporary output")?)
        } else {
            None
        };
        let mut out = Self {
            context: ptr::null_mut(),
            temporary,
            destination: destination.into(),
            strict_timing: false,
            completed: false,
            interleave: true,
        };
        let target = path_string(destination)?;
        let write_path = path_string(out.temporary.as_deref().unwrap_or(destination))?;
        // SAFETY: All descriptors and owned input parameters remain live; Output
        // takes ownership of the newly allocated format context and its AVIO.
        unsafe {
            check(
                avformat_alloc_output_context2(
                    &mut out.context,
                    ptr::null(),
                    ptr::null(),
                    target.as_ptr(),
                ),
                "select output container",
            )?;
            if out.context.is_null() {
                return Err("output context allocation failed".into());
            }
            if (*(*out.context).oformat).flags & AVFMT_NOFILE as i32 != 0 {
                return Err("output must be a file muxer".into());
            }
            (*out.context).max_interleave_delta = 1_000_000;
            (*out.context).avoid_negative_ts = AVFMT_AVOID_NEG_TS_DISABLED as i32;
            check(
                av_dict_copy(&mut (*out.context).metadata, (*source.0).metadata, 0),
                "copy container metadata",
            )?;
            if let Some(options) = options {
                apply_container_metadata(&mut (*out.context).metadata, options)?;
            }
            let count = (*source.0).nb_chapters as usize;
            if count > 4096 {
                return Err("too many chapters".into());
            }
            if count > 0 {
                (*out.context).chapters =
                    av_calloc(count, std::mem::size_of::<*mut AVChapter>()).cast();
                if (*out.context).chapters.is_null() {
                    return Err("chapter table allocation failed".into());
                }
                for index in 0..count {
                    let src = *(*source.0).chapters.add(index);
                    let dst: *mut AVChapter = av_mallocz(std::mem::size_of::<AVChapter>()).cast();
                    if dst.is_null() {
                        return Err("chapter allocation failed".into());
                    }
                    *(*out.context).chapters.add(index) = dst;
                    (*out.context).nb_chapters += 1;
                    (*dst).id = (*src).id;
                    (*dst).time_base = (*src).time_base;
                    (*dst).start = (*src).start;
                    (*dst).end = (*src).end;
                    check(
                        av_dict_copy(&mut (*dst).metadata, (*src).metadata, 0),
                        "copy chapter metadata",
                    )?;
                }
            }
            for &index in selected {
                let src = &*source.streams()[index];
                let dst = avformat_new_stream(out.context, ptr::null());
                if dst.is_null() {
                    return Err("output stream allocation failed".into());
                }
                let override_stream = overrides.iter().find(|(i, _, _)| *i == index);
                let parameters = override_stream.map_or(src.codecpar.cast_const(), |(_, p, _)| *p);
                check(
                    avcodec_parameters_copy((*dst).codecpar, parameters),
                    "copy codec parameters",
                )?;
                (*(*dst).codecpar).codec_tag = 0;
                (*dst).time_base = override_stream.map_or(src.time_base, |(_, _, tb)| *tb);
                (*dst).avg_frame_rate = src.avg_frame_rate;
                (*dst).sample_aspect_ratio = src.sample_aspect_ratio;
                (*dst).disposition = src.disposition;
                check(
                    av_dict_copy(&mut (*dst).metadata, src.metadata, 0),
                    "copy stream metadata",
                )?;
                if let Some(options) = options {
                    apply_stream_metadata(&mut (*dst).metadata, index, options)?;
                }
            }
            check(
                avio_open(
                    &mut (*out.context).pb,
                    write_path.as_ptr(),
                    AVIO_FLAG_WRITE as i32,
                ),
                "open output",
            )?;
            check(
                avformat_write_header(out.context, ptr::null_mut()),
                "write container header",
            )?;
        }
        Ok(out)
    }
    fn write(&mut self, packet: &mut Packet, index: usize, time_base: AVRational) -> Result<()> {
        // SAFETY: Stream index is from the checked selection map. Muxing consumes
        // the packet reference, not a Rust-owned payload copy; Packet remains valid/empty.
        unsafe {
            let stream = *(*self.context).streams.add(index);
            let target = (*stream).time_base;
            let same_time_base = time_base.num == target.num && time_base.den == target.den;
            if self.strict_timing && !same_time_base {
                if time_base.num <= 0 || time_base.den <= 0 || target.num <= 0 || target.den <= 0 {
                    return Err("invalid mux time base".into());
                }
                let denominator = i128::from(time_base.den) * i128::from(target.num);
                for value in [(*packet.0).pts, (*packet.0).dts, (*packet.0).duration] {
                    let numerator =
                        i128::from(value) * i128::from(time_base.num) * i128::from(target.den);
                    if numerator % denominator != 0 {
                        return Err("output container cannot represent exact packet timing".into());
                    }
                    i64::try_from(numerator / denominator)
                        .map_err(|_| "rescaled timestamp overflow")?;
                }
            }
            (*packet.0).stream_index = index as i32;
            if !same_time_base {
                av_packet_rescale_ts(packet.0, time_base, target);
            }
            (*packet.0).pos = -1;
            let code = if self.interleave {
                av_interleaved_write_frame(self.context, packet.0)
            } else {
                av_write_frame(self.context, packet.0)
            };
            check(code, "mux packet")
        }
    }
    /// Clip the stream's declared presentation duration (edit-list / mvhd path).
    /// Used when closed-GOP post-roll packets extend past the requested trim end.
    fn set_stream_duration(
        &mut self,
        index: usize,
        duration: i64,
        time_base: AVRational,
    ) -> Result<()> {
        if duration < 0 {
            return Err("negative presentation duration".into());
        }
        // SAFETY: Output owns the format context; index is a mapped output stream.
        unsafe {
            if index >= (*self.context).nb_streams as usize {
                return Err("stream duration index out of range".into());
            }
            let stream = *(*self.context).streams.add(index);
            let target = (*stream).time_base;
            let same = time_base.num == target.num && time_base.den == target.den;
            let scaled = if same {
                duration
            } else {
                if time_base.num <= 0 || time_base.den <= 0 || target.num <= 0 || target.den <= 0 {
                    return Err("invalid mux time base".into());
                }
                let denominator = i128::from(time_base.den) * i128::from(target.num);
                let numerator =
                    i128::from(duration) * i128::from(time_base.num) * i128::from(target.den);
                if numerator % denominator != 0 {
                    return Err("output container cannot represent exact presentation duration".into());
                }
                i64::try_from(numerator / denominator).map_err(|_| "duration overflow")?
            };
            (*stream).duration = scaled;
        }
        Ok(())
    }
    fn without_interleave(mut self) -> Self {
        self.interleave = false;
        self
    }
    fn finish(mut self) -> Result<()> {
        // SAFETY: The live output is uniquely owned; close before atomic publication.
        unsafe {
            check(av_write_trailer(self.context), "write trailer")?;
            check(avio_closep(&mut (*self.context).pb), "close output")?;
        }
        if let Some(temporary) = &self.temporary {
            publish_file(temporary, &self.destination)?;
        }
        self.completed = true;
        // Drop removes a leftover temporary name (if any) and frees the context.
        Ok(())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: Output uniquely owns its format context and any remaining AVIO.
        unsafe {
            if !self.context.is_null() {
                if !(*self.context).pb.is_null() {
                    avio_closep(&mut (*self.context).pb);
                }
                avformat_free_context(self.context);
            }
        }
        if let Some(temporary) = &self.temporary {
            let _ = std::fs::remove_file(temporary);
        } else if !self.completed {
            let _ = std::fs::remove_file(&self.destination);
        }
    }
}
pub(crate) fn selection(input: &Input, options: &CopyOptions) -> Result<Vec<usize>> {
    let selected = if options.streams.is_empty() {
        (0..input.streams().len()).collect()
    } else {
        options.streams.clone()
    };
    if selected.is_empty() {
        return Err("input has no streams".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for &index in &selected {
        if index >= input.streams().len() || !seen.insert(index) {
            return Err("stream index missing or duplicated".into());
        }
    }
    Ok(selected)
}
fn packet_info(packet: &Packet, input: &Input, options: &CopyOptions) -> Result<(usize, usize)> {
    // SAFETY: Live packet created by av_read_frame; numeric fields checked before use.
    let (index, size, flags) = unsafe {
        (
            (*packet.0).stream_index,
            (*packet.0).size,
            (*packet.0).flags,
        )
    };
    if index < 0 || index as usize >= input.streams().len() {
        return Err("packet has invalid stream index".into());
    }
    if size < 0 || size as usize > options.max_packet_bytes {
        return Err("packet exceeds payload budget".into());
    }
    if flags & AV_PKT_FLAG_CORRUPT as i32 != 0 {
        return Err("corrupt packet rejected".into());
    }
    Ok((index as usize, size as usize))
}
pub fn remux(source: &Path, destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    remux_input(Input::open_fast(source)?, destination, options)
}
fn remux_input(mut input: Input, destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    let selected = selection(&input, options)?;
    budget::admit_input_controlled_budget(&input, options, 1, false)?;
    budget::check_rss_budget(options)?;
    if !options.stream_metadata_set.is_empty() || !options.stream_metadata_delete.is_empty() {
        let stream_count = input.streams().len();
        for &(index, ..) in &options.stream_metadata_set {
            if index >= stream_count {
                return Err(format!("stream metadata index {index} out of range"));
            }
            if !selected.contains(&index) {
                return Err(format!(
                    "stream metadata index {index} is not in the selected streams"
                ));
            }
        }
        for &(index, _) in &options.stream_metadata_delete {
            if index >= stream_count {
                return Err(format!("stream metadata index {index} out of range"));
            }
            if !selected.contains(&index) {
                return Err(format!(
                    "stream metadata index {index} is not in the selected streams"
                ));
            }
        }
    }
    let mut output = Output::new_direct(destination, &input, &selected, Some(options))?;
    let mut packet = Packet::new()?;
    let mut stats = CopyStats {
        backend: "libavformat (native)",
        segments: 1,
        ..Default::default()
    };
    while packet.read(&mut input)? {
        check_budget_with_bytes(options, stats.packets, stats.payload_bytes)?;
        let (index, size) = packet_info(&packet, &input, options)?;
        if let Some(mapped) = selected.iter().position(|&i| i == index) {
            // SAFETY: Packet index and selected stream are checked and input is live.
            let tb = unsafe { (*input.streams()[index]).time_base };
            output.write(&mut packet, mapped, tb)?;
            stats.packets += 1;
            stats.payload_bytes += size as u64;
        }
    }
    emit_progress_done(options, stats.packets, stats.payload_bytes);
    output.finish()?;
    Ok(stats)
}
