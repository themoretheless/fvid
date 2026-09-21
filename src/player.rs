//! Window presentation for FVid's own video reader.
use crate::playback_native::NativeReader;
use eframe::egui;
use std::{fs::File, io::BufReader, path::PathBuf, time::Instant};

const BUFFER_BUDGET: usize = 256 * 1024 * 1024;

/// Open an empty player or a local Y4M / supported MP4 AVC file.
pub fn run(path: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = Player::default();
    if let Some(path) = path {
        app.open(path)?;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_drag_and_drop(true),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native("FVid", options, Box::new(move |_| Ok(Box::new(app))))
        .map_err(|error| error.to_string())?;
    Ok(())
}

struct Player {
    reader: Option<NativeReader<BufReader<File>>>,
    texture: Option<egui::TextureHandle>,
    name: String,
    error: Option<String>,
    paused: bool,
    ended: bool,
    dirty: bool,
    deadline: Instant,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            reader: None,
            texture: None,
            name: String::new(),
            error: None,
            paused: false,
            ended: false,
            dirty: false,
            deadline: Instant::now(),
        }
    }
}

impl Player {
    fn open(&mut self, path: PathBuf) -> crate::Result<()> {
        let mut reader = NativeReader::new(BufReader::new(File::open(&path)?), BUFFER_BUDGET)?;
        if !reader.read_frame()? {
            return Err(crate::invalid("video has no frames"));
        }
        self.deadline = Instant::now() + reader.frame_period();
        self.reader = Some(reader);
        self.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.error = None;
        self.paused = false;
        self.ended = false;
        self.dirty = true;
        Ok(())
    }

    fn try_open(&mut self, path: PathBuf) {
        if let Err(error) = self.open(path) {
            self.error = Some(error.to_string());
        }
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if let Some(reader) = &self.reader {
            self.deadline = Instant::now() + reader.frame_period();
        }
    }
}

impl eframe::App for Player {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_pause();
        }
        if let Some(path) =
            ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()))
        {
            self.try_open(path);
        }
        if let Some(reader) = &mut self.reader {
            if !self.paused && !self.ended {
                let now = Instant::now();
                if now >= self.deadline {
                    match reader.read_frame() {
                        Ok(true) => self.dirty = true,
                        Ok(false) => self.ended = true,
                        Err(error) => {
                            self.error = Some(error.to_string());
                            self.ended = true;
                        }
                    }
                    self.deadline += reader.frame_period();
                    // Bound catch-up after a stalled UI instead of decoding an unbounded backlog.
                    if self.deadline < now {
                        self.deadline = now + reader.frame_period();
                    }
                }
                ctx.request_repaint_after(self.deadline.saturating_duration_since(Instant::now()));
            }
            if self.dirty {
                let image = egui::ColorImage::from_rgb(reader.dimensions(), reader.rgb());
                if let Some(texture) = &mut self.texture {
                    texture.set(image, egui::TextureOptions::LINEAR);
                } else {
                    self.texture =
                        Some(ctx.load_texture("video", image, egui::TextureOptions::LINEAR));
                }
                self.dirty = false;
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::Panel::top("controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Video", &["y4m", "mp4", "mov"])
                        .pick_file()
                    {
                        self.try_open(path);
                    }
                }
                if ui
                    .add_enabled(
                        self.reader.is_some() && !self.ended,
                        egui::Button::new(if self.paused { "Play" } else { "Pause" }),
                    )
                    .clicked()
                {
                    self.toggle_pause();
                }
                if ui
                    .add_enabled(self.reader.is_some(), egui::Button::new("Restart"))
                    .clicked()
                {
                    if let Some(reader) = &mut self.reader {
                        match reader.rewind() {
                            Ok(()) => {
                                self.ended = false;
                                self.paused = false;
                                self.deadline = Instant::now();
                            }
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                }
                ui.label(&self.name);
                if self.ended {
                    ui.label("End");
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(texture) = &self.texture {
                ui.add(egui::Image::new(texture).fit_to_exact_size({
                    let size = texture.size_vec2();
                    size * (ui.available_width() / size.x)
                        .min(ui.available_height() / size.y)
                        .max(0.0)
                }));
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("Open or drop a Y4M or MP4 video");
                });
            }
        });
    }
}
