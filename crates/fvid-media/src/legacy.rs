// Temporary FFmpeg-library backend. No subprocess execution in production.
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
mod plan;
pub use budget::{estimate_path_controlled_bytes, parse_max_memory_mib, parse_max_rss_mib};
pub use edit::{concat, trim};
pub use fvid_media_info::parse_time;
pub use plan::{
    MediaPlan, PlanStep, PlanStream, plan_burn_subtitles, plan_concat, plan_decode_audio,
    plan_loudness, plan_loudnorm, plan_merge_audio, plan_mix_audio, plan_overlay, plan_remux,
    plan_transcode_lossless, plan_trim, plan_trim_pcm, plan_xfade,
};
mod audio;
mod audio_layout;
mod pcm_format_adapter;
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
    audio_duck_gain_milli, audio_peak_milli, audio_pitch_step_milli,
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
    phase_correlation_milli, pip_rect, pitch_from_swipe_px, pitch_step_milli, plain_subtitle,
    play_stereo3d_label, playback_continue, playlist_edge_fade_gain_milli,
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
#[cfg(feature = "player")]
pub use play::{audio_output_devices, play, play_paths};
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
/// `AVERROR_INVALIDDATA`; FFmpeg's own CLI reads and decodes past it instead of failing.
const INVALID_DATA: i32 = -1094995529;
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
fn rescale_owned(value: i64, source: AVRational, target: AVRational) -> Result<i64> {
    if source.num <= 0 || source.den <= 0 || target.num <= 0 || target.den <= 0 {
        return Err("invalid timestamp time base".into());
    }
    crate::owned_time::rescale_nearest(value,
        crate::owned_time::TimeBase { numerator: source.num as u32, denominator: source.den as u32 },
        crate::owned_time::TimeBase { numerator: target.num as u32, denominator: target.den as u32 })
}
fn rescale_capacity_owned(value: i64, output_rate: i32, input_rate: i32) -> Result<i64> {
    if output_rate <= 0 || input_rate <= 0 { return Err("invalid resampler rate".into()); }
    crate::owned_time::rescale_ceil(value,
        crate::owned_time::TimeBase {numerator:1, denominator:input_rate as u32},
        crate::owned_time::TimeBase {numerator:1, denominator:output_rate as u32})
}
fn check(code: i32, operation: &str) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    let detail = crate::owned_backend_error::describe(code);
    Err(format!("{operation}: {detail} ({code})"))
}
/// # Safety
/// `layout` must point to a writable, uninitialized channel layout, following
/// the existing backend initializer contract. No allocation is made for native masks.
unsafe fn channel_layout_default_owned(layout: *mut AVChannelLayout, channels: i32) {
    let description = u16::try_from(channels).ok().and_then(crate::owned_pcm_channels::default_layout);
    // Unknown channel counts preserve an unspecified layout, without guessing speakers.
    unsafe {
        layout.write(AVChannelLayout {
            order: if description.is_some() {
                AVChannelOrder_AV_CHANNEL_ORDER_NATIVE
            } else {
                AVChannelOrder_AV_CHANNEL_ORDER_UNSPEC
            },
            nb_channels: channels,
            u: AVChannelLayout__bindgen_ty_1 { mask: description.map_or(0, |layout| layout.mask) },
            opaque: std::ptr::null_mut(),
        });
    }
}
/// # Safety
/// Both pointers must refer to valid initialized channel layouts.
unsafe fn channel_layout_compare_owned(a: *const AVChannelLayout, b: *const AVChannelLayout) -> i32 {
    unsafe {
        let a = &*a;
        let b = &*b;
        if a.nb_channels != b.nb_channels { return 1; }
        let a_unspecified = a.order == AVChannelOrder_AV_CHANNEL_ORDER_UNSPEC;
        let b_unspecified = b.order == AVChannelOrder_AV_CHANNEL_ORDER_UNSPEC;
        if a_unspecified || b_unspecified { return i32::from(a_unspecified != b_unspecified); }
        if a.order == b.order && (a.order == AVChannelOrder_AV_CHANNEL_ORDER_NATIVE
            || a.order == AVChannelOrder_AV_CHANNEL_ORDER_AMBISONIC) {
            return i32::from(a.u.mask != b.u.mask);
        }
        i32::from(!crate::owned_pcm_channels::ordered_channels_equal(
            a.nb_channels, |index| channel_layout_channel_owned(a, index),
            |index| channel_layout_channel_owned(b, index),
        ))
    }
}
/// # Safety
/// A custom layout must have a readable map covering its declared channels.
unsafe fn channel_layout_channel_owned(layout: &AVChannelLayout, index: u32) -> Option<i32> {
    unsafe {
        if index >= layout.nb_channels.max(0) as u32 { return None; }
        if layout.order == AVChannelOrder_AV_CHANNEL_ORDER_CUSTOM {
            let id = (*layout.u.map.add(index as usize)).id;
            return (id != AVChannel_AV_CHAN_NONE).then_some(id);
        }
        let mut speaker_index = index;
        if layout.order == AVChannelOrder_AV_CHANNEL_ORDER_AMBISONIC {
            let components = layout.nb_channels - layout.u.mask.count_ones() as i32;
            if (index as i32) < components {
                return Some(AVChannel_AV_CHAN_AMBISONIC_BASE + index as i32);
            }
            speaker_index = index - components.max(0) as u32;
        } else if layout.order != AVChannelOrder_AV_CHANNEL_ORDER_NATIVE {
            return None;
        }
        crate::owned_pcm_channels::mask_channel_at(layout.u.mask, speaker_index)
    }
}
/// # Safety
/// The layout must be initialized; a custom map must belong to the backend allocator.
unsafe fn channel_layout_uninit_owned(layout: *mut AVChannelLayout) {
    unsafe {
        if (*layout).order == AVChannelOrder_AV_CHANNEL_ORDER_CUSTOM {
            av_channel_layout_uninit(layout);
        } else {
            layout.write(std::mem::zeroed());
        }
    }
}
/// # Safety
/// Both layouts must be initialized. Source and destination must not alias.
unsafe fn channel_layout_copy_owned(destination: *mut AVChannelLayout, source: *const AVChannelLayout) -> i32 {
    unsafe {
        if (*source).order == AVChannelOrder_AV_CHANNEL_ORDER_CUSTOM {
            return av_channel_layout_copy(destination, source);
        }
        channel_layout_uninit_owned(destination);
        // Non-custom layouts own no heap map. Preserve the opaque user pointer.
        std::ptr::copy_nonoverlapping(source, destination, 1);
        0
    }
}
#[allow(non_upper_case_globals)] // Generated backend constant names.
fn codec_descriptor_owned(codec: AVCodecID) -> Option<crate::owned_codec_metadata::CodecDescriptor> {
    let name = match codec {
        AVCodecID_AV_CODEC_ID_H264 => "h264",
        AVCodecID_AV_CODEC_ID_HEVC => "hevc",
        AVCodecID_AV_CODEC_ID_AV1 => "av1",
        AVCodecID_AV_CODEC_ID_VP8 => "vp8",
        AVCodecID_AV_CODEC_ID_VP9 => "vp9",
        AVCodecID_AV_CODEC_ID_FFV1 => "ffv1",
        AVCodecID_AV_CODEC_ID_AAC => "aac",
        AVCodecID_AV_CODEC_ID_AAC_LATM => "aac_latm",
        AVCodecID_AV_CODEC_ID_OPUS => "opus",
        AVCodecID_AV_CODEC_ID_VORBIS => "vorbis",
        AVCodecID_AV_CODEC_ID_FLAC => "flac",
        AVCodecID_AV_CODEC_ID_ALAC => "alac",
        AVCodecID_AV_CODEC_ID_MP3 => "mp3",
        AVCodecID_AV_CODEC_ID_MP2 => "mp2",
        AVCodecID_AV_CODEC_ID_AC3 => "ac3",
        AVCodecID_AV_CODEC_ID_EAC3 => "eac3",
        AVCodecID_AV_CODEC_ID_PCM_U8 => "pcm_u8",
        AVCodecID_AV_CODEC_ID_PCM_S8 => "pcm_s8",
        AVCodecID_AV_CODEC_ID_PCM_S16LE => "pcm_s16le",
        AVCodecID_AV_CODEC_ID_PCM_S16BE => "pcm_s16be",
        AVCodecID_AV_CODEC_ID_PCM_S24LE => "pcm_s24le",
        AVCodecID_AV_CODEC_ID_PCM_S24BE => "pcm_s24be",
        AVCodecID_AV_CODEC_ID_PCM_S32LE => "pcm_s32le",
        AVCodecID_AV_CODEC_ID_PCM_S32BE => "pcm_s32be",
        AVCodecID_AV_CODEC_ID_PCM_S64LE => "pcm_s64le",
        AVCodecID_AV_CODEC_ID_PCM_S64BE => "pcm_s64be",
        AVCodecID_AV_CODEC_ID_PCM_F32LE => "pcm_f32le",
        AVCodecID_AV_CODEC_ID_PCM_F32BE => "pcm_f32be",
        AVCodecID_AV_CODEC_ID_PCM_F64LE => "pcm_f64le",
        AVCodecID_AV_CODEC_ID_PCM_F64BE => "pcm_f64be",
        AVCodecID_AV_CODEC_ID_PCM_ALAW => "pcm_alaw",
        AVCodecID_AV_CODEC_ID_PCM_MULAW => "pcm_mulaw",
        AVCodecID_AV_CODEC_ID_RAWVIDEO => "rawvideo",
        AVCodecID_AV_CODEC_ID_MJPEG => "mjpeg",
        AVCodecID_AV_CODEC_ID_PNG => "png",
        AVCodecID_AV_CODEC_ID_BMP => "bmp",
        AVCodecID_AV_CODEC_ID_TIFF => "tiff",
        AVCodecID_AV_CODEC_ID_GIF => "gif",
        AVCodecID_AV_CODEC_ID_WEBP => "webp",
        AVCodecID_AV_CODEC_ID_MOV_TEXT => "mov_text",
        AVCodecID_AV_CODEC_ID_SUBRIP => "subrip",
        AVCodecID_AV_CODEC_ID_ASS => "ass",
        AVCodecID_AV_CODEC_ID_SSA => "ssa",
        AVCodecID_AV_CODEC_ID_BIN_DATA => "bin_data",
        _ => return None,
    };
    crate::owned_codec_metadata::descriptor(name)
}
fn codec_name_owned(codec: AVCodecID) -> String {
    match codec_descriptor_owned(codec) {
        Some(description) => description.name.into(),
        None => string(unsafe { avcodec_get_name(codec) }),
    }
}
fn codec_profile_name_owned(codec: AVCodecID, profile: i32) -> Option<String> {
    if let Some(description) = codec_descriptor_owned(codec) {
        return description.profile_name(profile).map(String::from);
    }
    let name = unsafe { avcodec_profile_name(codec, profile) };
    (!name.is_null()).then(|| string(name))
}
#[allow(non_upper_case_globals)] // Generated backend constant names.
fn media_type_name_owned(kind: AVMediaType) -> &'static str {
    match kind {
        AVMediaType_AVMEDIA_TYPE_VIDEO => "video",
        AVMediaType_AVMEDIA_TYPE_AUDIO => "audio",
        AVMediaType_AVMEDIA_TYPE_DATA => "data",
        AVMediaType_AVMEDIA_TYPE_SUBTITLE => "subtitle",
        AVMediaType_AVMEDIA_TYPE_ATTACHMENT => "attachment",
        _ => "",
    }
}
fn owned_pixel_format_from_legacy(format: AVPixelFormat) -> Option<crate::owned_pixel_format::PixelFormat> {
    use crate::owned_pixel_format::PixelFormat;
    match format {
        AVPixelFormat_AV_PIX_FMT_YUV420P => Some(PixelFormat::Yuv420p),
        AVPixelFormat_AV_PIX_FMT_YUV422P => Some(PixelFormat::Yuv422p),
        AVPixelFormat_AV_PIX_FMT_YUV444P => Some(PixelFormat::Yuv444p),
        AVPixelFormat_AV_PIX_FMT_GRAY8 => Some(PixelFormat::Gray8),
        AVPixelFormat_AV_PIX_FMT_RGB24 => Some(PixelFormat::Rgb24),
        AVPixelFormat_AV_PIX_FMT_BGR24 => Some(PixelFormat::Bgr24),
        AVPixelFormat_AV_PIX_FMT_RGBA => Some(PixelFormat::Rgba),
        AVPixelFormat_AV_PIX_FMT_BGRA => Some(PixelFormat::Bgra),
        AVPixelFormat_AV_PIX_FMT_NV12 => Some(PixelFormat::Nv12),
        AVPixelFormat_AV_PIX_FMT_NV21 => Some(PixelFormat::Nv21),
        AVPixelFormat_AV_PIX_FMT_YUV420P9LE => Some(PixelFormat::Yuv420p9Le),
        AVPixelFormat_AV_PIX_FMT_YUV420P9BE => Some(PixelFormat::Yuv420p9Be),
        AVPixelFormat_AV_PIX_FMT_YUV420P10LE => Some(PixelFormat::Yuv420p10Le),
        AVPixelFormat_AV_PIX_FMT_YUV420P10BE => Some(PixelFormat::Yuv420p10Be),
        AVPixelFormat_AV_PIX_FMT_YUV420P12LE => Some(PixelFormat::Yuv420p12Le),
        AVPixelFormat_AV_PIX_FMT_YUV420P12BE => Some(PixelFormat::Yuv420p12Be),
        AVPixelFormat_AV_PIX_FMT_YUV420P14LE => Some(PixelFormat::Yuv420p14Le),
        AVPixelFormat_AV_PIX_FMT_YUV420P14BE => Some(PixelFormat::Yuv420p14Be),
        AVPixelFormat_AV_PIX_FMT_YUV420P16LE => Some(PixelFormat::Yuv420p16Le),
        AVPixelFormat_AV_PIX_FMT_YUV420P16BE => Some(PixelFormat::Yuv420p16Be),
        AVPixelFormat_AV_PIX_FMT_YUV422P9LE => Some(PixelFormat::Yuv422p9Le),
        AVPixelFormat_AV_PIX_FMT_YUV422P9BE => Some(PixelFormat::Yuv422p9Be),
        AVPixelFormat_AV_PIX_FMT_YUV422P10LE => Some(PixelFormat::Yuv422p10Le),
        AVPixelFormat_AV_PIX_FMT_YUV422P10BE => Some(PixelFormat::Yuv422p10Be),
        AVPixelFormat_AV_PIX_FMT_YUV422P12LE => Some(PixelFormat::Yuv422p12Le),
        AVPixelFormat_AV_PIX_FMT_YUV422P12BE => Some(PixelFormat::Yuv422p12Be),
        AVPixelFormat_AV_PIX_FMT_YUV422P14LE => Some(PixelFormat::Yuv422p14Le),
        AVPixelFormat_AV_PIX_FMT_YUV422P14BE => Some(PixelFormat::Yuv422p14Be),
        AVPixelFormat_AV_PIX_FMT_YUV422P16LE => Some(PixelFormat::Yuv422p16Le),
        AVPixelFormat_AV_PIX_FMT_YUV422P16BE => Some(PixelFormat::Yuv422p16Be),
        AVPixelFormat_AV_PIX_FMT_YUV444P9LE => Some(PixelFormat::Yuv444p9Le),
        AVPixelFormat_AV_PIX_FMT_YUV444P9BE => Some(PixelFormat::Yuv444p9Be),
        AVPixelFormat_AV_PIX_FMT_YUV444P10LE => Some(PixelFormat::Yuv444p10Le),
        AVPixelFormat_AV_PIX_FMT_YUV444P10BE => Some(PixelFormat::Yuv444p10Be),
        AVPixelFormat_AV_PIX_FMT_YUV444P12LE => Some(PixelFormat::Yuv444p12Le),
        AVPixelFormat_AV_PIX_FMT_YUV444P12BE => Some(PixelFormat::Yuv444p12Be),
        AVPixelFormat_AV_PIX_FMT_YUV444P14LE => Some(PixelFormat::Yuv444p14Le),
        AVPixelFormat_AV_PIX_FMT_YUV444P14BE => Some(PixelFormat::Yuv444p14Be),
        AVPixelFormat_AV_PIX_FMT_YUV444P16LE => Some(PixelFormat::Yuv444p16Le),
        AVPixelFormat_AV_PIX_FMT_YUV444P16BE => Some(PixelFormat::Yuv444p16Be),
        AVPixelFormat_AV_PIX_FMT_GRAY16LE => Some(PixelFormat::Gray16Le),
        AVPixelFormat_AV_PIX_FMT_GRAY16BE => Some(PixelFormat::Gray16Be),
        _ => None,
    }
}
fn pixel_format_name_raw(format: AVPixelFormat) -> *const std::ffi::c_char {
    if let Some(format) = owned_pixel_format_from_legacy(format) {
        return format.nul_name().as_ptr().cast();
    }
    // SAFETY: libav returns a static format name or NULL. Unmigrated formats
    // retain their existing metadata route until their descriptions are owned.
    unsafe { av_get_pix_fmt_name(format) }
}
fn string(pointer: *const std::ffi::c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: Internal callers pass live backend-owned or static owned NUL-terminated strings.
    unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() }
}
struct Input(*mut AVFormatContext);
/// Whether a file's own container header carries the codec parameters and stream timing, so a
/// decode-only reader can skip FFmpeg's packet probe of the stream table.
fn header_carries_params(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            value.eq_ignore_ascii_case("mp4")
                || value.eq_ignore_ascii_case("mov")
                || value.eq_ignore_ascii_case("m4v")
                || value.eq_ignore_ascii_case("m4a")
                || value.eq_ignore_ascii_case("srt")
        })
}
impl Input {
    fn open(path: &Path) -> Result<Self> {
        Self::open_hinted(path, true, None)
    }
    /// MP4-family headers carry complete codec parameters and stream timing.
    /// Decode-only/filter paths can avoid a redundant packet probe at startup.
    fn open_fast(path: &Path) -> Result<Self> {
        Self::open_hinted(path, !header_carries_params(path), None)
    }
    /// As [`Input::open_fast`], for a caller that names the demuxer itself.
    pub(crate) fn open_fast_hinted(path: &Path, format: Option<&str>) -> Result<Self> {
        Self::open_hinted(path, !header_carries_params(path), format)
    }
    /// A file whose bytes name no demuxer is read through one the caller states, which is what
    /// ffmpeg's `-f` does. A named demuxer is used without probing, so the format whitelist that
    /// the standalone input policy sets has no say over it: under that policy a hint is refused
    /// rather than trusted. A hinted demuxer states nothing in its header either, so the stream
    /// table is always read.
    pub(crate) fn open_hinted(
        path: &Path,
        find_stream_info: bool,
        format: Option<&str>,
    ) -> Result<Self> {
        if format.is_some() && input_policy::active() {
            return Err("an input format hint is not accepted under the standalone input policy".into());
        }
        Self::open_with_stream_info(path, find_stream_info, format)
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
                let absolute = crate::owned_time::rescale_nearest(
                    chapter.start,
                    crate::owned_time::TimeBase { numerator: chapter.time_base.num as u32, denominator: chapter.time_base.den as u32 },
                    crate::owned_time::TimeBase { numerator: 1, denominator: 1_000_000 },
                ).unwrap_or(i64::MIN);
                starts.push(absolute.saturating_sub(origin_us).max(0));
            }
            starts.sort_unstable();
            starts.dedup();
            starts
        }
    }
    fn open_with_stream_info(
        path: &Path,
        mut find_stream_info: bool,
        format: Option<&str>,
    ) -> Result<Self> {
        if !path.is_file() {
            return Err("media input must be an existing local file".into());
        }
        let name = format.map(cstring).transpose()?;
        let hint = format.unwrap_or("");
        // A demuxer chosen by name is not the one a header would have named, so its own
        // description of the stream table is the only one worth trusting.
        find_stream_info |= format.is_some();
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
            let mut hinted = ptr::null();
            if let Some(name) = name.as_ref() {
                // SAFETY: The name is a live NUL-terminated string; the lookup only reads
                // the library's own demuxer table and returns a static descriptor or null.
                hinted = av_find_input_format(name.as_ptr());
                if hinted.is_null() {
                    av_dict_free(&mut options);
                    return Err(format!("no demuxer named {hint}"));
                }
            }
            let code = avformat_open_input(&mut input.0, path.as_ptr(), hinted, &mut options);
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
        match self.try_read(input) {
            Ok(available) => Ok(available),
            Err(code) => Err(check(code, "read packet").unwrap_err()),
        }
    }
    /// Like [`Packet::read`], but hands back the raw FFmpeg code so a caller that tolerates
    /// damaged input can decide per code whether the stream is still worth reading.
    fn try_read(&mut self, input: &mut Input) -> std::result::Result<bool, i32> {
        // SAFETY: Both resources are exclusively borrowed and valid. Unref permits reuse.
        unsafe {
            av_packet_unref(self.0);
            let code = av_read_frame(input.0, self.0);
            if code == EOF {
                Ok(false)
            } else if code < 0 {
                Err(code)
            } else {
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
pub use fvid_media_info::{StreamInfo, ChapterInfo, MediaInfo};
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
    probe_as(path, None)
}
/// `probe` for a file whose own bytes name no demuxer, read through the one `format` names —
/// what ffmpeg's `-f` states. Without that name the file opens as nothing, so a demuxer that
/// probes for no signature is only reachable through here.
pub fn probe_as(path: &Path, format: Option<&str>) -> Result<MediaInfo> {
    if let Some(info) = crate::owned_probe::try_mp4_as(path, format)? {
        return Ok(info);
    }
    if let Some(info) = crate::owned_probe::try_webm_as(path, format)? {
        return Ok(info);
    }
    if format.is_none_or(|name| name == "wav")
        && crate::owned_wave_inspect::is_wave(path).unwrap_or(false)
    {
        if let Ok(info) = crate::owned_probe::probe_wave(path) {
            return Ok(info);
        }
    }
    if let Some(info) = crate::owned_probe::try_adts_as(path, format)? {
        return Ok(info);
    }
    if format.is_none_or(|name| matches!(name,"y4m"|"yuv4mpegpipe")) {
        if let Some(info) = crate::owned_y4m_probe::try_y4m(path)? {
            return Ok(info);
        }
    }
    let input = Input::open_hinted(path, true, format)?;
    describe(path, &input)
}
fn describe(path: &Path, input: &Input) -> Result<MediaInfo> {
    // SAFETY: Input owns all contexts, stream parameters and strings for this block.
    unsafe {
        let mut streams = Vec::new();
        for (index, stream) in input.streams().iter().enumerate() {
            let s = &**stream;
            let p = &*s.codecpar;
            streams.push(StreamInfo {
                index,
                media_type: media_type_name_owned(p.codec_type).into(),
                codec: codec_name_owned(p.codec_id),
                time_base: [s.time_base.num, s.time_base.den],
                start: (s.start_time != NOPTS).then_some(s.start_time),
                duration: (s.duration != NOPTS).then_some(s.duration),
                bit_rate: (p.bit_rate > 0).then_some(p.bit_rate),
                average_frame_rate: [s.avg_frame_rate.num, s.avg_frame_rate.den],
                profile: codec_profile_name_owned(p.codec_id, p.profile),
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
pub use fvid_media_info::Capabilities;
pub use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};

pub use fvid_control::CopyOptions;

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
pub use fvid_media_info::CopyStats;

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
            if !same_time_base {
                if self.strict_timing {
                    if time_base.num <= 0 || time_base.den <= 0 || target.num <= 0 || target.den <= 0 {
                        return Err("invalid mux time base".into());
                    }
                    let source = crate::owned_time::TimeBase { numerator: time_base.num as u32, denominator: time_base.den as u32 };
                    let target = crate::owned_time::TimeBase { numerator: target.num as u32, denominator: target.den as u32 };
                    // Compute all fields before mutation: a failed exact conversion leaves timing intact.
                    let pts = crate::owned_time::rescale_exact((*packet.0).pts, source, target)?;
                    let dts = crate::owned_time::rescale_exact((*packet.0).dts, source, target)?;
                    let duration = crate::owned_time::rescale_exact((*packet.0).duration, source, target)?;
                    if !matches!((*packet.0).pts, i64::MIN | i64::MAX) { (*packet.0).pts = pts; }
                    if !matches!((*packet.0).dts, i64::MIN | i64::MAX) { (*packet.0).dts = dts; }
                    if (*packet.0).duration > 0 { (*packet.0).duration = duration; }
                } else {
                    if time_base.num <= 0 || time_base.den <= 0 || target.num <= 0 || target.den <= 0 { return Err("invalid mux time base".into()); }
                    let source = crate::owned_time::TimeBase { numerator: time_base.num as u32, denominator: time_base.den as u32 };
                    let target = crate::owned_time::TimeBase { numerator: target.num as u32, denominator: target.den as u32 };
                    let [pts, dts, duration] = crate::owned_time::packet_nearest((*packet.0).pts, (*packet.0).dts, (*packet.0).duration, source, target)?;
                    (*packet.0).pts = pts;
                    (*packet.0).dts = dts;
                    (*packet.0).duration = duration;
                }
            }
            (*packet.0).stream_index = index as i32;
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
    validate_packet(packet, input, options, true)
}

/// Like [`packet_info`], but lets `AV_PKT_FLAG_CORRUPT` through to the decoder, which is what
/// FFmpeg's CLI does: the decoder decides whether the damage matters. Copy and remux keep the
/// strict check so damaged bytes never land in a new file.
pub(crate) fn packet_info_for_decode(
    packet: &Packet,
    input: &Input,
    options: &CopyOptions,
) -> Result<(usize, usize)> {
    validate_packet(packet, input, options, false)
}

fn validate_packet(
    packet: &Packet,
    input: &Input,
    options: &CopyOptions,
    reject_corrupt: bool,
) -> Result<(usize, usize)> {
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
    if reject_corrupt && flags & AV_PKT_FLAG_CORRUPT as i32 != 0 {
        return Err("corrupt packet rejected".into());
    }
    Ok((index as usize, size as usize))
}
pub fn remux(source: &Path, destination: &Path, options: &CopyOptions) -> Result<CopyStats> {
    if crate::owned_mp4_remux::supports(source, destination, options) {
        return crate::owned_mp4_remux::remux(source, destination, options);
    }
    if crate::owned_adts_remux::supports(source, destination, options) {
        return crate::owned_adts_remux::remux(source, destination, options);
    }

    if crate::owned_matroska_remux::supports(source, destination, options) {
        return crate::owned_matroska_remux::remux(source, destination, options);
    }

    if crate::owned_wave_remux::supports(source, destination, options) {
        return crate::owned_wave_remux::remux(source, destination, options);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A GEM raster of 8x2 at one bit per pixel: two bytes the format skips, then seven
    /// big-endian words — header length in words, colour planes, pattern block size, sample
    /// aspect, width, height — and a run copying two row bytes. Nothing in these bytes states
    /// which format they are in, which is what a hint is for.
    const GEM: [u8; 20] = [0, 0, 0, 8, 0, 1, 0, 8, 0, 1, 0, 1, 0, 8, 0, 2, 0x80, 2, 0xAA, 0x55];

    /// Each test writes its own copy: the reads happen in parallel, and a file another test
    /// removes mid-run would fail this one for the wrong reason.
    fn signatureless(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fvid-hint-{name}-{}.gem", std::process::id()));
        std::fs::write(&path, GEM).expect("the fixture is written for the reading tests");
        path
    }

    #[test]
    fn a_file_that_probes_as_nothing_reads_when_the_caller_names_the_demuxer() {
        let path = signatureless("read");
        assert!(
            probe(&path).is_err(),
            "GEM states no signature, so a probe has nothing to find"
        );
        let info = probe_as(&path, Some("gem_pipe")).expect("the named demuxer reads the file");
        assert_eq!(info.format, "gem_pipe");
        assert_eq!(
            (info.streams[0].codec.as_str(), info.streams[0].width, info.streams[0].height),
            ("gem", 8, 2)
        );
        let stats = decode_video_transformed(
            &path,
            DecodeTransform {
                input_format: Some("gem_pipe".into()),
                ..Default::default()
            },
        )
        .expect("the same name carries the decode path");
        assert_eq!(stats.video_frames, 1);
        assert_eq!(
            stats.decode_errors, 0,
            "a read that met no damage reports none: the counter is not a constant"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_hint_that_names_no_demuxer_says_so_instead_of_reading_anyway() {
        let path = signatureless("unknown");
        let error = probe_as(&path, Some("no_such_demuxer"))
            .err()
            .expect("a name the build has no demuxer for is not a read");
        assert!(error.contains("no demuxer named no_such_demuxer"), "{error}");
        std::fs::remove_file(&path).ok();
    }

    /// A demuxer named by the caller is taken without probing, so the format whitelist the
    /// standalone policy sets screens nothing of it: under that policy the hint is refused.
    #[test]
    fn the_standalone_input_policy_refuses_a_demuxer_the_caller_names() {
        let path = signatureless("policy");
        let error = with_standalone_inputs(|| probe_as(&path, Some("gem_pipe")))
            .err()
            .expect("the policy cannot check what the caller states");
        assert!(error.contains("standalone input policy"), "{error}");
        std::fs::remove_file(&path).ok();
    }

    /// A Bitmap Brothers JV file: the signature the format probes for, a 64x64 frame table with one
    /// record, and the palette that record points at. Whole, this file is read by the reference with
    /// one frame; stripped of its pixels it is still read, with none.
    fn jv(pixels: bool) -> Vec<u8> {
        let mut file = b"JV\x00\x00 Compression by John M Phillips Copyright (C) 1995 \
                         The Bitmap Brothers Ltd."
            .to_vec();
        let mut words: Vec<u8> = vec![0];
        for value in [64u16, 64, 1, 40] {
            words.extend(value.to_le_bytes());
        }
        words.extend([0; 4]);
        words.extend(8000u16.to_le_bytes());
        words.extend([0; 10]);
        for value in [769u32, 0, 1] {
            words.extend(value.to_le_bytes());
        }
        words.extend([1, 0, 2, 0]);
        file.extend(words);
        if pixels {
            file.push(0x40);
            file.extend((0..256).flat_map(|i| [i as u8, 255 - i as u8, (i * 2) as u8]));
        }
        file
    }

    fn written(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fvid-jv-{name}-{}.jv", std::process::id()));
        std::fs::write(&path, bytes).expect("the fixture is written for the reading tests");
        path
    }

    /// JV's demuxer answers the read after its last frame with `AVERROR_INVALIDDATA`, so every
    /// well-formed file of this format ends in damage. The strict video path used to call that a
    /// failed decode; it now keeps the frames and reports what it skipped.
    #[test]
    fn a_demuxer_that_ends_its_stream_with_an_error_keeps_the_frames_it_gave() {
        let path = written("whole", &jv(true));
        let stats = decode_video_transformed(&path, DecodeTransform::default())
            .expect("the damaged end of stream is not a failed decode");
        assert_eq!(
            (stats.video_frames, stats.width, stats.height),
            (1, 64, 64),
            "the one frame the file declares arrives"
        );
        assert!(
            stats.decode_errors > 0,
            "the tolerance ran, and says so: {:?}",
            stats.decode_errors
        );
        std::fs::remove_file(&path).ok();
    }

    /// The control on that tolerance: the same header without its pixels is not read as a covered
    /// file. Fewer frames than the whole fixture is the only outcome that keeps the counter above
    /// from being a way to declare any damaged input decoded.
    #[test]
    fn a_stream_that_yielded_nothing_before_the_damage_is_still_a_failure_of_count() {
        let path = written("stripped", &jv(false));
        let stats = decode_video_transformed(&path, DecodeTransform::default())
            .expect("the demuxer ends this file at EOF rather than in damage");
        assert_eq!(
            stats.video_frames, 0,
            "a file with no pixels must not read as the whole one: {:?}",
            (stats.video_frames, stats.decode_errors)
        );
        std::fs::remove_file(&path).ok();
    }
}
