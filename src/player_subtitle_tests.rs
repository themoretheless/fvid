//! Exercise the caption painter used by the native player, with checked-in SRT.
use super::*;

fn paint(
    name: &str,
    frame: Rect,
    controls: Option<Rect>,
) -> (
    Pos2,
    std::sync::Arc<egui::Galley>,
    Vec<egui::ClippedPrimitive>,
) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-painting")
        .join(format!("{name}.srt"));
    let cues = subtitles::load(&path).unwrap();
    assert_eq!(cues[0].start, Duration::from_secs(1));
    assert_eq!(cues[0].end, Duration::from_secs(9));
    let ctx = egui::Context::default();
    let mut result = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(frame),
            ..Default::default()
        },
        |ui| {
            let painter = ui.painter().with_clip_rect(frame);
            result = Some(paint_subtitle_block(
                &painter,
                cues[0].text.clone(),
                FontId::proportional(20.0),
                frame,
                40.0,
                controls,
            ));
        },
    );
    output.textures_delta.clear();
    let (at, galley) = result.unwrap();
    let text_shapes = output
        .shapes
        .into_iter()
        .filter(|s| matches!(s.shape, egui::Shape::Text(_)))
        .collect();
    let meshes = ctx.tessellate(text_shapes, output.pixels_per_point);
    (at, galley, meshes)
}

#[test]
fn subtitle_paint_preserves_authored_line_breaks() {
    let frame = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0));
    let (_, galley, _) = paint("two-lines", frame, None);
    assert_eq!(galley.text(), "Первая α\nстрока");
    assert_eq!(galley.rows.len(), 2);
}

#[test]
fn subtitle_paint_centers_each_wrapped_row_inside_video() {
    for width in [320.0, 640.0, 1280.0] {
        let frame = Rect::from_min_size(Pos2::new(73.0, 51.0), Vec2::new(width, 720.0));
        let (at, galley, _) = paint("wrapped", frame, None);
        assert!(galley.rows.len() > 1);
        for row in &galley.rows {
            let rect = row.rect().translate(at.to_vec2());
            assert!(
                (rect.center().x - frame.center().x).abs() < 1.0,
                "{width}: {rect:?}"
            );
            assert!(
                rect.left() >= frame.left() && rect.right() <= frame.right(),
                "{rect:?}"
            );
        }
        assert!((at.y + galley.rect.bottom() - (frame.bottom() - 40.0)).abs() < 1.0);
    }
}

#[test]
fn subtitle_paint_emits_black_outline_and_white_fill() {
    let (_, _, primitives) = paint(
        "outline",
        Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 480.0)),
        None,
    );
    let colors: Vec<_> = primitives
        .iter()
        .flat_map(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(mesh) => {
                mesh.vertices.iter().map(|v| v.color).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect();
    assert!(colors.contains(&Color32::WHITE), "missing text fill");
    assert!(
        colors.contains(&Color32::BLACK),
        "outline fallback does not replace explicit white glyphs"
    );
}

#[test]
fn subtitle_paint_stays_above_visible_controls() {
    let frame = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0));
    let panel = Rect::from_min_max(Pos2::new(28.0, 576.0), Pos2::new(1252.0, 700.0));
    let (at, galley, _) = paint("two-lines", frame, Some(panel));
    assert!(at.y + galley.rect.bottom() <= panel.top() - 12.0);
}

#[test]
fn subtitle_paint_native_video_seek_and_cue_end() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-painting");
    let mut player = Player {
        queue: vec![root.join("video.webm")],
        subtitle_file: Some(root.join("two-lines.srt")),
        no_audio: true,
        asynchronous_open: false,
        ..Default::default()
    };
    player.play_index(0);
    let ctx = egui::Context::default();
    for millis in [1500, 9500, 500, 1500] {
        player.seek_time(Duration::from_millis(millis));
        let until = Instant::now() + Duration::from_secs(10);
        while player.presented.is_none() || player.seek_preview || player.buffering {
            assert!(player.error.is_none(), "{:?}", player.error);
            assert!(Instant::now() < until, "native seek stalled at {millis}");
            let mut output = ctx.run_ui(Default::default(), |ui| player.present(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            player.presented.as_ref().unwrap().generation,
            player.playback.as_ref().unwrap().generation()
        );
        let line = player.subtitle_line(Duration::from_millis(millis));
        if millis == 1500 {
            assert_eq!(line, Some("Первая α\nстрока"));
            let frame = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0));
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let (_, galley) = paint_subtitle_block(
                    ui.painter(),
                    line.unwrap().to_owned(),
                    FontId::proportional(20.0),
                    frame,
                    40.0,
                    None,
                );
                assert_eq!(
                    galley.rows.len(),
                    2,
                    "native cue was flattened by the painter"
                );
            });
            output.textures_delta.clear();
        } else {
            assert_eq!(line, None, "caption outside its authored interval");
        }
    }
}
