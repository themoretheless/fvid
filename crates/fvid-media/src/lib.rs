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
pub use budget::{estimate_path_controlled_bytes, parse_max_memory_mib, parse_max_rss_mib};
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
mod filter;
#[cfg(feature = "cuda-hw")]
mod hw_cuda;
mod lossless;
mod loudness;
mod pcm;
mod play;
mod subtitle;
mod wav;
mod xfade;
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
use ffi::*;

/// Configure a filter graph with slice threads, matching the ffmpeg CLI default.
/// Per-pixel filters (exposure, grayworld, eq, …) declare AVFILTER_FLAG_SLICE_THREADS;
/// without this the linked libavfilter runs them single-threaded inside fvid.
pub(crate) unsafe fn configure_filter_graph(
    graph: *mut AVFilterGraph,
) -> i32 {
    unsafe {
        if !graph.is_null() {
            (*graph).thread_type |= AVFILTER_THREAD_SLICE as i32;
            if (*graph).nb_threads < 1 {
                (*graph).nb_threads = std::thread::available_parallelism()
                    .map(|n| n.get() as i32)
                    .unwrap_or(1);
            }
        }
        avfilter_graph_config(graph, ptr::null_mut())
    }
}
pub use filter::{
    PadRect, RotateAngle, TransposeMode, overlay_cli_vf, subtitles_cli_vf, xfade_cli_vf,
};
#[cfg(feature = "cuda-hw")]
pub use hw_cuda::{HwFilterOptions, HwFilterStats, hw_filter};
pub use lossless::{
    CropRect, EncoderSettings, LosslessStats, LosslessTransform, OverlaySpec, ScaleSize, XfadeSpec,
    crop_lossless, transcode, transcode_lossless,
};
pub use loudness::{
    DEFAULT_LOUDNORM_ARGS, LoudnessStats, LoudnormStats, apply_loudnorm, apply_loudnorm_dual,
    measure_loudness, validate_loudnorm_args,
};
pub use pcm::{PcmTrimStats, trim_pcm};
pub use play::{
    AUDIO_PITCH_MAX_MILLI, AUDIO_PITCH_MIN_MILLI, AUDIO_PITCH_STEP_MILLI, AUDIO_PITCH_UNITY_MILLI,
    AbLoop, AmbisonicChannelOrder, AmbisonicMode, AspectMode, AudioChannelMode,
    BALANCE_CENTER_MILLI, BALANCE_MAX_MILLI, BALANCE_MIN_MILLI, BALANCE_STEP_MILLI, BitmapSubtitle,
    Bookmark, COLOR_PRIMARIES_BT709, COLOR_PRIMARIES_BT2020, COLOR_TEMP_DAYLIGHT_K, COLOR_TRC_HLG,
    COLOR_TRC_SMPTE2084, CONTROLS_AUTOHIDE_DEFAULT_MS, CONTROLS_AUTOHIDE_MAX_MS,
    CONTROLS_AUTOHIDE_MIN_MS, CROSSFADE_DEFAULT_MS, CROSSFADE_MAX_MS, CROSSFADE_MIN_MS,
    CROSSFADE_STEP_MS, CROSSFEED_MAX_MILLI, CROSSFEED_STEP_MILLI, CacheDomain, CastProtocol,
    ClosedCaptionChannel, CropBoxMilli, CropPixels, DEBAND_DEFAULT_MILLI, DEBAND_MAX_MILLI,
    DeinterlaceMode, DisplayEffect, DisplayWhitePoint, DropFrameMode, EQ_BAND_COUNT, EQ_BAND_HZ,
    EQ_PREAMP_DEFAULT_MILLI, EQ_PREAMP_MAX_MILLI, EQ_PREAMP_MIN_MILLI, EQ_PREAMP_STEP_MILLI,
    EQ_PRESET_COUNT, EdlClip, EqPreset, FAVORITES_MAX, FOV_DEFAULT_MILLI, FOV_MAX_MILLI,
    FOV_MIN_MILLI, FOV_PRESET_MILLI, FOV_STEP_MILLI, FrameStep, GAPLESS_THRESHOLD_DEFAULT_US,
    GraphicEqState, HDR_NITS_DEFAULT, HDR_NITS_MAX, HDR_NITS_MIN, HTTP_RECONNECT_DEFAULT,
    HTTP_RECONNECT_MAX, HdrLightModel, HdrTonemap, IMAGE_DURATION_DEFAULT_SECS,
    IMAGE_DURATION_MAX_SECS, IMAGE_DURATION_MIN_SECS, IPD_DEFAULT_MILLI, IPD_MAX_MILLI,
    IPD_MIN_MILLI, IPD_STEP_MILLI, LogoPosition, LyricLine, MOUSE_HIDE_DEFAULT_MS, MarqueePosition,
    MediaKeyAction, NETWORK_CACHE_DEFAULT_MS, NETWORK_CACHE_MAX_MS, NETWORK_CACHE_MIN_MS,
    OSD_TIMEOUT_DEFAULT_MS, OSD_TIMEOUT_MAX_MS, OSD_TIMEOUT_MIN_MS, PAN_STEP_PX,
    PAPER_WHITE_DEFAULT_NITS, PITCH_MAX_MILLI, PITCH_MIN_MILLI, PITCH_STEP_MILLI,
    PLAYBACK_PROTOCOLS, PlayOptions, PlayRenderOptions, PlayStats, PlayStereo3D, PlaybackContinue,
    PlaylistSort, PositionDisplay, ProxyMode, RATE_STEP_MILLI, RATE_WHEEL_STEP_MILLI,
    RECENT_PLAY_MAX, REMOTE_CONTROL_DEFAULT_PORT, REPLAYGAIN_MAX_MILLI, REPLAYGAIN_MIN_MILLI,
    REPLAYGAIN_UNITY_MILLI, ROLL_STEP_MILLI, RepeatMode, RotateMode, SEEK_COARSE_US, SEEK_FINE_US,
    SLEEP_TIMER_STEPS_MIN, SPATIALIZER_DEFAULT_MILLI, SPATIALIZER_MAX_MILLI, SPATIALIZER_MIN_MILLI,
    SPATIALIZER_STEP_MILLI, SUBTITLE_OPACITY_MAX_MILLI, SUBTITLE_OPACITY_MIN_MILLI,
    SUBTITLE_OPACITY_STEP_MILLI, SUBTITLE_OPACITY_UNITY_MILLI, SUBTITLE_SCALE_MAX_MILLI,
    SUBTITLE_SCALE_MIN_MILLI, SUBTITLE_SCALE_STEP_MILLI, SUBTITLE_SCALE_UNITY_MILLI, SeekJump,
    SmilClip, SnapshotFormat, SphericalHotspot, SphericalProjection, SphericalStereoLayout,
    StereoPacking, SubtitleColor, SubtitleCue, SubtitleEncoding, SubtitlePosition,
    TELETEXT_PAGE_DEFAULT, TELETEXT_PAGE_MAX, TELETEXT_PAGE_MIN, TONE_STEP_MILLI, TONE_UNITY_MILLI,
    TONEMAP_STRENGTH_DEFAULT_MILLI, ToneState, VOLUME_MAX_MILLI, VOLUME_STEP_MILLI,
    VOLUME_WHEEL_STEP_MILLI, VideoPostFx, VisualizationMode, VrDisplayMode, WIDTH_MAX_MILLI,
    WIDTH_MIN_MILLI, WIDTH_STEP_MILLI, WIDTH_UNITY_MILLI, YAW_STEP_MILLI, ZOOM_MAX_MILLI,
    ZOOM_MIN_MILLI, ab_mark, ab_restart_us, ab_slot_load, ab_slot_store,
    accelerometer_horizon_pitch_milli, active_bitmap_subtitle, active_lyric_line, active_subtitle,
    adjust_pixel, adjust_step_milli, advance_rate_phase, ambisonic_label, ambisonic_order_label,
    anaglyph_dubois, apply_amplifier_sample, apply_audio_balance, apply_audio_channel,
    apply_audio_pitch_sample_index, apply_box_denoise_pixel, apply_bt2020_to_bt709_pixel,
    apply_bt2446_tonemap_pixel, apply_chorus_sample, apply_compressor, apply_crossfeed,
    apply_deband_pixel, apply_deinterlace_rgb, apply_delogo_rect, apply_dialogue_enhance_sample,
    apply_display_effect_pixel, apply_echo_sample, apply_eq_preamp_sample, apply_gamma_pixel,
    apply_hdr_black_lift_channel, apply_hdr_highlight_desat_pixel, apply_hdr_tonemap_pixel,
    apply_highpass_1pole, apply_hlg_ootf_pixel, apply_lowpass_1pole, apply_motion_blur_rgb,
    apply_night_mode_sample, apply_normalizer_sample, apply_param_eq_sample, apply_play_stereo3d,
    apply_replaygain_sample, apply_reverb_sample, apply_soft_limiter_sample, apply_spatializer,
    apply_stereo_width, apply_tone_frame, apply_unsharp_pixel, apply_video_post_fx,
    apply_video_post_fx_with_prev, apply_vr_vignette_pixel, apply_white_balance_pixel,
    aspect_label, ass_override_margin_px, atempo_duration_us, audio_bargraph_fills,
    audio_channel_label, audio_delay_frames, audio_desync_ms_from_us, audio_desync_us,
    audio_duck_gain_milli, audio_output_devices, audio_peak_milli, audio_pitch_step_milli,
    auto_hdr_tonemap, average_rgb_pixel, balance_step_milli, barrel_distort_uv_milli,
    blend_rgba_over_rgb, blend_tonemap_channel, blit_bitmap_subtitle, bookmark_step,
    brown_conrady_uv_milli, bt2020_to_bt709_rgb, bt2446_tonemap_channel, buffer_health_pct,
    buffer_health_ratio_milli, cache_domain_label, cardboard_eye_rect,
    cardboard_eye_yaw_offset_milli, cardboard_view_yaw_milli, cast_protocol_label, center_crop,
    chapter_index, chapter_step, chapter_thumbnail_times, checkerboard_eye_is_left,
    chromatic_aberration_uv_milli, clamp_adjust_milli, clamp_amplifier_milli,
    clamp_audio_pitch_milli, clamp_balance_milli, clamp_cache_ms, clamp_color_temp_kelvin,
    clamp_controls_autohide_ms, clamp_crop_box_milli, clamp_crop_pixels, clamp_crossfade_ms,
    clamp_crossfeed_milli, clamp_deband_milli, clamp_diffuse_white_nits, clamp_dvr_playhead_us,
    clamp_eq_milli, clamp_eq_preamp_milli, clamp_exclusive_latency_ms, clamp_fov_milli,
    clamp_hdr_maxcll, clamp_hdr_nits, clamp_hlg_system_gamma_milli, clamp_http_reconnect,
    clamp_image_duration_secs, clamp_ipd_milli, clamp_live_latency_ms, clamp_logo_opacity_milli,
    clamp_network_cache_ms, clamp_osd_timeout_ms, clamp_pan_px, clamp_paper_white_nits,
    clamp_param_eq_milli, clamp_pitch_milli, clamp_rate_milli, clamp_replaygain_milli,
    clamp_roll_milli, clamp_seek_us, clamp_spatializer_milli, clamp_subtitle_margin,
    clamp_subtitle_opacity_milli, clamp_subtitle_scale_milli, clamp_teletext_page,
    clamp_tonemap_strength_milli, clamp_volume_milli, clamp_width_milli, clamp_yaw_milli,
    clear_bookmarks, closed_caption_label, color_primaries_label, column_interleaved_eye_is_left,
    compass_heading_deg, compress_sample, controller_ray_hit, controls_should_hide,
    crop_box_to_pixels, crop_output_size, crossfade_gain_pair, crossfade_step_ms,
    crossfeed_step_milli, cubemap_cross_face_rect, cycle_ambisonic, cycle_ambisonic_order,
    cycle_angle, cycle_aspect, cycle_audio_channel, cycle_cache_domain, cycle_cast_protocol,
    cycle_closed_caption, cycle_deinterlace, cycle_display_effect, cycle_display_white_point,
    cycle_drop_frame, cycle_eq_bypass, cycle_eq_preset, cycle_fov_preset_milli,
    cycle_hdr_light_model, cycle_hdr_tonemap, cycle_integer_zoom, cycle_logo_position,
    cycle_marquee_position, cycle_output_device, cycle_play_stereo3d, cycle_playlist_sort,
    cycle_position_display, cycle_proxy_mode, cycle_rate_preset_milli, cycle_repeat, cycle_rotate,
    cycle_seek_jump, cycle_show_osd, cycle_sleep_timer_min, cycle_snapshot_format,
    cycle_spherical_projection, cycle_spherical_stereo, cycle_stereo_packing, cycle_subtitle_color,
    cycle_subtitle_encoding, cycle_subtitle_position, cycle_track, cycle_video_post_fx,
    cycle_video_track, cycle_visualization, cycle_vr_display, default_cache_ms,
    deinterlace_blend_rgb, deinterlace_bob_rgb, deinterlace_label, deinterlace_linear_rgb,
    deinterlace_mean_rgb, detect_bpm_from_onset_gaps_ms, detect_equirect_aspect,
    detect_letterbox_bars, display_effect_label, display_ratio, downmix_surround_to_stereo,
    drop_frame_label, dual_fisheye_tb_to_equirect, dual_fisheye_to_equirect, eac_face_uv_from_dir,
    encode_bmp, encode_png, eq_band_step_milli, eq_db_to_milli, eq_preamp_step_milli, eq_preset_db,
    eq_preset_gains, eq_preset_label, eq_unity_gains, estimate_frame_peak_milli,
    expand_hdr_channel, expand_play_inputs, filter_playlist_by_extension, filter_playlist_paths,
    find_audio_device, fit_aspect, fit_window_to_video, flip_uv, format_ab_osd, format_ab_slot_osd,
    format_abr_osd, format_adjust_osd, format_album_art_osd, format_ambisonic_order_osd,
    format_ambisonic_osd, format_amplifier_osd, format_anaglyph_dubois_osd, format_angle_osd,
    format_aspect_lock_osd, format_aspect_osd, format_ass_force_style, format_ass_force_style_osd,
    format_ass_override_osd, format_atempo_osd, format_atmos_layout_osd, format_audio_channel_osd,
    format_audio_desync_osd, format_audio_duck_osd, format_audio_pitch_osd,
    format_auto_horizon_osd, format_balance_osd, format_bargraph_osd, format_barrel_osd,
    format_binge_mode_osd, format_bitperfect_osd, format_bookmark_label, format_bookmark_osd,
    format_bookmarks_export, format_box_denoise_osd, format_bpm_osd, format_bt2446_osd,
    format_buffer_health_osd, format_cache_osd, format_cast_osd, format_cea708_service_osd,
    format_chapter_art_osd, format_chapter_list_export, format_chapter_osd,
    format_chapter_thumbs_osd, format_checkerboard_3d_osd, format_chorus_osd,
    format_chromatic_aberration_osd, format_clipboard_snapshot_osd, format_closed_caption_osd,
    format_cms_lut_osd, format_color_primaries_osd, format_color_temp_osd, format_compass_osd,
    format_compressor_osd, format_content_rating_osd, format_continue_watching_osd,
    format_crop_box_osd, format_crop_osd, format_crop_pixels_osd, format_crossfade_osd,
    format_crossfeed_osd, format_cube_lut_osd, format_cubemap_cross_osd, format_deband_osd,
    format_deinterlace_osd, format_delay_osd, format_delogo_osd, format_dialogue_enhance_osd,
    format_diffuse_white_osd, format_display_effect_osd, format_dolby_vision_osd,
    format_dolby_vision_profile_level_osd, format_downmix_osd, format_drag_seek_osd,
    format_drop_frame_osd, format_dvr_window_osd, format_eac_face_osd, format_echo_osd,
    format_edid_peak_osd, format_edl_clip_osd, format_epg_program_osd, format_eq_bypass_osd,
    format_eq_preamp_osd, format_eq_preset_osd, format_exclusive_latency_osd,
    format_external_subtitle_osd, format_favorite_osd, format_fit_window_osd, format_flip_osd,
    format_force_hdr_osd, format_forced_only_osd, format_forced_subtitle_osd,
    format_fov_preset_osd, format_frame_rate_osd, format_gamut_map_osd, format_gapless_osd,
    format_gaze_dwell_osd, format_guardian_osd, format_gyro_osd, format_haas_osd,
    format_hdr_black_lift_osd, format_hdr_brightness_osd, format_hdr_gamut_osd,
    format_hdr_headroom_osd, format_hdr_highlight_desat_osd, format_hdr_light_model_osd,
    format_hdr_mastering_osd, format_hdr_metadata_osd, format_hdr_nits_osd, format_hdr_peak_osd,
    format_hdr_sdr_ratio_osd, format_hdr_tonemap_osd, format_hdr10_plus_osd,
    format_hearing_impaired_osd, format_hlg_ootf_osd, format_hlg_system_gamma_osd,
    format_horizon_lock_osd, format_hotkeys_help_osd, format_http_auth_osd,
    format_http_reconnect_osd, format_hw_decode_osd, format_icecast_metadata_osd, format_ictcp_osd,
    format_image_duration_osd, format_image_loop_osd, format_instant_replay_osd,
    format_integer_zoom_osd, format_integrated_lufs_osd, format_ipd_osd, format_jump_osd,
    format_keyframe_seek_osd, format_lens_calibration_osd, format_letterbox_osd,
    format_live_edge_osd, format_live_latency_osd, format_logo_osd, format_loudness_osd,
    format_loudness_range_osd, format_lyric_osd, format_marquee_osd, format_maxrgb_osd,
    format_media_fingerprint_osd, format_media_info_osd, format_media_key_osd,
    format_media_library_osd, format_minimal_interface_osd, format_mosaic_osd,
    format_motion_blur_osd, format_named_bookmark_osd, format_network_bandwidth_osd,
    format_network_cache_osd, format_night_mode_osd, format_normalizer_osd, format_pan_osd,
    format_paper_white_osd, format_param_eq_osd, format_passthrough_osd, format_pause_osd,
    format_phase_correlation_osd, format_pip_osd, format_play_clock, format_play_stats,
    format_play_stats_csv, format_play_stereo3d_osd, format_playlist_fade_osd,
    format_playlist_filter_osd, format_playlist_m3u, format_playlist_osd, format_playlist_sort_osd,
    format_position_osd, format_program_osd, format_proxy_osd, format_queue_osd, format_rate_osd,
    format_rate_preset_osd, format_recent_osd, format_recenter_osd, format_record_osd,
    format_record_path, format_remote_control_osd, format_repeat_osd, format_replaygain_osd,
    format_resume_positions, format_reverb_osd, format_rotate_osd, format_row_interleaved_osd,
    format_scaletempo_osd, format_scrobble_line, format_scrub_preview_osd,
    format_secondary_subtitle_delay_osd, format_seek_jump_osd, format_show_osd, format_shuffle_osd,
    format_silence_skip_osd, format_skip_marker_osd, format_skip_segment_osd, format_sleep_osd,
    format_smart_playlist_osd, format_smil_clip_osd, format_snapshot_dir_osd,
    format_snapshot_prefix_osd, format_snapshot_sequential_name, format_snapshot_sequential_osd,
    format_snapshot_with_osd, format_soft_limiter_osd, format_spatializer_osd, format_spectrum_osd,
    format_spherical_hotspot_osd, format_spherical_osd, format_spherical_osd_ex,
    format_spherical_projection_osd, format_spherical_stereo_osd, format_st2094_l1_osd,
    format_stereo_packing_osd, format_stop_osd, format_storyboard_osd, format_stream_quality_osd,
    format_stream_rendition_osd, format_subtitle_color_osd, format_subtitle_encoding_osd,
    format_subtitle_opacity_osd, format_subtitle_position_osd, format_subtitle_scale_osd,
    format_teletext_osd, format_thumbnail_grid_osd, format_thumbnail_seek_osd, format_timecode_osd,
    format_title_osd, format_tone_filter_osd, format_tone_osd, format_tonemap_strength_osd,
    format_track_osd, format_true_peak_osd, format_unsharp_osd, format_up_next_osd,
    format_vectorscope_osd, format_video_post_fx_osd, format_video_track_osd,
    format_visualization_osd, format_volume_osd, format_vr_display_osd, format_vr_vignette_osd,
    format_vu_osd, format_wallpaper_osd, format_watch_party_osd, format_waveform_osd,
    format_webvtt_timestamp, format_white_point_osd, format_width_osd, format_wiggle_3d_osd,
    format_window_title, format_window_title_with_position, format_zoom_osd, fov_step_milli,
    frame_aspect, frame_rate_milli_from_duration_us, frame_step_target_us, gamma_channel,
    gapless_should_prefetch, gaze_dwell_triggered, graphic_eq_step, gyro_look_delta_milli,
    haas_delay_samples, hdr_brightness_boost_milli, hdr_gamut_warning, hdr_sdr_ratio_milli,
    hdr_tonemap_label, hlg_eotf, hlg_oetf, hlg_ootf_channel, hlg_system_gamma_default_milli,
    http_should_reconnect, ictcp_intensity_milli, image_duration_us, image_loop_remaining,
    initial_seek_us, initial_stop_us, insert_bookmark, instant_replay_us,
    integrated_lufs_from_short_term, ipd_step_milli, is_hdr_transfer, is_hls_playlist,
    is_playback_url, load_subtitle_file, locked_pitch_milli, locked_window_size, logo_anchor_xy,
    logo_position_label, loudness_range_l_milli, map_nits_via_paper_white, marquee_block_top_y,
    marquee_position_label, maxrgb_tonemap_pixel, media_display_title, media_fraction,
    media_key_label, media_library_entries, media_us_from_fraction, momentary_lufs_from_peak_milli,
    mosaic_tile_rect, mouse_should_hide, multi_room_sync_target_us, network_bandwidth_bps,
    network_cache_delay_us, next_snapshot_index, normalizer_gain_milli, normalizer_peak_step,
    order_step, osd_should_clear, pal8_to_rgba, palette_rgba_pixel, pan_step_px,
    parse_bookmarks_export, parse_cube_lut_1d_size, parse_degrees_milli, parse_edid_max_luminance,
    parse_edl_line, parse_hdr_mastering_nits, parse_hdr_maxcll_maxfall, parse_hdr_tonemap,
    parse_m3u_extinf_title, parse_play_clock, parse_play_stereo3d, parse_playlist_text,
    parse_resume_positions, parse_smil_clip_line, parse_spherical_projection,
    parse_spherical_stereo, parse_srt, parse_subtitle_clock, parse_subtitle_text, parse_ttml_clock,
    parse_webvtt_region_id, parse_webvtt_timestamp, passthrough_blend_milli,
    phase_correlation_milli, pip_rect, pitch_from_swipe_px, pitch_step_milli, plain_subtitle, play,
    play_paths, play_stereo3d_label, playback_continue, playlist_edge_fade_gain_milli,
    playlist_sort_label, playlist_step, position_us_from_digit, pq_eotf, pq_oetf,
    prefer_abr_rendition_index, prefer_album_art_path, prefer_external_subtitle_path,
    prefer_forced_subtitle_index, prefer_hearing_impaired_subtitle_index,
    prefer_opensubtitles_path, prefer_program_index, prefer_stream_quality_index,
    prefer_track_index, project_azimuthal_equidistant_view, project_cylindrical_view,
    project_equirect_view, project_equirect_view_ex, project_equisolid_view, project_gnomonic_view,
    project_little_planet, project_mercator_view, project_miller_view, project_octahedral_view,
    project_orthographic_view, project_panini_view, project_sinusoidal_view,
    project_spherical_view, proxy_mode_label, push_recent_path, queue_insert, random_seek_us,
    rate_from_wheel, rate_step_milli, recenter_spherical_view, remaining_media_us,
    render_play_pixels, replaygain_milli_from_db_milli, reset_av_delays, reset_tone_gains,
    reset_video_adjust, reset_zoom_pan, resume_seek_us, roll_step_milli, rotate_label,
    rotate_pixel, rotate_size, row_interleaved_eye_pixel, sample_1d_lut_u8, sample_cubemap_pixel,
    sample_eac_pixel, sample_equirect_pixel, sample_octahedral_pixel, scale_elapsed_us,
    scale_hdr_display_channel, scale_sdr_overlay_to_paper_white, scaletempo_duration_us,
    scope_samples_u8, scrub_preview_us, secondary_subtitle_delay_us, seek_end_us,
    seek_from_drag_px, seek_from_wheel, seek_step_us, seek_step_us_ex, set_eq_gains_from_preset,
    short_term_lufs_from_peaks, should_drop_late_frame, should_quit_at_end, should_stop_playback,
    should_toggle_fullscreen_on_click, shuffled_indices, silence_skip_target_us,
    skip_marker_target_us, skip_segment_target_us, sleep_deadline_secs, sleep_timer_fired,
    snap_seek_to_keyframe, snapshot_format_ext, snapshot_path, snapshot_path_in_dir,
    snapshot_path_with_ext, snapshot_path_with_prefix, soft_clip_sample, sort_playlist_paths,
    spatializer_step_milli, spectrum_bar_fills, spherical_hotspot_hit, spherical_projection_label,
    spherical_stereo_label, spherical_stereo_uv_rect, step_audio_skew, stereo_packing_label,
    stop_playback_us, storyboard_tile_index, subtitle_block_top_y, subtitle_clock_us,
    subtitle_color_label, subtitle_color_rgba, subtitle_delay_us, subtitle_encoding_label,
    subtitle_font_px, subtitle_margin_px, subtitle_opacity_step_milli, subtitle_opacity_u8,
    subtitle_position_label, subtitle_scale_step_milli, subtitle_window,
    suggest_hdr_nits_from_peak, teletext_page_step, thumbnail_cache_key, thumbnail_grid_rect,
    thumbnail_seek_us, timeshift_lag_us, toggle_favorite, tone_gain_step_milli, tone_step,
    tonemap_channel, true_peak_milli, up_next_should_start, vectorscope_quadrant_counts,
    video_post_fx_label, visualization_label, volume_from_wheel, volume_step_milli,
    vr_display_label, vr_vignette_gain_milli, vu_bar_fills, watch_progress_milli,
    waveform_column_fills, white_point_xy_milli, width_step_milli, wiggle_yaw_offset_milli,
    yaw_from_swipe_px, yaw_step_milli, zoom_label, zoom_pan_rect, zoom_size, zoom_step,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    path::{Path, PathBuf},
    ptr, slice,
};
pub use subtitle::{
    SubtitleCodec, SubtitleConvertOptions, SubtitleConvertStats, burn_subtitles, convert_subtitles,
    overlay_video,
};
pub use xfade::{xfade_filter_complex, xfade_video};
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
            let code =
                avformat_open_input(&mut input.0, location.as_ptr(), ptr::null(), &mut options);
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
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )))
    }
    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
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
                    return Err(
                        "output container cannot represent exact presentation duration".into(),
                    );
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
