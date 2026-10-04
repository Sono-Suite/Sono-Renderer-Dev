//! Native desktop frontend. Widgets edit RenderConfig; the export worker calls the backend.
use crate::{
    export_control::{Cancelled, ExportControl},
    export_progress::ExportProgress,
    render::{LevelOptionValue, RenderBackend, RenderConfig},
};
use anyhow::{bail, Result};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread::JoinHandle,
    time::Duration,
};

// The hook captures the origin before unwinding. A catch-site backtrace would
// only show the worker recovery code. Other threads keep their existing hook.
thread_local! {
    static WORKER_PANIC: std::cell::RefCell<Option<Option<String>>> = const {
        std::cell::RefCell::new(None)
    };
}
fn catch_export_panic<T>(export: impl FnOnce() -> Result<T>) -> Result<T> {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let handled = WORKER_PANIC.with(|slot| {
                let mut slot = slot.borrow_mut();
                if let Some(diagnostic) = slot.as_mut() {
                    let message = panic_message(info.payload());
                    let location = info.location().map(|l| l.to_string()).unwrap_or_default();
                    let trace = std::backtrace::Backtrace::capture();
                    let mut text =
                        format!("Export worker panicked: {message}\nPanic origin: {location}");
                    if trace.status() == std::backtrace::BacktraceStatus::Captured {
                        text.push_str(&format!("\n{trace}"));
                    }
                    *diagnostic = Some(text);
                    true
                } else {
                    false
                }
            });
            if !handled {
                previous(info);
            }
        }));
    });
    WORKER_PANIC.with(|slot| *slot.borrow_mut() = Some(None));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(export));
    let diagnostic = WORKER_PANIC.with(|slot| slot.borrow_mut().take().flatten());
    match result {
        Ok(result) => result,
        Err(payload) => Err(anyhow::anyhow!(diagnostic.unwrap_or_else(|| {
            format!(
                "Export worker panicked: {}",
                panic_message(payload.as_ref())
            )
        }))),
    }
}
fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload")
}

pub fn launch() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn FreeConsole() -> i32;
        }
        FreeConsole();
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 790.0])
            .with_min_inner_size([820.0, 650.0])
            .with_position([40.0, 40.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Sono-Renderer",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            let mut app = Desktop::default();
            app.parent = cc.winit_window().cloned();
            Ok(Box::new(app))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Native GUI startup failed: {error}"));
    if let Err(error) = &result {
        rfd::MessageDialog::new()
            .set_title("Sono-Renderer")
            .set_description(error.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
    result
}

#[derive(Default)]
struct LiveState {
    progress: Option<ExportProgress>,
    logs: VecDeque<String>,
}
impl LiveState {
    fn log(&mut self, message: String) {
        if self.logs.len() == 200 {
            self.logs.pop_front();
        }
        self.logs.push_back(message);
    }
    fn progress(&mut self, event: ExportProgress) {
        if self
            .progress
            .as_ref()
            .is_none_or(|old| old.phase != event.phase)
        {
            self.log(format!("{:?}", event.phase));
        }
        self.progress = Some(event);
    }
}
struct Job {
    control: ExportControl,
    result: mpsc::Receiver<Result<String>>,
    worker: JoinHandle<()>,
}
struct Desktop {
    parent: Option<Arc<winit::window::Window>>,
    config: RenderConfig,
    tab: usize,
    live: Arc<Mutex<LiveState>>,
    job: Option<Job>,
    summary: String,
    error: bool,
    options: Vec<serde_json::Value>,
    option_job: Option<mpsc::Receiver<Result<Vec<serde_json::Value>>>>,
    options_engine: PathBuf,
    arbitrary_index: usize,
    arbitrary_value: f64,
    close_pending: bool,
}
impl Default for Desktop {
    fn default() -> Self {
        Self {
            parent: None,
            config: serde_json::from_value(serde_json::json!({
                "engine":"", "resources":"", "level":"", "width":1920,
                "height":1080, "fps":60, "duration":20.0
            }))
            .unwrap(),
            tab: 0,
            live: Default::default(),
            job: None,
            summary: "Select your project files to begin.".into(),
            error: false,
            options: Vec::new(),
            option_job: None,
            options_engine: PathBuf::new(),
            arbitrary_index: 0,
            arbitrary_value: 0.0,
            close_pending: false,
        }
    }
}
fn apply_selection(path: &mut PathBuf, selection: Option<PathBuf>) {
    if let Some(selected) = selection {
        *path = selected;
    }
}
fn path_row(
    ui: &mut egui::Ui,
    label: &str,
    path: &mut PathBuf,
    extensions: &[&str],
    save: bool,
    parent: Option<&winit::window::Window>,
) {
    ui.label(label);
    let mut text = path.to_string_lossy().into_owned();
    if ui
        .add_sized(
            [
                (ui.ctx().content_rect().width() - 290.0).clamp(300.0, 560.0),
                24.0,
            ],
            egui::TextEdit::singleline(&mut text),
        )
        .changed()
    {
        *path = PathBuf::from(text);
    }
    if ui
        .button(if save { "Save as..." } else { "Browse..." })
        .clicked()
    {
        let mut dialog = rfd::FileDialog::new()
            .set_title(label)
            .add_filter(label, extensions)
            .add_filter("All files", &["*"]);
        if let Some(parent) = parent {
            dialog = dialog.set_parent(parent);
        }
        if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
            dialog = dialog.set_directory(parent);
        }
        let selected = if save {
            dialog
                .set_file_name(if path.as_os_str().is_empty() {
                    "render.mp4"
                } else {
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("render.mp4")
                })
                .save_file()
        } else {
            dialog.pick_file()
        };
        apply_selection(path, selected);
    }
    ui.end_row();
}
fn optional_path_row(
    ui: &mut egui::Ui,
    label: &str,
    path: &mut Option<PathBuf>,
    ext: &[&str],
    save: bool,
    parent: Option<&winit::window::Window>,
) {
    let mut value = path.clone().unwrap_or_default();
    path_row(ui, label, &mut value, ext, save, parent);
    *path = (!value.as_os_str().is_empty()).then_some(value);
}
fn obvious_validation(config: &RenderConfig) -> Result<()> {
    config.validate()?;
    for (name, path) in [
        ("Engine", &config.engine),
        ("Level", &config.level),
        ("Skin / SCP", &config.resources),
    ] {
        if !path.is_file() {
            bail!("{name}: select an accessible input file");
        }
    }
    if config
        .output
        .as_ref()
        .is_none_or(|p| p.as_os_str().is_empty())
    {
        bail!("Choose an output MP4 with Save as...");
    }
    if config.layers.bgm && config.music.as_ref().is_none_or(|p| !p.is_file()) {
        bail!("BGM is enabled: select music or turn BGM off");
    }
    if config.width % 2 != 0 || config.height % 2 != 0 {
        bail!("MP4 requires even width and height");
    }
    Ok(())
}
impl Desktop {
    fn start(&mut self, ctx: &egui::Context) {
        if self.job.is_some() {
            return;
        }
        if let Err(error) = obvious_validation(&self.config) {
            self.summary = error.to_string();
            self.error = true;
            self.live.lock().unwrap().log(format!("{error:#}"));
            return;
        }
        *self.live.lock().unwrap() = LiveState::default();
        let log = self.live.clone();
        let wake = ctx.clone();
        let control = ExportControl::with_log(move |message| {
            log.lock().unwrap().log(message);
            wake.request_repaint();
        });
        let token = control.clone();
        let live = self.live.clone();
        let config = self.config.clone();
        let wake = ctx.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let progress = live.clone();
            let paint = wake.clone();
            let result = catch_export_panic(|| {
                config.render_video_controlled(
                    Box::new(move |event| {
                        progress.lock().unwrap().progress(event);
                        paint.request_repaint();
                    }),
                    token,
                )
            })
            .map_err(|error| {
                let phase = live.lock().unwrap().progress.as_ref().map(|p| p.phase);
                error.context(format!(
                    "GUI export during {phase:?}; engine={}, level={}, output={}",
                    config.engine.display(),
                    config.level.display(),
                    config
                        .output
                        .as_deref()
                        .unwrap_or_else(|| std::path::Path::new(""))
                        .display()
                ))
            });
            let result = result.map(|report| {
                format!(
                    "Finished: {} frames, {:.2}s. {}",
                    report.submitted_frames,
                    report.timeline.duration,
                    config.output.unwrap().display()
                )
            });
            let _ = tx.send(result);
            wake.request_repaint();
        });
        self.job = Some(Job {
            control,
            result: rx,
            worker,
        });
        self.error = false;
        self.summary = "Export running".into();
    }
    fn poll(&mut self) {
        if let Some(job) = &self.job {
            if let Ok(result) = job.result.try_recv() {
                let job = self.job.take().unwrap();
                let _ = job.worker.join();
                match result {
                    Ok(text) => {
                        self.summary = text;
                        self.error = false;
                        self.live.lock().unwrap().log(self.summary.clone());
                    }
                    Err(error) => {
                        self.error = !job.control.is_cancelled() && !error.is::<Cancelled>();
                        self.summary = if job.control.is_cancelled() || error.is::<Cancelled>() {
                            "Cancelled. No export was published.".into()
                        } else {
                            error
                                .chain()
                                .last()
                                .unwrap()
                                .to_string()
                                .lines()
                                .next()
                                .unwrap_or("Export failed")
                                .to_owned()
                        };
                        self.live.lock().unwrap().log(format!("{error:#}"));
                    }
                }
            }
        }
        if let Some(rx) = &self.option_job {
            if let Ok(result) = rx.try_recv() {
                self.option_job = None;
                match result {
                    Ok(options) => self.options = options,
                    Err(e) => self
                        .live
                        .lock()
                        .unwrap()
                        .log(format!("Option discovery: {e:#}")),
                }
            }
        }
        if self.config.engine != self.options_engine && self.option_job.is_none() {
            self.options_engine = self.config.engine.clone();
            self.options.clear();
            if self.options_engine.is_file() {
                let engine = self.options_engine.clone();
                let (tx, rx) = mpsc::sync_channel(1);
                self.option_job = Some(rx);
                std::thread::spawn(move || {
                    let result = crate::formats::load_engine(&engine).map(|package| {
                        package.configuration["options"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                    });
                    let _ = tx.send(result);
                });
            }
        }
    }
    fn inputs(&mut self, ui: &mut egui::Ui) {
        ui.heading("Project inputs");
        ui.label("Choose files with native Open / Save dialogs. Separate resource packages are optional.");
        ui.add_space(12.0);
        egui::Grid::new("inputs")
            .spacing([12.0, 14.0])
            .show(ui, |ui| {
                path_row(
                    ui,
                    "Engine ZIP",
                    &mut self.config.engine,
                    &["zip"],
                    false,
                    self.parent.as_deref(),
                );
                path_row(
                    ui,
                    "Level",
                    &mut self.config.level,
                    &["gz", "data", "json", "zip"],
                    false,
                    self.parent.as_deref(),
                );
                path_row(
                    ui,
                    "Skin / SCP",
                    &mut self.config.resources,
                    &["scp", "zip"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "Particle SCP (optional)",
                    &mut self.config.resource_overrides.particles,
                    &["scp", "zip"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "Music / BGM",
                    &mut self.config.music,
                    &["mp3", "ogg", "wav", "flac", "m4a"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "MV (optional)",
                    &mut self.config.mv,
                    &["mp4", "webm", "mkv", "mov"],
                    false,
                    self.parent.as_deref(),
                );
                optional_path_row(
                    ui,
                    "Output MP4",
                    &mut self.config.output,
                    &["mp4"],
                    true,
                    self.parent.as_deref(),
                );
            });
        ui.add_space(12.0);
        ui.collapsing("Resource names / separate audio and background packages", |ui| {
            ui.label("Defaults come from the engine. Resource names are SCP entry names, not filenames.");
            egui::Grid::new("resources").show(ui, |ui| {
                optional_path_row(ui,"SFX SCP (optional)",&mut self.config.resource_overrides.effects,&["scp","zip"],false, self.parent.as_deref());
                optional_path_row(ui,"Background SCP (optional)",&mut self.config.resource_overrides.background,&["scp","zip"],false, self.parent.as_deref());
            });
            for (label,value) in [("Skin resource name",&mut self.config.skin),("Particle resource name",&mut self.config.resource_overrides.particle_name)] { ui.horizontal(|ui| {ui.label(label); let mut text=value.clone().unwrap_or_default(); if ui.text_edit_singleline(&mut text).changed() {*value=(!text.is_empty()).then_some(text);} }); }
        });
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Render range");
        ui.horizontal(|ui| {
            ui.label("Start (Watch seconds)");
            ui.add(
                egui::DragValue::new(&mut self.config.start_time)
                    .speed(0.1)
                    .range(0.0..=86400.0),
            );
            ui.checkbox(&mut self.config.whole_chart, "Whole Chart");
        });
        ui.add_enabled_ui(!self.config.whole_chart, |ui| {
            ui.horizontal(|ui| {
                ui.label("Duration (seconds)");
                ui.add(
                    egui::DragValue::new(&mut self.config.duration)
                        .speed(0.1)
                        .range(0.01..=86400.0),
                );
            });
        });
        if self.config.whole_chart {
            ui.label("Uses the existing chart-end inference. Manual duration is disabled.");
        }
        ui.add_space(16.0);
        ui.heading("Video");
        ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(egui::DragValue::new(&mut self.config.width).range(2..=8192));
            ui.label("Height");
            ui.add(egui::DragValue::new(&mut self.config.height).range(2..=8192));
            ui.label("FPS");
            ui.add(egui::DragValue::new(&mut self.config.fps).range(1..=240));
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.config.backend, RenderBackend::Wgpu, "WGPU");
            ui.selectable_value(&mut self.config.backend, RenderBackend::Cpu, "CPU");
            ui.checkbox(&mut self.config.frame_pipeline, "Frame pipeline");
        });
        ui.add_space(16.0);
        ui.heading("MV background");
        ui.add_enabled(
            self.config.mv.is_some(),
            egui::Checkbox::new(&mut self.config.mv_enabled, "Use selected MV"),
        );
        ui.add_enabled(
            self.config.mv.is_some() && self.config.mv_enabled,
            egui::Slider::new(&mut self.config.mv_background, 0.0..=100.0).text("Background %"),
        );
        ui.label(
            "Presentation brightness toward black, before gameplay. Media timing is unchanged.",
        );
        ui.add_space(16.0);
        ui.heading("Layers");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.config.layers.particles, "Particles");
            ui.checkbox(&mut self.config.layers.sfx, "SFX");
            ui.checkbox(&mut self.config.layers.bgm, "BGM");
        });
        ui.collapsing("Validation", |ui| {
            ui.checkbox(
                &mut self.config.validate_determinism,
                "Full determinism preflight (additional Watch playback)",
            );
        });
    }
    fn options(&mut self, ui: &mut egui::Ui) {
        ui.heading("Engine options");
        if self.option_job.is_some() {
            ui.spinner();
            ui.label("Reading engine metadata...");
        }
        if self.options.is_empty() {
            ui.label("Select an engine to discover named options.");
        }
        for (index, option) in self.options.iter().enumerate() {
            let def = option["def"].as_f64().unwrap_or(0.0);
            let current = self
                .config
                .level_options
                .iter()
                .find(|v| v.index == index)
                .map_or(def, |v| v.value);
            let mut value = current;
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [210.0, 24.0],
                        egui::Label::new(option_label(
                            option["name"].as_str().unwrap_or("Unnamed option"),
                        )),
                    );
                    match option["type"].as_str().unwrap_or("") {
                        "toggle" => {
                            let mut on = value != 0.0;
                            if ui.checkbox(&mut on, "").changed() {
                                value = if on { 1.0 } else { 0.0 };
                            }
                        }
                        "select" => {
                            egui::ComboBox::from_id_salt("value")
                                .width(180.0)
                                .selected_text(option_label(
                                    option["values"]
                                        .get(if value >= 0.0 && value.fract() == 0.0 {
                                            value as usize
                                        } else {
                                            usize::MAX
                                        })
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("Custom"),
                                ))
                                .show_ui(ui, |ui| {
                                    if let Some(values) = option["values"].as_array() {
                                        for (i, name) in values.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut value,
                                                i as f64,
                                                option_label(name.as_str().unwrap_or("Unnamed")),
                                            );
                                        }
                                    }
                                });
                        }
                        "slider" => {
                            if let (Some(min), Some(max)) =
                                (option["min"].as_f64(), option["max"].as_f64())
                            {
                                if min > max {
                                    ui.label("Invalid metadata range; use numeric input");
                                } else {
                                    ui.add(
                                        egui::Slider::new(&mut value, min..=max)
                                            .clamping(egui::SliderClamping::Never)
                                            .step_by(
                                                option["step"]
                                                    .as_f64()
                                                    .filter(|v| *v > 0.0)
                                                    .unwrap_or(0.01),
                                            ),
                                    );
                                }
                            }
                        }
                        _ => {}
                    }
                    ui.add(egui::DragValue::new(&mut value).speed(0.1));
                    if ui.small_button("Default").clicked() {
                        value = def;
                    }
                });
            });
            if value != current {
                set_option(&mut self.config, index, value, def);
            }
        }
        ui.separator();
        ui.label("Arbitrary numeric override (values are not constrained by metadata)");
        ui.horizontal(|ui| {
            ui.label("Index");
            ui.add(egui::DragValue::new(&mut self.arbitrary_index));
            ui.label("Value");
            ui.add(egui::DragValue::new(&mut self.arbitrary_value));
            if ui.button("Set override").clicked() {
                set_option(
                    &mut self.config,
                    self.arbitrary_index,
                    self.arbitrary_value,
                    f64::NAN,
                );
            }
        });
        ui.label(format!(
            "{} explicit overrides",
            self.config.level_options.len()
        ));
    }
}
fn option_label(text: &str) -> String {
    text.strip_prefix("##LOCALIZE:")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .and_then(|value| value["en"].as_str().map(str::to_owned))
        .unwrap_or_else(|| text.to_owned())
}

fn set_option(config: &mut RenderConfig, index: usize, value: f64, default: f64) {
    config.level_options.retain(|v| v.index != index);
    if value != default {
        config.level_options.push(LevelOptionValue { index, value });
    }
}
impl eframe::App for Desktop {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if ctx.input(|i| i.viewport().close_requested()) && self.job.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_pending = true;
            self.job.as_ref().unwrap().control.cancel();
            self.summary = "Cancelling before closing...".into();
        }
        if self.close_pending && self.job.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.job.is_some() || self.option_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(root, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Sono-Renderer");
                ui.label("Native video export");
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (i, name) in ["Project / inputs", "Engine options", "Render / video"]
                    .iter()
                    .enumerate()
                {
                    ui.selectable_value(&mut self.tab, i, *name);
                }
            });
            ui.separator();
            let available = (ui.available_height() - 225.0).max(240.0);
            egui::ScrollArea::vertical()
                .max_height(available)
                .id_salt("settings")
                .show(ui, |ui| {
                    ui.add_enabled_ui(self.job.is_none(), |ui| match self.tab {
                        0 => self.inputs(ui),
                        1 => self.options(ui),
                        _ => self.settings(ui),
                    });
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.job.is_none(),
                        egui::Button::new("Render MP4").min_size(egui::vec2(140.0, 36.0)),
                    )
                    .clicked()
                {
                    self.start(ui.ctx());
                }
                if ui
                    .add_enabled(
                        self.job.is_some(),
                        egui::Button::new("Cancel").min_size(egui::vec2(90.0, 36.0)),
                    )
                    .clicked()
                {
                    self.job.as_ref().unwrap().control.cancel();
                    self.summary = "Cancellation requested...".into();
                }
            });
            ui.colored_label(
                if self.error {
                    egui::Color32::LIGHT_RED
                } else {
                    egui::Color32::LIGHT_GREEN
                },
                &self.summary,
            );
            let live = self.live.lock().unwrap();
            if let Some(p) = &live.progress {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{:?} | {} / {} frames | elapsed {:.1}s",
                        p.phase,
                        p.completed_frames,
                        p.total_frames.map_or_else(|| "?".into(), |v| v.to_string()),
                        p.elapsed_seconds
                    ));
                    if let Some(fps) = p.frames_per_second {
                        ui.label(format!("{fps:.1} fps"));
                    }
                    if let Some(eta) = p.eta_seconds {
                        ui.label(format!("ETA {eta:.1}s"));
                    }
                });
                if let Some(percent) = p.percentage {
                    ui.add(egui::ProgressBar::new((percent / 100.0) as f32).show_percentage());
                } else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Preparing / determining export length");
                    });
                }
            }
            ui.collapsing("Export log", |ui| {
                if ui.button("Copy log").clicked() {
                    ui.ctx()
                        .copy_text(live.logs.iter().cloned().collect::<Vec<_>>().join("\n"));
                }
                egui::ScrollArea::vertical()
                    .max_height(95.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &live.logs {
                            ui.label(line);
                        }
                    });
            });
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_picker_preserves_spaced_unicode_path() {
        let mut p = PathBuf::from("C:/my files/\u{8c31}\u{9762}.data");
        let old = p.clone();
        apply_selection(&mut p, None);
        assert_eq!(p, old);
        apply_selection(&mut p, Some(PathBuf::from("other.scp")));
        assert_eq!(p, PathBuf::from("other.scp"));
    }
    #[test]
    fn arbitrary_override_is_preserved_and_default_removes_override() {
        let mut app = Desktop::default();
        set_option(&mut app.config, 73, -2.25, 0.0);
        assert_eq!(app.config.level_options[0].value, -2.25);
        set_option(&mut app.config, 73, 0.0, 0.0);
        assert!(app.config.level_options.is_empty());
    }
    #[test]
    fn progress_is_typed_and_logs_are_bounded() {
        let mut state = LiveState::default();
        state.progress(ExportProgress {
            phase: crate::export_progress::ExportPhase::Rendering,
            completed_frames: 8,
            total_frames: Some(100),
            percentage: Some(8.0),
            elapsed_seconds: 2.0,
            frames_per_second: Some(4.0),
            eta_seconds: Some(23.0),
        });
        assert_eq!(state.progress.as_ref().unwrap().completed_frames, 8);
        for i in 0..300 {
            state.log(i.to_string());
        }
        assert_eq!(state.logs.len(), 200);
    }
    #[test]
    fn running_job_blocks_duplicate_start_and_reports_error_or_cancel() {
        for cancelled in [false, true] {
            let mut app = Desktop::default();
            let (tx, rx) = mpsc::sync_channel(1);
            let control = ExportControl::default();
            app.job = Some(Job {
                control: control.clone(),
                result: rx,
                worker: std::thread::spawn(|| {}),
            });
            let summary = app.summary.clone();
            app.start(&egui::Context::default());
            assert_eq!(app.summary, summary);
            assert!(app.job.is_some());
            if cancelled {
                control.cancel();
                tx.send(Err(Cancelled.into())).unwrap();
            } else {
                tx.send(Err(
                    anyhow::anyhow!("FFmpeg failed").context("GUI export during Rendering")
                ))
                .unwrap();
            }
            app.poll();
            assert!(app.job.is_none());
            assert_eq!(app.live.lock().unwrap().logs.len(), 1);
            assert_eq!(app.error, !cancelled);
            assert!(app.summary.contains(if cancelled {
                "Cancelled"
            } else {
                "FFmpeg failed"
            }));
        }
    }

    #[test]
    fn localized_options_use_supplied_english_metadata() {
        assert_eq!(
            option_label(r##"##LOCALIZE:{"en":"Standard","ja":"other"}"##),
            "Standard"
        );
        assert_eq!(option_label("#NOTE_SPEED"), "#NOTE_SPEED");
        assert_eq!(option_label("##LOCALIZE:invalid"), "##LOCALIZE:invalid");
    }

    #[test]
    fn obvious_invalid_input_is_reported() {
        let app = Desktop::default();
        assert!(obvious_validation(&app.config)
            .unwrap_err()
            .to_string()
            .contains("Engine"));
    }
    #[test]
    fn worker_panic_preserves_payload_origin_and_allows_recovery() {
        let error = std::thread::spawn(|| {
            catch_export_panic::<()>(|| panic!("diagnostic sentinel"))
                .unwrap_err()
                .to_string()
        })
        .join()
        .unwrap();
        assert!(error.contains("diagnostic sentinel"));
        assert!(error.contains(file!()));
        assert_eq!(catch_export_panic(|| Ok(42)).unwrap(), 42);
        let non_string = catch_export_panic::<()>(|| std::panic::panic_any(7u32)).unwrap_err();
        assert!(non_string.to_string().contains("non-string panic payload"));
    }
}
