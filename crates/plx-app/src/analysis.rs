//! Running the analysis: writes the input file, starts CalculiX and shows PrePoMax's monitor
//! window with the solver output, progress and the button to open the results.

use std::path::PathBuf;
use std::time::Instant;

use plx_job::{Job, JobStatus};
use plx_model::EquationSolver;

use crate::model::Model;
use crate::settings::Solver;

/// Lines of solver output kept for the monitor window.
const MAX_OUTPUT_LINES: usize = 5000;

pub struct Analysis {
    /// Name of the analysis that ran; follows a rename, while the files keep the job name.
    name: String,
    job: Job,
    output: Vec<String>,
    /// Text typed into the monitor's filter field; only output lines containing it are shown.
    filter: String,
    started: Instant,
    finished: Option<f32>,
    status: JobStatus,
    pub monitor: bool,
    /// The results of this run were bound to the project when they were opened.
    pub bound: bool,
    /// The keywords of the input file, see [`plx_io::inp::keywords::input_keywords`].
    keywords: Vec<String>,
}

pub enum MonitorEvent {
    None,
    /// Open the results of the run.
    OpenResults,
}

/// Copies the results file of a submodel's global model next to the input file `<name>.inp`
/// in `dir`, as `<name>-global.frd`, and lets `fe` refer to the copy: CalculiX reads it from
/// the directory of the input file and cannot take paths with spaces. Does nothing unless a
/// submodel boundary condition needs the file.
pub fn stage_global_results(
    fe: &mut plx_model::FeModel,
    dir: &std::path::Path,
    name: &str,
) -> Result<(), String> {
    if !fe.uses_global_results() {
        return Ok(());
    }
    // Without a file the input file reports the missing global results.
    let Some(source) = fe
        .properties
        .submodel_input()
        .map(std::path::Path::to_path_buf)
    else {
        return Ok(());
    };
    let target = dir.join(format!("{name}-global.frd"));
    let same = |a: &std::path::Path, b: &std::path::Path| {
        a.canonicalize()
            .ok()
            .is_some_and(|a| b.canonicalize().ok() == Some(a))
    };
    if same(&source, &dir.join(format!("{name}.frd"))) {
        return Err(format!(
            "The global results {} are this analysis's own results file, which the run \
             overwrites. Copy the file elsewhere and pick the copy in Model > Model Properties.",
            source.display()
        ));
    }
    if !same(&source, &target) {
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::copy(&source, &target))
            .map_err(|e| format!("Global results {} not copied: {e}", source.display()))?;
    }
    fe.properties.global_results = Some(target);
    Ok(())
}

impl Analysis {
    /// Writes `<analysis name>.inp` into the work directory and starts CalculiX on it with the
    /// analysis' arguments; steps left at the default solver use `default_solver`, see
    /// [`Solver::default_solver`]. With `check_model` CalculiX only reads and checks the
    /// model (PrePoMax's "Check Model").
    pub fn start(
        solver: &Solver,
        model: &Model,
        default_solver: EquationSolver,
        check_model: bool,
    ) -> Result<Self, String> {
        if model.fe.steps.is_empty() && !check_model {
            return Err(
                "Analysis not started: the model has no step (Model > Create Step …).".into(),
            );
        }
        if !check_model && !model.fe.steps.iter().any(|s| s.active) {
            return Err("Analysis not started: all steps are deactivated.".into());
        }
        let heading = format!("PrePoLix: {}", model.file_name());
        let mut fe = model.fe.clone();
        fe.resolve_default_solver(default_solver);
        let work_dir = solver.work_dir();
        let analysis = &model.fe.analysis;
        let name = analysis.name.as_str();
        stage_global_results(&mut fe, &work_dir, name)?;
        let write = if check_model {
            plx_io::inp::write_check_inp
        } else {
            plx_io::inp::write_inp
        };
        let input = write(&model.mesh, &fe, &heading)
            .map_err(|e| format!("Input file not written: {e}"))?;
        // The model check leaves out the steps; their keywords are still tested.
        let keywords = if check_model {
            let full = plx_io::inp::write_inp(&model.mesh, &fe, &heading).unwrap_or_default();
            plx_io::inp::keywords::input_keywords(&full)
        } else {
            plx_io::inp::keywords::input_keywords(&input)
        };
        let copied = copy_result_files(&fe, &work_dir);
        let job_solver = solver.job_solver();
        let hint = "Check the executable under Tools > Settings > CalculiX.";
        if let Some(message) = crate::settings::missing_executable(&job_solver.executable) {
            return Err(format!("CalculiX not started: {message}. {hint}"));
        }
        let arguments = analysis.command_arguments();
        let job = Job::start_with_arguments(&job_solver, &work_dir, name, &input, &arguments)
            .map_err(|e| {
                format!(
                    "CalculiX ({}) not started: {e}. {hint}",
                    job_solver.executable.display()
                )
            })?;
        let mut output = vec![format!(
            "{} {} in {}",
            name,
            if check_model {
                "model check started"
            } else {
                "started"
            },
            work_dir.display()
        )];
        output.push(format!(
            "Command: {} {}",
            job_solver.executable.display(),
            arguments.join(" ")
        ));
        output.extend(copied);
        Ok(Self {
            name: name.to_owned(),
            output,
            filter: String::new(),
            job,
            started: Instant::now(),
            finished: None,
            status: JobStatus::Running,
            monitor: true,
            bound: false,
            keywords,
        })
    }

    /// Name of the analysis that ran.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The analysis that ran was renamed; its files keep their names.
    pub fn rename(&mut self, name: String) {
        self.name = name;
    }

    /// Adds a line of prepolix's own to the monitor output.
    pub fn note(&mut self, line: String) {
        self.output.push(line);
    }

    /// The keywords of the input file CalculiX runs on.
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    /// The solver output collected so far.
    pub fn output(&self) -> &[String] {
        &self.output
    }

    pub fn is_running(&self) -> bool {
        self.finished.is_none()
    }

    /// Collects new output; returns the final status once, when the job has just ended.
    pub fn poll(&mut self) -> Option<JobStatus> {
        self.output.extend(self.job.new_output());
        if self.output.len() > MAX_OUTPUT_LINES {
            self.output.drain(..self.output.len() - MAX_OUTPUT_LINES);
        }
        if self.finished.is_some() {
            return None;
        }
        let status = self.job.poll();
        if status == JobStatus::Running {
            return None;
        }
        self.output.extend(self.job.new_output());
        self.finished = Some(self.started.elapsed().as_secs_f32());
        self.status = status;
        Some(status)
    }

    pub fn status(&self) -> JobStatus {
        self.status
    }

    pub fn kill(&mut self) {
        self.job.kill();
        self.poll();
    }

    pub fn status_text(status: JobStatus) -> &'static str {
        match status {
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::FailedWithResults => "finished with errors, results available",
            JobStatus::Failed => "failed",
            JobStatus::Killed => "killed",
        }
    }

    /// PrePoMax's monitor window.
    pub fn window(&mut self, ctx: &egui::Context) -> MonitorEvent {
        let mut event = MonitorEvent::None;
        let mut open = self.monitor;
        let status = self.job.poll();
        egui::Window::new(format!("Monitor: {}", self.name))
            .id(egui::Id::new("analysis monitor"))
            .open(&mut open)
            .collapsible(false)
            .default_size([620.0, 380.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if status == JobStatus::Running {
                        ui.spinner();
                    }
                    let seconds = self
                        .finished
                        .unwrap_or_else(|| self.started.elapsed().as_secs_f32());
                    ui.label(format!(
                        "Status: {} ({seconds:.1} s)",
                        Self::status_text(status)
                    ));
                    let progress = match self.job.progress() {
                        Some(p) => Some(format!(
                            "Step {}, increment {}, iterations {}, time {}",
                            p.step, p.increment, p.iterations, p.total_time
                        )),
                        // The .sta file only changes when an increment ends; until then the
                        // solver output tells where the run is.
                        None => live_progress(&self.output),
                    };
                    if let Some(text) = progress {
                        ui.separator();
                        ui.label(text);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("Show only lines containing ...")
                            .desired_width(240.0),
                    );
                    if ui
                        .add_enabled(!self.filter.is_empty(), egui::Button::new("Clear"))
                        .clicked()
                    {
                        self.filter.clear();
                    }
                });
                let height = ui.available_height() - 36.0;
                let area = egui::ScrollArea::both()
                    .max_height(height.max(80.0))
                    .auto_shrink([false, false])
                    .stick_to_bottom(true);
                if self.filter.is_empty() {
                    crate::virtual_table::lines(ui, area, &self.output);
                } else {
                    let needle = self.filter.to_lowercase();
                    let shown: Vec<&String> = self
                        .output
                        .iter()
                        .filter(|line| line.to_lowercase().contains(&needle))
                        .collect();
                    crate::virtual_table::lines(ui, area, &shown);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(status == JobStatus::Running, egui::Button::new("Kill"))
                        .clicked()
                    {
                        self.kill();
                    }
                    let results = self.job.results().filter(|_| status != JobStatus::Running);
                    if ui
                        .add_enabled(results.is_some(), egui::Button::new("Results"))
                        .clicked()
                    {
                        event = MonitorEvent::OpenResults;
                    }
                });
            });
        self.monitor = open;
        if status == JobStatus::Running {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        event
    }

    pub fn results(&self) -> Option<PathBuf> {
        self.job.results()
    }
}

/// Step, increment and iteration from the latest `STEP n`, `INCREMENT n` and `iteration n`
/// lines of CalculiX's output, for the time before the first `.sta` line exists.
fn live_progress(output: &[String]) -> Option<String> {
    let number = |line: &str, key: &str| -> Option<u32> {
        line.trim().strip_prefix(key)?.trim().parse().ok()
    };
    let (mut step, mut increment, mut iteration) = (None, None, None);
    for line in output.iter().rev() {
        if iteration.is_none() && increment.is_none() {
            iteration = number(line, "iteration");
        }
        if increment.is_none() {
            increment = number(line, "INCREMENT");
        }
        if step.is_none() {
            step = number(line, "STEP");
        }
        if step.is_some() && increment.is_some() {
            break;
        }
    }
    let (step, increment) = (step?, increment?);
    Some(match iteration {
        Some(iteration) => format!("Step {step}, increment {increment}, iterations {iteration}"),
        None => format!("Step {step}, increment {increment}"),
    })
}

/// Copies the result files the defined fields read next to the input file, where CalculiX
/// looks for them; a file already there is left alone. Returns a line per file for the
/// monitor output.
pub fn copy_result_files(fe: &plx_model::FeModel, dir: &std::path::Path) -> Vec<String> {
    let mut notes = Vec::new();
    let files = fe.result_files();
    if !files.is_empty()
        && let Err(error) = std::fs::create_dir_all(dir)
    {
        return vec![format!("{} not created: {error}", dir.display())];
    }
    for file in files {
        let Some(name) = file.file_name() else {
            continue;
        };
        let target = dir.join(name);
        if target == file {
            continue;
        }
        match std::fs::copy(file, &target) {
            Ok(_) => notes.push(format!("{} copied to {}", file.display(), dir.display())),
            Err(error) => notes.push(format!("{} not copied: {error}", file.display())),
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::live_progress;

    #[test]
    fn progress_follows_the_solver_output() {
        let lines = |text: &str| text.lines().map(String::from).collect::<Vec<_>>();
        assert_eq!(live_progress(&lines("started\nCalculiX")), None);
        assert_eq!(
            live_progress(&lines(" STEP 2\n\n INCREMENT 3\n\n ATTEMPT 1\n")),
            Some("Step 2, increment 3".into())
        );
        assert_eq!(
            live_progress(&lines(
                " STEP 1\n INCREMENT 4\n iteration 2\n x\n iteration 5\n"
            )),
            Some("Step 1, increment 4, iterations 5".into())
        );
        // An iteration of an earlier increment does not count.
        assert_eq!(
            live_progress(&lines(
                " STEP 1\n INCREMENT 1\n iteration 7\n INCREMENT 2\n"
            )),
            Some("Step 1, increment 2".into())
        );
    }
}
