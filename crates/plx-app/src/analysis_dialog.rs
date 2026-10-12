//! PrePoMax's analysis dialog: creates an analysis or edits one, its name and the arguments
//! CalculiX is started with. The executable and the work directory are shown as PrePoMax
//! shows them; they are settings of the program.

use egui::Ui;
use plx_model::{AnalysisJob, analysis_name_error};

use crate::settings::Solver;

pub struct AnalysisDialog {
    pub draft: AnalysisJob,
    /// The analysis being edited, by index; `None` creates a new one.
    pub index: Option<usize>,
    /// The arguments as typed.
    arguments: String,
    /// The analysis is running: its name names the files CalculiX writes now.
    running: bool,
}

pub enum AnalysisDialogResult {
    Open,
    Ok,
    Cancel,
    /// Open the CalculiX settings, where the executable and the work directory are set.
    Settings,
}

impl AnalysisDialog {
    /// A new analysis with the next free default name.
    pub fn create(analyses: &plx_model::Analyses) -> Self {
        Self::with(AnalysisJob::new(analyses.next_name()), None, false)
    }

    pub fn edit(analyses: &plx_model::Analyses, index: usize, running: bool) -> Option<Self> {
        let job = analyses.get(index)?.clone();
        Some(Self::with(job, Some(index), running))
    }

    fn with(draft: AnalysisJob, index: Option<usize>, running: bool) -> Self {
        Self {
            arguments: draft.arguments_text().to_owned(),
            draft,
            index,
            running,
        }
    }

    fn title(&self) -> String {
        let action = if self.index.is_some() {
            "Edit"
        } else {
            "Create"
        };
        format!("{action} Analysis: {}", self.draft.name)
    }

    /// Why the draft cannot be taken; `analyses` are those of the model.
    pub fn error(&self, analyses: &plx_model::Analyses) -> Option<String> {
        let others = (analyses.iter().enumerate())
            .filter(|(i, _)| Some(*i) != self.index)
            .map(|(_, a)| a.name.as_str());
        analysis_name_error(&self.draft.name, others)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        analyses: &plx_model::Analyses,
        solver: &Solver,
    ) -> AnalysisDialogResult {
        let mut result = AnalysisDialogResult::Open;
        let mut open = true;
        let error = self.error(analyses);
        egui::Window::new(self.title())
            .id(egui::Id::new("analysis dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.set_width(420.0);
                ui.strong("Data");
                egui::Grid::new("analysis form")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| self.form(ui, solver));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.weak("The executable and the work directory are set in the");
                    if ui.link("CalculiX settings").clicked() {
                        result = AnalysisDialogResult::Settings;
                    }
                });
                if let Some(error) = &error {
                    ui.add(
                        egui::Label::new(egui::RichText::new(error).color(crate::style::error()))
                            .wrap(),
                    );
                }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Cancel").clicked() {
                        result = AnalysisDialogResult::Cancel;
                    }
                    if ui
                        .add_enabled(error.is_none(), egui::Button::new("OK"))
                        .clicked()
                    {
                        result = AnalysisDialogResult::Ok;
                    }
                });
            });
        self.draft.set_arguments(&self.arguments);
        if error.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            result = AnalysisDialogResult::Ok;
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            result = AnalysisDialogResult::Cancel;
        }
        result
    }

    fn form(&mut self, ui: &mut Ui, solver: &Solver) {
        let old_name = self.draft.name.clone();
        ui.label("Name")
            .on_hover_text("Name of the analysis and of its files.");
        let name = ui.add_enabled(
            !self.running,
            egui::TextEdit::singleline(&mut self.draft.name).desired_width(f32::INFINITY),
        );
        name.on_disabled_hover_text("The analysis is running: its name names the files.");
        ui.end_row();
        // The default arguments follow the name, as in PrePoMax.
        if self.arguments.trim() == old_name {
            self.arguments = self.draft.name.clone();
        }
        ui.label("FEM solver")
            .on_hover_text("FEM solver to be used for the analysis.");
        ui.label("CalculiX");
        ui.end_row();
        let executable = solver.job_solver().executable;
        ui.label("Executable")
            .on_hover_text("FEM solver executable file.");
        ui.add(egui::Label::new(executable.display().to_string()).truncate())
            .on_hover_text(executable.display().to_string());
        ui.end_row();
        ui.label("Executable arguments").on_hover_text(
            "Additional command line arguments. Change this value only if you want to run \
             the solver in a different way; CalculiX takes the job name, the analysis' name.",
        );
        ui.add(
            egui::TextEdit::singleline(&mut self.arguments)
                .hint_text(self.draft.name.as_str())
                .desired_width(f32::INFINITY),
        );
        ui.end_row();
        let work_dir = solver.work_dir();
        ui.label("Work directory").on_hover_text("Work directory.");
        ui.add(egui::Label::new(work_dir.display().to_string()).truncate())
            .on_hover_text(work_dir.display().to_string());
        ui.end_row();
        ui.label("Threads")
            .on_hover_text("Number of threads CalculiX uses (OMP_NUM_THREADS).");
        ui.label(solver.threads.max(1).to_string());
        ui.end_row();
    }
}
