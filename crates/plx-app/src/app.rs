use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use plx_model::UnitSystem;
use plx_render::StandardView;
use plx_results::hot_spot::{HotSpot, HotSpotPath};

use crate::analysis::{Analysis, MonitorEvent};
use crate::animation::{AnimationKind, ColorLimits, Playback};
use crate::animation_export::{self, Format};
use crate::contact_search::{ContactSearchDialog, SearchResult};
use crate::exploded::{ExplodedDialog, ExplodedResult};
use crate::features::{FeatureDialog, FeatureKind, FeatureResult};
use crate::field_output_dialog::{DialogAction, FieldOutputDialog};
use crate::history_output_dialog::HistoryOutputDialog;
use crate::history_table::HistoryTable;
use crate::hot_spot_dialog::HotSpotDialog;
use crate::icons::{self, Icon};
use crate::keywords::KeywordEditor;
use crate::material_library::{LibraryResult, MaterialLibraryEditor};
use crate::mesh_tools::{MeshTool, MeshToolDialog, MeshToolResult};
use crate::meshing::{
    MeshItemEditor, MeshItemResult, MeshSetupResult, MeshSetupWindow, MeshingJob,
};
use crate::model::{self, Highlight, LoadedModel, Model};
use crate::model_properties::{DialogResult, ModelPropertiesDialog, geometry_check};
use crate::numeric;
use crate::overlay::AnnotationCommand;
use crate::overlay::{Marker, Overlay};
use crate::path_plot::{self, PathPlot, PlotEvent};
use crate::properties;
use crate::query::{QueryAction, QueryClick, QueryWindow};
use crate::results::{Deformation, ResultsView, format_legend_value};
use crate::screenshot::{self, Screenshot};
use crate::section::{PlaneDefinition, SectionDialog, SectionResult, SectionView};
use crate::selection::Operation;
use crate::settings::{self, Settings, SettingsWindow, WindowResult};
use crate::setup::{Editor, EditorResult, NewItem};
use crate::sound::{self, ModeSound};
use crate::symbols;
use crate::transformation_dialog::{TransformationAction, TransformationDialog};
use crate::tree::{self, AnalysisAction, TreeItem, TreeResponse, TreeState, TreeView};
use crate::viewport::{Axis, BoxSelect, Click, ViewCommand, Viewport};
use plx_render::RenderMesh;

enum LoadEvent {
    Started(PathBuf),
    Finished(PathBuf, Result<Box<LoadedModel>, String>),
    /// CAD files added to the geometry of the open model.
    Added(Vec<PathBuf>, Result<Box<plx_mesher::CadAddition>, String>),
    /// Meshes of input files, each read from its file, to add as parts of the open model.
    MeshesRead(Vec<(PathBuf, Result<Box<plx_io::inp::InpImport>, String>)>),
}

struct Workbench {
    settings: Settings,
    /// Open settings window with its unsaved draft.
    settings_window: Option<SettingsWindow>,
    /// The check for a new release and its window.
    updater: crate::update::Updater,
    /// Campbell diagram window with its speed sweep; kept with its results while hidden.
    campbell: Option<crate::campbell::CampbellWindow>,
    /// Open dialog of the model space and unit system, for a new or the open model.
    model_dialog: Option<ModelPropertiesDialog>,
    /// The new model asked for the geometry import; the app opens the file dialog.
    import_requested: bool,
    viewport: Viewport,
    /// The FE model workspace: mesh and analysis set up from an input or project file.
    model: Option<Model>,
    /// The CAD geometry of the FE model as shown on the Geometry tab.
    geometry: Option<Model>,
    /// Open window of the default meshing parameters.
    mesh_setup: Option<MeshSetupWindow>,
    /// Open dialog of a mesh setup item.
    mesh_item_editor: Option<MeshItemEditor>,
    meshing: Option<MeshingJob>,
    /// CAD faces and edges Gmsh named when meshing failed, shown red on the geometry.
    mesh_failure: BTreeSet<plx_mesher::CadEntity>,
    /// The results workspace: every results file opened in this session, PrePoMax's results
    /// collection. One of them is shown on the Results tab.
    results: Vec<Model>,
    current_result: usize,
    /// Camera of the workspace not shown, so switching tabs keeps each view.
    parked_camera: Option<plx_render::Camera>,
    tree: TreeState,
    /// Item whose properties window is open.
    dialog: Option<TreeItem>,
    /// Name typed in the properties window of a part, with why it cannot be taken.
    part_name: (String, Option<String>),
    /// Element types of the part in the properties window, as the user picks them.
    part_types: Vec<String>,
    /// Part the open context menu of the 3D view belongs to.
    menu_part: Option<usize>,
    /// Which of the three trees is shown.
    tree_view: TreeView,
    output: Vec<String>,
    view_command: Option<ViewCommand>,
    /// The result selection or deformation changed; the scene must be rebuilt.
    results_changed: bool,
    /// Only the animation frame changed; frames already built are reused.
    frame_changed: bool,
    /// Scene of each animation frame shown so far, cleared when anything else changes.
    frame_cache: std::collections::HashMap<usize, Vec<RenderMesh>>,
    /// Open dialog creating or editing an item of the FE model.
    editor: Option<Editor>,
    /// Item asked to be deleted, waiting for the user's confirmation.
    /// Items to delete once the user agrees: several parts, or one item.
    confirm_delete: Option<(TreeView, Vec<TreeItem>)>,
    /// Open CalculiX keyword editor.
    keyword_editor: Option<KeywordEditor>,
    /// Open search for contact pairs.
    contact_search: Option<ContactSearchDialog>,
    /// Open mesh tool: transform parts, merge nodes or renumber.
    mesh_tool: Option<MeshToolDialog>,
    /// PrePoMax's Query tool, while its window is open.
    query: Option<QueryWindow>,
    /// Open material library editor.
    material_library: Option<MaterialLibraryEditor>,
    /// Open dialog creating or editing a field output derived from the shown results.
    field_output_dialog: Option<FieldOutputDialog>,
    /// Open dialog creating or editing a history output of the shown results.
    history_dialog: Option<HistoryOutputDialog>,
    /// Open table of a history output component.
    history_table: Option<HistoryTable>,
    /// Diagram of a table selection, PrePoMax's diagram view.
    history_plot: Option<crate::xy_plot::XyData>,
    /// The tree selection whose region is highlighted.
    /// `None` until it is computed, so a dialog's highlight is cleared once it closes.
    highlighted: Option<TreeSelection>,
    analysis: Option<Analysis>,
    /// Open dialog creating or editing an analysis.
    analysis_dialog: Option<crate::analysis_dialog::AnalysisDialog>,
    /// Results being bound to or restored from the project file of the model at this path.
    bound_work: Option<(PathBuf, std::sync::mpsc::Receiver<BoundWork>)>,
    /// What was defined on the results before the analysis ran again, for its new results
    /// at this path.
    carried_settings: Option<(PathBuf, plx_results::ResultsSettings)>,
    /// Problems CalculiX reported when the last analysis failed, shown in the tree.
    solver_findings: Vec<plx_model::Finding>,
    /// The findings of a tree item whose warning sign was clicked, explained in a window.
    findings_window: Option<Vec<plx_model::Finding>>,
    /// Results file the user asked to open; read by the app on a worker thread.
    open_results: Option<PathBuf>,
    screenshot: Screenshot,
    /// Animation export taking its frames from the 3D view.
    recording: Option<animation_export::Recording>,
    /// Exported animation being encoded, with its file name.
    animation_encoding: Option<(String, animation_export::Encoder)>,
    /// Open dialog of a hot spot definition of the current results.
    hot_spot_dialog: Option<HotSpotDialog>,
    /// Hot spot whose paths are shown, edited or selected, with the results file and its
    /// paths there.
    hot_spot_preview: Option<(HotSpot, usize, Vec<HotSpotPath>)>,
    /// The toe node whose path the hot spot dialog plots, as last drawn.
    hot_spot_plotted: Option<plx_mesh::NodeId>,
    /// Audio output of the sound window, opened when it first plays.
    audio: Option<sound::Player>,
    /// The section view, while it is on; it cuts whatever the 3D view shows.
    section: Option<SectionView>,
    section_dialog: Option<SectionDialog>,
    /// Open dialog of the transformations of the current results.
    transformation_dialog: Option<TransformationDialog>,
    /// The cut shown in the scene of the given version with the visible parts, and whether
    /// it shows the section faces alone, to rebuild it on changes.
    section_shown: Option<(u64, Vec<bool>, SectionView, bool)>,
    /// The boundary conditions and loads, with the shown parts and the exploded view, whose
    /// symbols are drawn.
    symbols_shown: Option<(Vec<symbols::Item>, Vec<bool>, u64)>,
    /// Open exploded view dialog with the model it edits.
    exploded_dialog: Option<(ExplodedDialog, ShownModel)>,
    /// The exploded view last applied, where the next one starts, as in PrePoMax.
    last_exploded: crate::exploded::Parameters,
    /// Open dialog of a feature or of results on one.
    feature_dialog: Option<FeatureDialog>,
    /// The Edit Annotation dialog of a path plot in the 3D view.
    plot_editor: Option<path_plot::EditDialog>,
    /// The Color Scale dialog of a results file, with the scale it last applied.
    color_scale_dialog: Option<(
        crate::color_scale_dialog::ColorScaleDialog,
        plx_model::ColorScale,
    )>,
    /// The Export VTU or Export VTK dialog of a results file.
    vtk_export_dialog: Option<crate::vtk_export_dialog::VtkExportDialog>,
    /// The shown values on the plane of the shown plane result, in model coordinates: where
    /// the plane cuts the parts as shown, and where it cuts the undeformed mesh.
    section_values: Option<plx_render::SectionValues>,
    plane_values: Option<plx_render::SectionValues>,
}

/// Which model the 3D view shows: the geometry, the FE model or one of the results.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShownModel(TreeView, usize);

pub struct PrepolixApp {
    workbench: Workbench,
    load_events: (Sender<LoadEvent>, Receiver<LoadEvent>),
    loading: Option<PathBuf>,
}

impl PrepolixApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let render_state = cc
            .wgpu_render_state
            .clone()
            .ok_or("PrePoLix requires the wgpu backend of eframe")?;
        crate::style::apply(&cc.egui_ctx);
        let adapter = render_state.adapter.get_info();
        let output = vec![format!(
            "PrePoLix {} started, graphics: {} ({:?})",
            env!("CARGO_PKG_VERSION"),
            adapter.name,
            adapter.backend
        )];
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, settings::STORAGE_KEY))
            .unwrap_or_default();
        settings.upgrade();
        crate::style::set_theme(&cc.egui_ctx, settings.graphics.theme);
        plx_mesher::set_library_path(settings.gmsh.library());
        let mut app = Self {
            workbench: Workbench {
                settings,
                settings_window: None,
                updater: Default::default(),
                model_dialog: None,
                import_requested: false,
                viewport: Viewport::new(render_state),
                model: None,
                geometry: None,
                mesh_setup: None,
                mesh_item_editor: None,
                meshing: None,
                mesh_failure: BTreeSet::new(),
                results: Vec::new(),
                current_result: 0,
                parked_camera: None,
                tree: TreeState::default(),
                dialog: None,
                part_name: Default::default(),
                part_types: Vec::new(),
                menu_part: None,
                tree_view: TreeView::FeModel,
                output,
                view_command: None,
                results_changed: false,
                frame_changed: false,
                frame_cache: Default::default(),
                editor: None,
                confirm_delete: None,
                keyword_editor: None,
                contact_search: None,
                mesh_tool: None,
                query: None,
                material_library: None,
                field_output_dialog: None,
                history_dialog: None,
                history_table: None,
                history_plot: None,
                highlighted: None,
                analysis: None,
                analysis_dialog: None,
                bound_work: None,
                carried_settings: None,
                campbell: None,
                solver_findings: Vec::new(),
                findings_window: None,
                open_results: None,
                screenshot: Screenshot::default(),
                recording: None,
                animation_encoding: None,
                hot_spot_dialog: None,
                hot_spot_preview: None,
                hot_spot_plotted: None,
                audio: None,
                section: None,
                section_dialog: None,
                transformation_dialog: None,
                feature_dialog: None,
                plot_editor: None,
                color_scale_dialog: None,
                vtk_export_dialog: None,
                section_values: None,
                plane_values: None,
                section_shown: None,
                symbols_shown: None,
                exploded_dialog: None,
                last_exploded: Default::default(),
            },
            load_events: channel(),
            loading: None,
        };
        if app.workbench.settings.updates.check_at_startup
            && app.workbench.updater.installation().checks_at_startup()
        {
            app.workbench.updater.check(&cc.egui_ctx, false);
        }
        // Every file on the command line is opened in turn, e.g. a model and its results.
        let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        // Without files on the command line, the project open at the last exit is reopened.
        if paths.is_empty() {
            paths.extend(cc.storage.and_then(stored_last_project));
        }
        if !paths.is_empty() {
            let (sender, ctx) = (app.load_events.0.clone(), cc.egui_ctx.clone());
            let units = app.workbench.import_units();
            std::thread::spawn(move || {
                // Several CAD files make one geometry, as with Geometry > Import.
                if paths.len() > 1 && paths.iter().all(|p| plx_mesher::is_cad_file(p)) {
                    return load_geometry_in_background(paths, units, sender, ctx);
                }
                for path in paths {
                    load_in_background(path, units, sender.clone(), ctx.clone());
                }
            });
        }
        Ok(app)
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let sender = self.load_events.0.clone();
        let ctx = ctx.clone();
        let units = self.workbench.import_units();
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Open Model")
                .add_filter(
                    "Project, CalculiX model, results or geometry",
                    &[
                        "plx", "PLX", "inp", "INP", "frd", "FRD", "step", "STEP", "stp", "STP",
                        "iges", "IGES", "igs", "IGS", "brep", "BREP",
                    ],
                )
                .add_filter("PrePoLix project (*.plx)", &["plx", "PLX"])
                .add_filter("Input file (*.inp)", &["inp", "INP"])
                .add_filter("Results file (*.frd)", &["frd", "FRD"])
                .add_filter(GEOMETRY_FILTER.0, GEOMETRY_FILTER.1)
                .pick_file();
            if let Some(path) = picked {
                load_in_background(path, units, sender, ctx);
            }
        });
    }

    /// Geometry > Import. Without a model, PrePoMax first asks for the new model's
    /// properties, so that a 2D geometry lands in a 2D model.
    fn import(&mut self, ctx: &egui::Context) {
        if self.workbench.model.is_none() {
            self.workbench.new_model(true);
        } else {
            self.import_dialog(ctx);
        }
    }

    /// PrePoMax's Geometry > Import: one or several STEP, IGES or BREP files, read into
    /// one geometry, or input files (.inp) whose mesh becomes mesh parts, such as a mesh
    /// written by "Export Deformed Mesh". A model that already has geometry or a mesh keeps
    /// it; the new parts are added.
    fn import_dialog(&mut self, ctx: &egui::Context) {
        if self.loading.is_some() {
            return;
        }
        let sender = self.load_events.0.clone();
        let ctx = ctx.clone();
        let units = self.workbench.import_units();
        // What the new parts are added to: the geometry, if any, and the names of the
        // mesh parts they must not take.
        let base = (self.workbench.model.as_ref())
            .filter(|m| m.geometry.is_some() || m.mesh.element_count() > 0)
            .map(|m| {
                let taken: Vec<String> = m.parts.iter().map(|p| p.name.clone()).collect();
                (m.geometry.clone(), taken, m.fe.properties.units)
            });
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Import Geometry")
                .add_filter(IMPORT_FILTER.0, IMPORT_FILTER.1)
                .add_filter(GEOMETRY_FILTER.0, GEOMETRY_FILTER.1)
                .add_filter("Input file (*.inp)", &["inp", "INP"])
                .pick_files();
            let Some(paths) = picked.filter(|p| !p.is_empty()) else {
                return;
            };
            let (meshes, paths): (Vec<PathBuf>, Vec<PathBuf>) =
                paths.into_iter().partition(|p| !plx_mesher::is_cad_file(p));
            if paths.is_empty() {
                read_meshes_in_background(meshes, sender, ctx);
                return;
            }
            match base {
                Some((geometry, taken, units)) => {
                    let _ = sender.send(LoadEvent::Started(paths[0].clone()));
                    ctx.request_repaint();
                    let result =
                        plx_mesher::add_cad_files(geometry.as_ref(), &paths, units, &taken)
                            .map(Box::new)
                            .map_err(|e| e.to_string());
                    let _ = sender.send(LoadEvent::Added(paths, result));
                    ctx.request_repaint();
                }
                None => load_geometry_in_background(paths, units, sender.clone(), ctx.clone()),
            }
            if !meshes.is_empty() {
                read_meshes_in_background(meshes, sender, ctx);
            }
        });
    }

    fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.loading.is_none() {
            let (sender, ctx) = (self.load_events.0.clone(), ctx.clone());
            let units = self.workbench.import_units();
            std::thread::spawn(move || load_in_background(path, units, sender, ctx));
        }
    }

    fn open_geometry(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        if self.loading.is_none() {
            let (sender, ctx) = (self.load_events.0.clone(), ctx.clone());
            let units = self.workbench.import_units();
            std::thread::spawn(move || load_geometry_in_background(paths, units, sender, ctx));
        }
    }

    fn handle_load_events(&mut self) {
        while let Ok(event) = self.load_events.1.try_recv() {
            match event {
                LoadEvent::Started(path) => self.loading = Some(path),
                LoadEvent::Finished(path, result) => {
                    self.loading = None;
                    self.workbench.model_loaded(path, result);
                }
                LoadEvent::Added(paths, result) => {
                    self.loading = None;
                    self.workbench.geometry_added(&paths, result);
                }
                LoadEvent::MeshesRead(files) => {
                    self.loading = None;
                    self.workbench.meshes_added(files);
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                let new = egui::Button::new("New …").shortcut_text("Ctrl+N");
                if ui.add(new).clicked() {
                    self.workbench.new_model(false);
                }
                let open = egui::Button::new("Open …").shortcut_text("Ctrl+O");
                if ui.add_enabled(self.loading.is_none(), open).clicked() {
                    self.open_dialog(ui.ctx());
                }
                let setup = self.workbench.setup_model().is_some();
                let save = egui::Button::new("Save").shortcut_text("Ctrl+S");
                if ui.add_enabled(setup, save).clicked() {
                    self.workbench.save_project(false);
                }
                let save_as = egui::Button::new("Save As …").shortcut_text("Ctrl+Shift+S");
                if ui.add_enabled(setup, save_as).clicked() {
                    self.workbench.save_project(true);
                }
                let bound =
                    (self.workbench.setup_model()).is_some_and(|m| m.bound_results.is_some());
                let without = egui::Button::new("Save Without Results …");
                if ui
                    .add_enabled(bound, without)
                    .on_hover_text(
                        "Saves a copy with only the mesh and the model definition, without \
                         the result bound to the analysis. The open project keeps its result.",
                    )
                    .on_disabled_hover_text("No result is bound to the analysis.")
                    .clicked()
                {
                    self.workbench.save_without_results();
                }
                ui.separator();
                let export = egui::Button::new("Export CalculiX Input File …");
                if ui.add_enabled(setup, export).clicked() {
                    self.workbench.export_inp();
                }
                let deformed = self.workbench.shown_results_view().is_some();
                let export = egui::Button::new("Export Deformed Mesh (.inp) …");
                let tip = "Writes the mesh of the current results, deformed by the current \
                           deformation scale factor, as an input file with only the mesh.";
                if ui
                    .add_enabled(deformed, export)
                    .on_hover_text(tip)
                    .clicked()
                {
                    self.workbench.export_deformed_mesh();
                }
                let tip = "Writes the current results for ParaView and other VTK programs, \
                           one increment or a time series of several.";
                for (label, format) in [
                    ("Export VTU …", plx_io::vtk::VtkFormat::Xml),
                    ("Export VTK …", plx_io::vtk::VtkFormat::Legacy),
                ] {
                    if ui
                        .add_enabled(deformed, egui::Button::new(label))
                        .on_hover_text(tip)
                        .on_disabled_hover_text("No results are open.")
                        .clicked()
                    {
                        self.workbench.open_vtk_export(format);
                    }
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| self.workbench.edit_menu(ui));
            ui.menu_button("View", |ui| {
                if ui.button("Zoom to Fit").clicked() {
                    self.workbench.view_command = Some(ViewCommand::Fit);
                }
                for (view, label) in STANDARD_VIEWS {
                    if ui.button(label).clicked() {
                        self.workbench.view_command = Some(ViewCommand::View(view));
                    }
                }
                if ui.button("Vertical View").clicked() {
                    self.workbench.view_command = Some(ViewCommand::Vertical);
                }
                ui.menu_button("View Normal to Axis", |ui| {
                    for axis in Axis::ALL {
                        if ui.button(axis.label()).clicked() {
                            self.workbench.view_command = Some(ViewCommand::AxisView(axis));
                        }
                    }
                });
                ui.menu_button("Isometric, Axis Up", |ui| {
                    for axis in Axis::ALL {
                        if ui.button(axis.label()).clicked() {
                            self.workbench.view_command = Some(ViewCommand::IsometricAxis(axis));
                        }
                    }
                });
                ui.separator();
                ui.checkbox(
                    &mut self.workbench.viewport.options.mesh_edges,
                    "Mesh Edges",
                );
                ui.checkbox(
                    &mut self.workbench.settings.graphics.shell_thickness,
                    "Shell Thickness",
                )
                .on_hover_text(settings::SHELL_THICKNESS_TIP);
                ui.separator();
                let has_model = self.workbench.shown().is_some();
                if ui
                    .add_enabled(has_model, egui::Button::new("Section View …"))
                    .clicked()
                {
                    self.workbench.open_section_dialog();
                }
                let active = self.workbench.section.is_some();
                if ui
                    .add_enabled(active, egui::Button::new("Section View Off"))
                    .clicked()
                {
                    self.workbench.section = None;
                    self.workbench.section_dialog = None;
                }
                ui.separator();
                if ui
                    .add_enabled(has_model, egui::Button::new("Exploded View …"))
                    .clicked()
                {
                    self.workbench.open_exploded_dialog();
                }
                let exploded = self.workbench.exploded_applied();
                if ui
                    .add_enabled(exploded, egui::Button::new("Exploded View Off"))
                    .clicked()
                {
                    self.workbench.toggle_exploded();
                }
            });
            ui.menu_button("Geometry", |ui| {
                let import = egui::Button::new("Import …");
                if ui.add_enabled(self.loading.is_none(), import).clicked() {
                    self.import(ui.ctx());
                }
            });
            ui.menu_button("Mesh", |ui| self.workbench.mesh_menu(ui));
            ui.menu_button("Model", |ui| self.workbench.model_menu(ui));
            ui.menu_button("Interaction", |ui| self.workbench.interaction_menu(ui));
            ui.menu_button("Analysis", |ui| self.workbench.analysis_menu(ui));
            ui.menu_button("Results", |ui| self.workbench.results_menu(ui));
            ui.menu_button("Tools", |ui| {
                if ui.button("Query …").clicked() {
                    self.workbench.open_query();
                }
                let annotated = self.workbench.has_annotations();
                if ui
                    .add_enabled(annotated, egui::Button::new("Remove Annotations"))
                    .clicked()
                {
                    self.workbench.annotations(AnnotationCommand::DeleteAll);
                }
                ui.separator();
                if ui.button("Settings …").clicked() {
                    self.workbench.settings_window =
                        Some(SettingsWindow::new(&self.workbench.settings));
                }
            });
            ui.menu_button("Help", |ui| {
                let tip = "Opens the user guide in the web browser";
                if ui.button("User Guide").on_hover_text(tip).clicked() {
                    let line = crate::help::open_user_guide();
                    self.workbench.output.push(line);
                }
                let updater = &mut self.workbench.updater;
                if ui
                    .add_enabled(!updater.busy(), egui::Button::new("Check for Updates …"))
                    .on_hover_text("Asks GitHub whether a newer version of PrePoLix is available")
                    .clicked()
                {
                    updater.check(ui.ctx(), true);
                }
            });
            // Version number, greyed out at the far right of the menu bar, with the symbol of
            // a newer release left of it.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                        .color(ui.visuals().weak_text_color()),
                );
                self.workbench.updater.indicator(ui);
            });
        });
    }

    /// PrePoMax's main tool bar: file commands, then views and display options.
    fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            if icons::button(ui, Icon::New, "New (Ctrl+N)", true, false).clicked() {
                self.workbench.new_model(false);
            }
            let can_open = self.loading.is_none();
            if icons::button(ui, Icon::Open, "Open (Ctrl+O)", can_open, false).clicked() {
                self.open_dialog(ui.ctx());
            }
            let import = "Import Geometry (STEP, IGES, BREP) or mesh (INP)";
            if icons::button(ui, Icon::Import, import, can_open, false).clicked() {
                self.import(ui.ctx());
            }
            let can_save = self.workbench.setup_model().is_some();
            if icons::button(ui, Icon::Save, "Save (Ctrl+S)", can_save, false).clicked() {
                self.workbench.save_project(false);
            }
            ui.separator();
            if icons::button(ui, Icon::Fit, "Zoom to Fit", true, false).clicked() {
                self.workbench.view_command = Some(ViewCommand::Fit);
            }
            for (view, label) in STANDARD_VIEWS {
                if icons::button(ui, Icon::View(view), label, true, false).clicked() {
                    self.workbench.view_command = Some(ViewCommand::View(view));
                }
            }
            if icons::button(ui, Icon::Vertical, "Vertical View", true, false).clicked() {
                self.workbench.view_command = Some(ViewCommand::Vertical);
            }
            let camera = icons::button(ui, Icon::Screenshot, "Screenshot", true, false);
            egui::Popup::menu(&camera).show(|ui| {
                if ui.button("Copy to Clipboard").clicked() {
                    self.workbench
                        .screenshot
                        .request(screenshot::Target::Clipboard);
                }
                if ui.button("Save As …").clicked() {
                    self.workbench.screenshot.request(screenshot::Target::File);
                }
            });
            ui.separator();
            let options = &mut self.workbench.viewport.options;
            let mesh = options.mesh_edges;
            if icons::button(ui, Icon::FeatureEdges, "Edges Only", true, !mesh).clicked() {
                options.mesh_edges = false;
            }
            if icons::button(ui, Icon::MeshEdges, "Mesh Edges", true, mesh).clicked() {
                options.mesh_edges = true;
            }
            ui.separator();
            let shown = self.workbench.shown().is_some();
            let sectioned =
                self.workbench.section.is_some() || self.workbench.section_dialog.is_some();
            if icons::button(ui, Icon::SectionView, "Section View", shown, sectioned).clicked() {
                self.workbench.open_section_dialog();
            }
            // PrePoMax: a left click turns the exploded view on and off, a right click opens
            // its dialog.
            let exploded =
                self.workbench.exploded_applied() || self.workbench.exploded_dialog.is_some();
            let tooltip = "Exploded View on/off (right click: settings)";
            let button = icons::button(ui, Icon::ExplodedView, tooltip, shown, exploded);
            if button.clicked() {
                self.workbench.toggle_exploded();
            }
            if button.secondary_clicked() {
                self.workbench.open_exploded_dialog();
            }
            ui.separator();
            // PrePoMax: Query, then Remove annotations.
            let querying = self.workbench.query.is_some();
            if icons::button(ui, Icon::Query, "Query", shown, querying).clicked() {
                if querying {
                    self.workbench.close_query();
                } else {
                    self.workbench.open_query();
                }
            }
            let annotated = self.workbench.has_annotations();
            let tooltip = "Remove Annotations";
            if icons::button(ui, Icon::RemoveAnnotations, tooltip, annotated, false).clicked() {
                self.workbench.annotations(AnnotationCommand::DeleteAll);
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // PrePoMax shows the unit system of the model at the right.
            if let Some(model) = &self.workbench.model {
                let properties = &model.fe.properties;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!(
                        "Unit system: {}   Model space: {}",
                        properties.units.label(),
                        properties.space.label()
                    ));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        self.status_text(ui);
                    });
                });
            } else {
                self.status_text(ui);
            }
        });
    }

    fn status_text(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| match (&self.loading, self.workbench.shown()) {
            (Some(path), _) => {
                ui.spinner();
                ui.label(format!("Loading {} …", path.display()));
            }
            (None, _) if self.workbench.recording.is_some() => {
                if let Some(recording) = &self.workbench.recording {
                    let (done, all) = recording.progress();
                    ui.spinner();
                    ui.label(format!(
                        "Recording {}: frame {} of {all} (Esc cancels)",
                        recording.file,
                        (done + 1).min(all)
                    ));
                }
            }
            (None, _) if self.workbench.animation_encoding.is_some() => {
                if let Some((file, _)) = &self.workbench.animation_encoding {
                    ui.spinner();
                    ui.label(format!("Writing {file} …"));
                }
            }
            (None, _) if self.workbench.meshing.is_some() => {
                ui.spinner();
                ui.label("Meshing …");
            }
            (None, Some(model)) if model.is_geometry() => {
                ui.label(format!(
                    "{}: {} parts",
                    model.file_name(),
                    model.parts.len()
                ));
            }
            (None, Some(model)) => {
                ui.label(format!(
                    "{}: {} nodes, {} elements, {} parts",
                    model.file_name(),
                    model.mesh.node_count(),
                    model.mesh.element_count(),
                    model.parts.len()
                ));
            }
            (None, None) => {
                ui.weak(if self.workbench.tree_view == TreeView::Results {
                    "No results loaded"
                } else {
                    "No model loaded"
                });
            }
        });
    }
}

/// Moves the parts of a model between two scales of an exploded view: through the levels of a
/// sequential disassembly one after the other, else in one animation.
fn animate_explosion(
    model: &mut Model,
    parameters: &crate::exploded::Parameters,
    from: f64,
    to: f64,
) {
    let sequential =
        parameters.method == crate::exploded::Method::Disassembly && parameters.sequential;
    let stops = if sequential {
        model.explosion_layout(parameters).sequence(from, to)
    } else {
        vec![to]
    };
    let duration = if stops.len() > 1 {
        crate::exploded::STEP_TIME
    } else {
        crate::exploded::ANIMATION_TIME
    };
    let targets = stops
        .into_iter()
        .map(|scale| {
            let offsets = model
                .explosion_layout(parameters)
                .offsets(scale, parameters.sequential);
            (offsets, duration)
        })
        .collect();
    model.explosion.show_sequence(targets, true);
}

/// Shows the exploded view being edited in a dialog.
fn preview_explosion(model: &mut Model, dialog: &mut ExplodedDialog, animate: bool) {
    let parameters = dialog.draft.clone();
    dialog.step_count = model.explosion_layout(&parameters).step_count();
    let offsets = model.explosion_offsets(&parameters);
    model.explosion.show(offsets, animate);
}

/// Loads a file on this thread; CAD geometry in the length unit of `units`.
fn load_in_background(
    path: PathBuf,
    units: UnitSystem,
    sender: Sender<LoadEvent>,
    ctx: egui::Context,
) {
    let _ = sender.send(LoadEvent::Started(path.clone()));
    ctx.request_repaint();
    let result = model::load(&path, units).map(Box::new);
    let _ = sender.send(LoadEvent::Finished(path, result));
    ctx.request_repaint();
}

/// Imports several CAD files into one geometry on the calling thread, reporting like
/// [`load_in_background`] under the first file's path.
fn load_geometry_in_background(
    paths: Vec<PathBuf>,
    units: UnitSystem,
    sender: Sender<LoadEvent>,
    ctx: egui::Context,
) {
    let path = paths[0].clone();
    let _ = sender.send(LoadEvent::Started(path.clone()));
    ctx.request_repaint();
    let result = model::load_geometry(&paths, units).map(Box::new);
    let _ = sender.send(LoadEvent::Finished(path, result));
    ctx.request_repaint();
}

/// Reads the meshes of input files on the calling thread, to be added to the open model.
fn read_meshes_in_background(paths: Vec<PathBuf>, sender: Sender<LoadEvent>, ctx: egui::Context) {
    let _ = sender.send(LoadEvent::Started(paths[0].clone()));
    ctx.request_repaint();
    let files = (paths.into_iter())
        .map(|path| {
            let result = plx_io::inp::read_inp(&path)
                .map(Box::new)
                .map_err(|e| e.to_string());
            (path, result)
        })
        .collect();
    let _ = sender.send(LoadEvent::MeshesRead(files));
    ctx.request_repaint();
}

/// The selected tree item and the parts selected along with it.
type TreeSelection = (Option<(TreeView, TreeItem)>, BTreeSet<usize>);

/// The output line after deleting parts.
fn deleted_message(names: &[String]) -> String {
    match names {
        [name] => format!("Part {name} deleted"),
        names => format!("Parts {} deleted", names.join(", ")),
    }
}

/// How many regions of the FE model consist of node or element numbers that a new mesh no
/// longer has. Regions picked on the geometry are found on the new mesh again.
fn lost_selections(fe: &plx_model::FeModel, mesh: &plx_mesh::FeMesh) -> usize {
    fe.regions()
        .filter(|r| r.by_mesh_ids() && r.missing_reference(mesh).is_some())
        .count()
}

/// File dialog filter of everything Import Geometry takes.
const IMPORT_FILTER: (&str, &[&str]) = (
    "Geometry or mesh (*.step, *.stp, *.iges, *.igs, *.brep, *.inp)",
    &[
        "step", "STEP", "stp", "STP", "iges", "IGES", "igs", "IGS", "brep", "BREP", "inp", "INP",
    ],
);

/// File dialog filter of the CAD formats Gmsh imports.
const GEOMETRY_FILTER: (&str, &[&str]) = (
    "Geometry (*.step, *.stp, *.iges, *.igs, *.brep)",
    &[
        "step", "STEP", "stp", "STP", "iges", "IGES", "igs", "IGS", "brep", "BREP",
    ],
);

const STANDARD_VIEWS: [(StandardView, &str); 7] = [
    (StandardView::Front, "Front View"),
    (StandardView::Back, "Back View"),
    (StandardView::Top, "Top View"),
    (StandardView::Bottom, "Bottom View"),
    (StandardView::Left, "Left View"),
    (StandardView::Right, "Right View"),
    (StandardView::Isometric, "Isometric View"),
];

/// Storage key of the project file open at exit, reopened at the next start.
const LAST_PROJECT_KEY: &str = "last_project";

/// The project file stored at the last exit, if it still exists. It is stored as an
/// `Option`, so it has to be read back as one.
fn stored_last_project(storage: &dyn eframe::Storage) -> Option<PathBuf> {
    eframe::get_value::<Option<PathBuf>>(storage, LAST_PROJECT_KEY)
        .flatten()
        .filter(|path| path.is_file())
}

impl eframe::App for PrepolixApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, settings::STORAGE_KEY, &self.workbench.settings);
        eframe::set_value(storage, LAST_PROJECT_KEY, &self.workbench.last_project());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_load_events();
        let ctx = ui.ctx().clone();
        // Under the System theme, the operating system may switch between light and dark.
        crate::style::sync(&ctx);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.open_dialog(&ctx);
        }
        let shift_command = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        if ctx.input_mut(|i| i.consume_key(shift_command, egui::Key::S)) {
            self.workbench.save_project(true);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            self.workbench.save_project(false);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.workbench.new_model(false);
        }
        // A focused text field undoes its own typing.
        if !ctx.egui_wants_keyboard_input() {
            // Ctrl+Z matches with Shift held too, so Redo is checked first.
            if ctx.input_mut(|i| i.consume_key(shift_command, egui::Key::Z)) {
                self.workbench.redo();
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.workbench.undo();
            }
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            (i.raw.dropped_files.iter())
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        // Several CAD files dropped together are imported into one geometry, as with
        // Geometry > Import; otherwise the first file opens.
        if dropped.len() > 1 && dropped.iter().all(|p| plx_mesher::is_cad_file(p)) {
            self.open_geometry(dropped, &ctx);
        } else if let Some(path) = dropped.into_iter().next() {
            self.open_path(path, &ctx);
        }

        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("tools").show(ui, |ui| {
            self.tool_bar(ui);
            // Like PrePoMax, the results row stays in place and is greyed out outside the
            // Results tab, so the 3D view does not jump when switching tabs.
            ui.separator();
            self.workbench.results_tool_bar(ui);
        });
        self.workbench.animate(&ctx);
        self.workbench.play_sound(&ctx);
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        // PrePoMax's fixed layout: tree on the left over the full height, 3D view with the
        // output below it; only the separators move.
        let pane = egui::Frame::new()
            .fill(crate::style::window())
            .stroke(egui::Stroke::new(1.0, crate::style::border()));
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(280.0)
            .size_range(160.0..=700.0)
            .frame(egui::Frame::new().fill(crate::style::control()))
            .show(ui, |ui| {
                let selected_tab = self.workbench.tree_tabs(ui);
                // Tabs sit flush on the pane: their bottom border is the pane's top border.
                ui.add_space(-ui.spacing().item_spacing.y - 1.0);
                pane.inner_margin(4).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let view = self.workbench.tree_view;
                    self.workbench.model_tree(ui, view);
                });
                // The selected tab opens into the pane like a Windows tab control.
                if let Some(tab) = selected_tab {
                    ui.painter().hline(
                        tab.x_range().shrink(1.0),
                        tab.bottom() - 0.5,
                        egui::Stroke::new(1.0, crate::style::window()),
                    );
                }
            });
        egui::Panel::bottom("output")
            .resizable(true)
            .default_size(140.0)
            .size_range(40.0..=600.0)
            .frame(pane.inner_margin(4))
            .show(ui, |ui| self.workbench.output(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            self.workbench.viewport.selecting = self.workbench.picking()
                || self.workbench.section_picks()
                || self.workbench.transformation_picks()
                || self.workbench.feature_picks();
            self.workbench.viewport.gizmo =
                match (&self.workbench.section_dialog, self.workbench.shown()) {
                    (Some(dialog), Some(model)) => Some(dialog.gizmo(model)),
                    _ => None,
                };
            let response = self.workbench.viewport.ui(ui);
            if let (Some(drag), Some(dialog)) = (response.gizmo, &mut self.workbench.section_dialog)
            {
                dialog.drag(drag);
            }
            self.workbench.path_plot_events(response.path_plots);
            if response.edit_legend {
                self.workbench.open_color_scale();
            }
            if let Some(command) = response.command {
                self.workbench.view_command = Some(command);
            }
            if let Some(click) = response.pivot {
                // Rotate about the model point under the cursor, or about the view centre
                // when the cursor points into empty space.
                let pivot = (self.workbench.shown()).and_then(|model| model.pick_click(&click));
                self.workbench
                    .viewport
                    .set_pivot(pivot.map(|hit| hit.point));
            }
            if let Some(click) = response.click {
                self.workbench.click(click);
            }
            if let Some(view) = &response.response {
                self.workbench.viewport_menu(view, response.secondary_click);
            }
            if let Some(command) = response.annotation {
                self.workbench.annotations(command);
            }
            if let Some(area) = response.box_select {
                self.workbench.box_select(&area);
            }
            if let Some(hover) = response.hover {
                self.workbench.hover(hover);
                ui.ctx().request_repaint();
            }
        });
        let view = self.workbench.viewport.rect;
        let workbench = &mut self.workbench;
        workbench
            .screenshot
            .update(&ctx, view, &mut workbench.output);
        self.workbench.properties_window(&ctx);
        self.workbench.editor_window(&ctx);
        self.workbench.update_motion_preview(&ctx);
        self.workbench.confirm_delete_window(&ctx);
        self.workbench.contact_search_window(&ctx);
        self.workbench.mesh_tool_window(&ctx);
        self.workbench.query_window(&ctx);
        self.workbench.section_window(&ctx);
        self.workbench.exploded_window(&ctx);
        self.workbench.keyword_editor_window(&ctx);
        self.workbench.analysis_dialog_window(&ctx);
        self.workbench.material_library_window(&ctx);
        self.workbench.mesh_setup_window(&ctx);
        self.workbench.mesh_item_window(&ctx);
        self.workbench.poll_meshing();
        self.workbench.field_output_window(&ctx);
        self.workbench.history_output_window(&ctx);
        self.workbench.hot_spot_dialog_window(&ctx);
        self.workbench.history_table_window(&ctx);
        self.workbench.transformation_window(&ctx);
        self.workbench.feature_window(&ctx);
        self.workbench.plot_editor_window(&ctx);
        self.workbench.color_scale_window(&ctx);
        self.workbench.vtk_export_window(&ctx);
        self.workbench.sync_result_features();
        self.workbench.run_analysis(&ctx);
        self.workbench.poll_bound_results(&ctx);
        self.workbench.campbell_window(&ctx);
        if let Some(path) = self.workbench.open_results.take() {
            self.open_path(path, &ctx);
        }
        self.workbench.update_highlight();
        self.workbench.update_symbols(&ctx);
        self.workbench.update_hot_spot_preview();
        self.workbench.findings_window(&ctx);
        self.workbench.settings_window(&ctx);
        (self.workbench.updater).show(&ctx, &mut self.workbench.output);
        self.workbench.model_dialog_window(&ctx);
        if std::mem::take(&mut self.workbench.import_requested) {
            self.import_dialog(&ctx);
        }
        self.workbench.update_explosion(&ctx);
        self.workbench.update_thick_shells();
        self.workbench.rebuild_if_results_changed();
        self.workbench.update_section();
        self.workbench.update_feature_overlay();
        self.workbench.update_path_plots();

        if let Some(command) = self.workbench.view_command.take() {
            let bounds = self.workbench.shown().and_then(Model::visible_bounds);
            self.workbench.viewport.apply(command, bounds);
        }
        // Last, after everything the user changed in this frame.
        if let Some(model) = &mut self.workbench.model {
            crate::undo::record(model);
        }
    }
}

/// What a worker binding or restoring bound results sends back.
enum BoundWork {
    /// The results file and the files bound.
    Bound(PathBuf, Result<Box<plx_io::project::BoundResults>, String>),
    /// The analysis, the directory and the results file written there.
    Restored(String, PathBuf, Result<PathBuf, String>),
}

fn saved_camera(saved: plx_results::saved_view::SavedCamera) -> plx_render::Camera {
    plx_render::Camera {
        target: glam::Vec3::from_array(saved.target),
        distance: saved.distance,
        rotation: glam::Quat::from_array(saved.rotation).normalize(),
        scene_radius: saved.scene_radius,
    }
}

/// A file size in kB or MB.
fn file_size(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        format!("{kb:.0} kB")
    } else {
        format!("{:.1} MB", kb / 1024.0)
    }
}

/// Asks for a project file to save `model` to, proposing its name with `suffix`.
fn pick_project_file(model: &Model, title: &str, suffix: &str) -> Option<PathBuf> {
    let stem = model
        .path
        .file_stem()
        .map_or_else(|| "Project".into(), |s| s.to_string_lossy().into_owned());
    let mut dialog = rfd::FileDialog::new()
        .set_title(title)
        .add_filter("PrePoLix project (*.plx)", &[model::PROJECT_EXTENSION])
        .set_file_name(format!("{stem}{suffix}.{}", model::PROJECT_EXTENSION));
    if let Some(dir) = model.path.parent() {
        dialog = dialog.set_directory(dir);
    }
    let mut path = dialog.save_file()?;
    if path.extension().is_none() {
        path.set_extension(model::PROJECT_EXTENSION);
    }
    Some(path)
}

/// Geometry imported into a model that has nothing yet keeps its FE model and its file: a
/// project saved before the import is still saved to its own file, not to the CAD file.
fn take_over_empty_model(model: &mut Model, old: &Model) {
    model.fe = old.fe.clone();
    model.path = old.path.clone();
}

impl Workbench {
    fn model_loaded(&mut self, path: PathBuf, result: Result<Box<LoadedModel>, String>) {
        match result {
            Ok(loaded) => {
                let LoadedModel {
                    mut model,
                    render_meshes,
                    geometry_view,
                } = *loaded;
                // Geometry imported into a model that has nothing yet, such as one just
                // created with File > New, has to fit its model space.
                let into_empty = (self.model.as_ref()).filter(|old| {
                    plx_mesher::is_cad_file(&path)
                        && old.mesh.element_count() == 0
                        && old.geometry.is_none()
                });
                let mut turned_faces = Vec::new();
                if let (Some(old), Some(view)) = (into_empty, &geometry_view) {
                    match geometry_check(old.fe.properties.space, view) {
                        Ok(faces) => turned_faces = faces,
                        Err(error) => {
                            self.output
                                .push(format!("Error importing {}: {error}", path.display()));
                            return;
                        }
                    }
                }
                // The scene built on the worker thread is reused unless the settings change it.
                let mut rebuild = false;
                // The camera of bound results as saved, replacing the fit to the model.
                let mut camera = None;
                if let Some(view) = &mut model.results {
                    let post = &self.settings.post;
                    let levels = view.color_scale.levels;
                    if (levels, view.show_undeformed) != (post.levels, post.undeformed_outline) {
                        view.color_scale.levels = post.levels;
                        view.show_undeformed = post.undeformed_outline;
                        rebuild = true;
                    }
                }
                match &geometry_view {
                    Some(view) if model.mesh.element_count() == 0 => {
                        // Several files imported together are listed by name.
                        let source = model.geometry.as_ref().map(|g| g.source.as_str());
                        let single = path.file_name().and_then(|n| n.to_str()) == source;
                        let what = match source {
                            Some(names) if !single => names.to_string(),
                            _ => path.display().to_string(),
                        };
                        self.output.push(format!(
                            "{what} imported: {} parts ({} ms)",
                            view.parts.len(),
                            model.load_time.as_millis()
                        ));
                    }
                    _ => self.output.push(format!(
                        "{} loaded: {} nodes, {} elements, {} parts ({} ms)",
                        path.display(),
                        model.mesh.node_count(),
                        model.mesh.element_count(),
                        model.parts.len(),
                        model.load_time.as_millis()
                    )),
                }
                if model.included_files > 0 {
                    self.output
                        .push(format!("{} included file(s) read", model.included_files));
                }
                for warning in &model.warnings {
                    self.output.push(format!("Warning: {warning}"));
                }
                if !turned_faces.is_empty() {
                    let faces: Vec<String> = turned_faces.iter().map(i32::to_string).collect();
                    self.output.push(format!(
                        "Note: the normal of face {} points in -z; the elements are flipped \
                         when meshing.",
                        faces.join(", ")
                    ));
                }
                if !model.skipped_keywords.is_empty() {
                    let keywords: Vec<String> = model
                        .skipped_keywords
                        .keys()
                        .map(|k| format!("*{k}"))
                        .collect();
                    self.output
                        .push(format!("Not evaluated yet: {}", keywords.join(", ")));
                }
                if let Some(view) = &model.results {
                    self.output.push(format!(
                        "{} result increment(s) read",
                        view.increments.len()
                    ));
                    if !view.file_history.is_empty() {
                        self.output.push(format!(
                            "{} history output(s) read from the .dat file",
                            view.file_history.len()
                        ));
                    }
                }
                self.dialog = None;
                self.field_output_dialog = None;
                self.close_history_windows();
                self.viewport.labels = Default::default();
                let mut model = model;
                if let Some(view) = &mut model.results {
                    // Results are in the unit system of the open FE model, as those of its
                    // analyses are; without one in that of new models.
                    view.units = self
                        .model
                        .as_ref()
                        .map_or(self.settings.new_model.units, |m| m.fe.properties.units);
                }
                if let Some(view) = &model.results {
                    // A results file joins the results collection and leaves the FE model
                    // alone; opening the same file again replaces it.
                    self.tree.selected = Some((
                        TreeView::Results,
                        TreeItem::Component(view.field, view.component),
                    ));
                    self.set_tree_view(TreeView::Results);
                    // Results of a 2D model are flat in z and seen from the front.
                    if model
                        .mesh
                        .bounds()
                        .is_some_and(|(min, max)| (max[2] - min[2]).abs() < 1e-9)
                    {
                        self.viewport
                            .apply(ViewCommand::View(StandardView::Front), None);
                    }
                    let mut model = model;
                    // Results get the features of the FE model; reloaded ones keep their own
                    // and their paths.
                    let previous = self.results.iter().position(|r| r.path == model.path);
                    if let Some(old) = previous.map(|i| &self.results[i].fe) {
                        model.fe.add_missing_features(old);
                        model.fe.result_paths = old.result_paths.clone();
                        model.fe.result_planes = old.result_planes.clone();
                    }
                    // Results of a new run take over what was defined on those before; what
                    // refers to nodes or elements only if the mesh stayed the same.
                    let carried = (self.carried_settings.take())
                        .filter(|(path, _)| *path == model.path)
                        .map(|(_, mut settings)| {
                            if settings.mesh != Some(model.mesh.fingerprint()) {
                                let changes = settings.without_mesh_references();
                                (self.output).extend(changes.into_iter().map(|change| {
                                    format!("The mesh changed since the last results: {change}")
                                }));
                            }
                            settings
                        });
                    let carried_over = carried.is_some();
                    // Results bound to the project come back as they were saved.
                    let settings = carried.or_else(|| {
                        (self.model.as_ref())
                            .filter(|_| previous.is_none())
                            .filter(|p| p.bound_results_file.as_ref() == Some(&model.path))
                            .and_then(|p| p.bound_results.as_ref()?.settings.clone())
                    });
                    if let Some(settings) = &settings {
                        let warnings =
                            model.restore_results_settings(settings, self.moving_load_stations());
                        self.output.extend(warnings);
                        camera = settings.view.camera.map(saved_camera);
                        if let Some(edges) = settings.view.mesh_edges {
                            self.viewport.options.mesh_edges = edges;
                        }
                        if let Some(view) = &model.results {
                            self.tree.selected = Some((
                                TreeView::Results,
                                TreeItem::Component(view.field, view.component),
                            ));
                        }
                        rebuild = true;
                    }
                    if let Some(project) = &self.model {
                        model.fe.add_missing_features(&project.fe);
                    }
                    // Results read again, e.g. after the analysis ran once more, keep their
                    // colour scale.
                    let previous_scale = previous
                        .and_then(|i| self.results[i].results.as_ref())
                        .map(|v| v.color_scale.clone());
                    if let (Some(view), Some(scale)) = (model.results.as_mut(), previous_scale) {
                        view.color_scale = scale;
                    }
                    match previous {
                        Some(index) => {
                            // Results read again, e.g. after the analysis ran once more,
                            // keep their hot spots.
                            let mut model = model;
                            let old = std::mem::take(&mut self.results[index].hot_spots);
                            if !carried_over {
                                model.hot_spots.definitions = old.definitions;
                            }
                            self.results[index] = model;
                            self.current_result = index;
                        }
                        None => {
                            self.results.push(model);
                            self.current_result = self.results.len() - 1;
                        }
                    }
                    self.evaluate_hot_spots();
                } else {
                    self.tree.selected = None;
                    self.editor = None;
                    self.highlighted = None;
                    self.mesh_setup = None;
                    self.mesh_item_editor = None;
                    self.meshing = None;
                    // A model without a mesh yet opens on the Geometry tab.
                    let view = if geometry_view.is_some() && model.mesh.element_count() == 0 {
                        TreeView::Geometry
                    } else {
                        TreeView::FeModel
                    };
                    self.set_tree_view(view);
                    // Geometry imported into a model that has nothing yet, such as one just
                    // created with File > New, keeps its model space and units.
                    let mut model = model;
                    if let Some(old) = &self.model
                        && plx_mesher::is_cad_file(&path)
                        && old.mesh.element_count() == 0
                        && old.geometry.is_none()
                    {
                        take_over_empty_model(&mut model, old);
                    }
                    // 2D models lie in the x-y plane and are seen from the front.
                    if model.fe.properties.space.is_2d() {
                        self.viewport
                            .apply(ViewCommand::View(StandardView::Front), None);
                    }
                    self.model = Some(model);
                    self.symbols_shown = None;
                    self.geometry = geometry_view;
                    // Results bound to the project open with it.
                    if self
                        .model
                        .as_ref()
                        .is_some_and(|m| m.bound_results.is_some())
                    {
                        self.open_bound_results();
                    }
                }
                self.frame_cache.clear();
                self.viewport.set_parts(&render_meshes);
                let hidden: Vec<usize> = (self.shown().into_iter())
                    .flat_map(|m| m.parts.iter().enumerate())
                    .filter(|(_, p)| !p.visible)
                    .map(|(i, _)| i)
                    .collect();
                for part in hidden {
                    self.viewport.set_part_visible(part, false);
                }
                self.results_changed = rebuild;
                self.update_contour();
                match camera {
                    Some(camera) => {
                        self.viewport.set_camera(camera);
                        self.view_command = None;
                    }
                    None => self.view_command = Some(ViewCommand::Fit),
                }
            }
            Err(error) => self.output.push(format!("Error loading: {error}")),
        }
    }

    /// Tab strip above the tree, like PrePoMax's Windows tab control.
    /// Returns the rect of the selected tab.
    fn tree_tabs(&mut self, ui: &mut egui::Ui) -> Option<egui::Rect> {
        let mut selected_rect = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(2.0);
            for view in [TreeView::Geometry, TreeView::FeModel, TreeView::Results] {
                let selected = self.tree_view == view;
                let galley = ui.painter().layout_no_wrap(
                    view.title().to_string(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    crate::style::text(),
                );
                let size = galley.size() + egui::vec2(16.0, 8.0);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                let fill = if selected {
                    crate::style::window()
                } else if response.hovered() {
                    crate::style::hover_fill()
                } else {
                    crate::style::control()
                };
                let rect = if selected {
                    selected_rect = Some(rect);
                    rect
                } else {
                    rect.shrink2(egui::vec2(0.0, 1.0))
                        .translate(egui::vec2(0.0, 1.0))
                };
                ui.painter().rect(
                    rect,
                    0.0,
                    fill,
                    egui::Stroke::new(1.0, crate::style::border()),
                    egui::StrokeKind::Inside,
                );
                ui.painter().galley(
                    rect.center() - galley.size() * 0.5,
                    galley,
                    crate::style::text(),
                );
                if response.clicked() {
                    self.set_tree_view(view);
                }
            }
        });
        selected_rect
    }

    fn output(&self, ui: &mut egui::Ui) {
        crate::virtual_table::lines(
            ui,
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .stick_to_bottom(true),
            &self.output,
        );
    }

    fn model_tree(&mut self, ui: &mut egui::Ui, view: TreeView) {
        let empty = match view {
            TreeView::Geometry => self.geometry.is_none().then_some("No geometry loaded"),
            TreeView::FeModel => self.model.is_none().then_some("No model loaded"),
            TreeView::Results => self.results.is_empty().then_some("No results loaded"),
        };
        if let Some(text) = empty {
            let hint = if view == TreeView::Geometry {
                "Geometry > Import or drag a STEP, IGES or BREP file into the window."
            } else {
                "File > Open (Ctrl+O) or drag a .plx, .inp or .frd file into the window."
            };
            ui.weak(format!("{text}.\n{hint}"));
            ui.separator();
        }
        let mesh_items: Vec<String> = (self.model.as_ref())
            .and_then(|m| m.geometry.as_ref())
            .map(|g| g.mesh_items.iter().map(|i| i.name.clone()).collect())
            .unwrap_or_default();
        let jobs = self.job_states();
        let shown = match view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        };
        let response = tree::show(
            ui,
            view,
            shown,
            &mesh_items,
            &jobs,
            &self.solver_findings,
            &mut self.tree,
        );
        self.tree_response(ui.ctx(), view, response);
    }

    /// Acts on what the user picked in the tree or in a part's context menu in the 3D view.
    fn tree_response(&mut self, ctx: &egui::Context, view: TreeView, response: TreeResponse) {
        for (index, visible) in response.visibility {
            if let Some(part) = self.shown_mut().and_then(|m| m.parts.get_mut(index)) {
                part.visible = visible;
            }
            self.viewport.set_part_visible(index, visible);
        }
        if !response.contours.is_empty() {
            for (index, contours) in response.contours {
                if let Some(model) = self.shown_mut() {
                    model.set_contours(&[index], contours);
                }
            }
            self.results_changed = true;
        }
        if let (Some((field, component)), Some(results)) =
            (response.component, self.shown_results_mut())
        {
            results.field = field;
            results.component = component;
            self.results_changed = true;
        }
        if let (Some(shown), Some(results)) = (response.plane_result, self.shown_results_mut()) {
            results.plane_result = shown;
        }
        if let Some(TreeItem::Part(index)) = response.open {
            let name = self.shown().and_then(|m| m.parts.get(index));
            let types = (name.into_iter())
                .flat_map(|p| p.element_types.iter().map(|(t, _)| t.clone()))
                .collect();
            self.part_name = (name.map(|p| p.name.clone()).unwrap_or_default(), None);
            self.part_types = types;
            self.dialog = Some(TreeItem::Part(index));
        } else if let Some(TreeItem::ResultFieldOutput(field)) = response.open {
            self.edit_field_output(field);
        } else if let Some(TreeItem::HistorySet(set)) = response.open {
            self.edit_history_output(set);
        } else if let Some(TreeItem::HotSpot(index)) = response.open {
            self.edit_hot_spot(index);
        } else if let Some(TreeItem::HistoryComponent(set, field, component)) = response.open {
            let same = (self.history_table.as_ref())
                .is_some_and(|t| (t.set, t.field, t.component) == (set, field, component));
            if !same {
                self.history_table = Some(HistoryTable::new(set, field, component));
            }
        } else if let (Some(TreeItem::Model), TreeView::FeModel) = (&response.open, view) {
            self.edit_model_properties();
        } else if let Some(TreeItem::MeshItem(index)) = response.open {
            self.mesh_item_editor = self.model.as_ref().and_then(|m| {
                MeshItemEditor::edit(m.geometry.as_ref()?, index, m.fe.properties.units)
            });
        } else if let Some(feature) = response.open.as_ref().and_then(tree::feature) {
            let results = (view == TreeView::Results).then_some(self.current_result);
            if let Some(model) = self.feature_model(results) {
                self.feature_dialog = FeatureDialog::edit(feature, results, model);
            }
        } else if let Some(item) = response.open {
            let model = self.model.as_ref().filter(|_| view != TreeView::Results);
            match model.and_then(|m| Editor::edit(&item, &m.fe, &m.mesh)) {
                Some(editor) => self.editor = Some(editor),
                None => self.dialog = Some(item),
            }
        }
        if let Some(kind) = response.create {
            self.create(kind);
        }
        if let Some(item) = response.delete {
            // A part among several selected ones deletes them all.
            let parts = self.tree.selected_parts(view);
            let items = match item {
                TreeItem::Part(index) if parts.contains(&index) => {
                    parts.into_iter().map(TreeItem::Part).collect()
                }
                item => vec![item],
            };
            self.confirm_delete = Some((view, items));
        }
        if let (Some((s, up)), Some(model)) = (response.move_step, self.model.as_mut())
            && let Some(to) = crate::setup::move_step(&mut model.fe, s, up)
        {
            // The moved step stays selected.
            if let Some((_, item @ TreeItem::Step(_))) = &mut self.tree.selected {
                *item = TreeItem::Step(to);
            }
        }
        if let (Some(item), Some(model)) = (response.toggle_active, self.model.as_mut()) {
            crate::setup::toggle_active(&mut model.fe, &item);
        }
        if let Some(model) = self.model.as_mut() {
            for (item, visible) in &response.symbol_visibility {
                crate::setup::set_visible(&mut model.fe, item, *visible);
            }
        }
        if let (Some(item), Some(model)) = (response.swap_master_slave, self.model.as_mut())
            && crate::setup::swap_master_slave(&mut model.fe, &item)
        {
            // The selection stays, but master and slave colours change places.
            self.highlighted = None;
        }
        if let Some((index, action)) = response.analysis {
            self.analysis_action(index, action);
        }
        if response.material_library {
            self.open_material_library();
        }
        if response.mesh_defaults {
            self.open_mesh_setup();
        }
        if response.generate_mesh {
            self.generate_mesh(ctx, None);
        }
        if let Some(index) = response.mesh_part {
            // A part among several selected ones meshes them all.
            let selected = self.tree.selected_parts(view);
            let indices: Vec<usize> = if selected.contains(&index) {
                selected.into_iter().collect()
            } else {
                vec![index]
            };
            let names: Vec<String> = match self.geometry.as_ref() {
                Some(g) => (indices.iter())
                    .filter_map(|&i| g.parts.get(i).map(|p| p.name.clone()))
                    .collect(),
                None => Vec::new(),
            };
            if !names.is_empty() {
                self.generate_mesh(ctx, Some(names));
            }
        }
        if response.search_contacts {
            self.open_contact_search();
        }
        if let Some(index) = response.transform_part {
            // A part among several selected ones transforms them all.
            let parts = self.tree.selected_parts(view);
            let parts = if parts.contains(&index) {
                parts.into_iter().collect()
            } else {
                vec![index]
            };
            self.open_mesh_tool(MeshTool::Transform(parts));
        }
        if response.merge_parts {
            let parts: Vec<usize> = self.tree.selected_parts(view).into_iter().collect();
            self.merge_mesh_parts(&parts);
        }
        if let Some(findings) = response.findings {
            self.findings_window = Some(findings);
        }
    }

    /// Explains the findings of the tree item whose warning sign was clicked: what is wrong,
    /// why CalculiX cannot cope with it and how to fix it.
    fn findings_window(&mut self, ctx: &egui::Context) {
        let Some(findings) = &self.findings_window else {
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new("Model Check")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.6)
                    .show(ui, |ui| {
                        for (i, finding) in findings.iter().enumerate() {
                            if i > 0 {
                                ui.separator();
                            }
                            let (kind, color) = match finding.severity() {
                                plx_model::Severity::Error => ("Error", crate::style::error()),
                                plx_model::Severity::Warning => {
                                    ("Warning", crate::style::warning())
                                }
                            };
                            ui.horizontal(|ui| {
                                ui.colored_label(color, egui::RichText::new(kind).strong());
                                ui.label(egui::RichText::new(finding.problem.title()).strong());
                            });
                            ui.add(
                                egui::Label::new(egui::RichText::new(&finding.detail).weak())
                                    .wrap(),
                            );
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(finding.problem.explanation()).wrap());
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(format!("Remedy: {}", finding.problem.fix()))
                                    .wrap(),
                            );
                        }
                    });
                ui.separator();
                ui.vertical_centered(|ui| {
                    close = ui.button("Close").clicked();
                });
            });
        if !open || close {
            self.findings_window = None;
        }
    }

    /// Evaluates the hot spots of the current results file over all increments and writes
    /// the values next to it.
    fn evaluate_hot_spots(&mut self) {
        let moving_loads = self.moving_load_stations();
        let Some(results) = self.results.get_mut(self.current_result) else {
            return;
        };
        let reports = crate::hot_spots::evaluate(results);
        let hot_spots = &mut results.hot_spots;
        hot_spots.reports = Vec::new();
        hot_spots.file = None;
        if !reports.is_empty() {
            match crate::hot_spots::write(&results.path, &reports) {
                Ok(file) => {
                    self.output
                        .push(format!("Hot spots evaluated: {}", file.display()));
                    hot_spots.file = Some(file);
                }
                Err(error) => self.output.push(error),
            }
            self.output.extend(crate::hot_spots::summary(&reports));
            for report in &reports {
                self.output
                    .extend(report.warnings.iter().map(|w| format!("Warning: {w}")));
            }
            hot_spots.reports = reports.clone();
        }
        // History outputs of the hot spots follow them.
        if let Some(view) = &mut results.results
            && let Some(warnings) = view.set_history_sources(reports, moving_loads, &results.mesh)
        {
            (self.output).extend(warnings.into_iter().map(|w| format!("Warning: {w}")));
            self.history_table = None;
        }
        self.update_contour();
    }

    /// Takes the moving loads of the FE model as they are now into the current results, for
    /// the history outputs.
    fn refresh_moving_loads(&mut self) {
        let moving_loads = self.moving_load_stations();
        let Some(results) = self.results.get_mut(self.current_result) else {
            return;
        };
        if let Some(view) = &mut results.results {
            let reports = view.hot_spot_reports.clone();
            if let Some(warnings) = view.set_history_sources(reports, moving_loads, &results.mesh) {
                (self.output).extend(warnings.into_iter().map(|w| format!("Warning: {w}")));
                self.history_table = None;
            }
        }
    }

    /// Where the moving loads of the FE model stand in the steps of its results.
    fn moving_load_stations(&self) -> Vec<plx_model::LoadStations> {
        (self.model.as_ref()).map_or_else(Vec::new, |project| plx_model::load_stations(&project.fe))
    }

    fn edit_hot_spot(&mut self, index: usize) {
        if let Some(model) = self.results.get(self.current_result) {
            self.hot_spot_dialog =
                HotSpotDialog::edit(&model.hot_spots.definitions, index, &model.mesh);
        }
    }

    fn delete_hot_spot(&mut self, index: usize) {
        let Some(model) = self.results.get_mut(self.current_result) else {
            return;
        };
        if index >= model.hot_spots.definitions.len() {
            return;
        }
        let removed = model.hot_spots.definitions.remove(index);
        self.output
            .push(format!("Hot spot {} deleted", removed.name));
        self.hot_spot_dialog = None;
        self.tree.selected = None;
        self.evaluate_hot_spots();
    }

    fn hot_spot_dialog_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (
            &mut self.hot_spot_dialog,
            self.results.get_mut(self.current_result),
        ) else {
            return;
        };
        let action = dialog.show(ctx, model);
        let plot_name = dialog.plot_name(&model.hot_spots.definitions);
        if dialog.take_plot_toggle() {
            match model
                .path_plots
                .iter()
                .position(|p| p.is_hot_spot(&plot_name))
            {
                Some(index) => {
                    model.path_plots.remove(index);
                }
                None => {
                    model.path_plots.push(PathPlot::hot_spot(plot_name.clone()));
                    dialog.set_plot_key(plot_name.clone());
                }
            }
        }
        let (hot_spot, next) = match action {
            DialogAction::Open => return,
            DialogAction::Cancel => {
                // The plot of a hot spot that was not created goes with it.
                if dialog.edit.is_none() {
                    model.path_plots.retain(|p| !p.is_hot_spot(&plot_name));
                }
                self.hot_spot_dialog = None;
                return;
            }
            DialogAction::Ok { output, next } => (output, next),
        };
        // The plot follows a renamed hot spot.
        for plot in model.path_plots.iter_mut() {
            if plot.is_hot_spot(&plot_name) {
                plot.path = hot_spot.name.clone();
            }
        }
        let definitions = &mut model.hot_spots.definitions;
        let verb = match dialog.edit.filter(|&i| i < definitions.len()) {
            Some(index) => {
                definitions[index] = hot_spot.clone();
                "modified"
            }
            None => {
                definitions.push(hot_spot.clone());
                "created"
            }
        };
        self.output
            .push(format!("Hot spot {} {verb}", hot_spot.name));
        if next {
            dialog.next(&hot_spot);
        } else {
            self.hot_spot_dialog = None;
        }
        self.evaluate_hot_spots();
    }

    /// Finds the paths of the hot spot being edited or selected in the Results tree.
    fn update_hot_spot_preview(&mut self) {
        let current = self.current_result;
        let model = self.results.get(current);
        let wanted = match (&self.hot_spot_dialog, &self.tree.selected) {
            _ if self.tree_view != TreeView::Results => None,
            (Some(dialog), _) => Some(dialog.hot_spot()),
            (None, Some((TreeView::Results, TreeItem::HotSpot(i)))) => {
                model.and_then(|m| m.hot_spots.definitions.get(*i).cloned())
            }
            _ => None,
        };
        let shown = (self.hot_spot_preview.as_ref()).map(|(h, index, _)| (h, *index));
        let plotted = (self.hot_spot_dialog.as_ref()).and_then(HotSpotDialog::plotted_node);
        let plotted_changed = std::mem::replace(&mut self.hot_spot_plotted, plotted) != plotted;
        if wanted.as_ref().map(|h| (h, current)) == shown {
            if plotted_changed {
                self.update_contour();
            }
            return;
        }
        self.hot_spot_preview = match (wanted, model) {
            (Some(hot_spot), Some(model)) => {
                let paths = crate::hot_spots::paths(model, &hot_spot);
                Some((hot_spot, current, paths))
            }
            _ => None,
        };
        self.update_contour();
    }

    /// Hot spot paths drawn over the results, deformed like them: of the hot spot edited or
    /// selected.
    fn overlay_paths(&self) -> Vec<Vec<glam::Vec3>> {
        let model = self.results.get(self.current_result);
        let Some(model) = model.filter(|_| self.tree_view == TreeView::Results) else {
            return Vec::new();
        };
        // The dialog shows the path of one toe node, that of the highest hot spot stress.
        let plotted = (self.hot_spot_dialog.as_ref()).and_then(HotSpotDialog::plotted_node);
        match &self.hot_spot_preview {
            Some((_, index, paths)) if *index == self.current_result => {
                let paths = paths.iter().filter(|p| plotted.is_none_or(|n| p.node == n));
                crate::hot_spots::render_paths(model, paths)
            }
            _ => Vec::new(),
        }
    }

    /// The y axis of an axisymmetric model as PrePoMax shows it, from below to above the
    /// shown model, in render coordinates. Results show it when they are flat like the 2D
    /// model they come from.
    fn axis_line(&self, shown: &Model) -> Option<[glam::Vec3; 2]> {
        let space = self.model.as_ref()?.fe.properties.space;
        if space != plx_model::ModelSpace::Axisymmetric {
            return None;
        }
        if shown.is_results() {
            let (min, max) = shown.mesh.bounds()?;
            let flat = (max[2] - min[2]).abs() <= 1e-6 * (max[0] - min[0]).abs().max(1e-30);
            if !flat {
                return None;
            }
        }
        let (min, max) = shown.visible_bounds()?;
        let origin = shown.global_origin();
        let margin = 0.1 * (max.y - min.y).max(max.x - min.x);
        Some([
            glam::Vec3::new(origin.x, min.y - margin, origin.z),
            glam::Vec3::new(origin.x, max.y + margin, origin.z),
        ])
    }

    /// The FE model, which can be set up.
    fn setup_model(&self) -> Option<&Model> {
        self.model.as_ref()
    }

    /// What the 3D view shows: the current results on the Results tab, else the FE model.
    fn shown(&self) -> Option<&Model> {
        match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        }
    }

    fn shown_mut(&mut self) -> Option<&mut Model> {
        match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        }
    }

    fn shown_results_mut(&mut self) -> Option<&mut ResultsView> {
        self.shown_mut()?.results.as_mut()
    }

    /// Whether clicks in the 3D view pick for an open dialog: an item dialog of the FE model
    /// or a history output or hot spot dialog of the results.
    fn picking(&self) -> bool {
        if self.query.is_some() {
            return true;
        }
        match self.tree_view {
            TreeView::Results => {
                (self.history_dialog.as_ref()).is_some_and(HistoryOutputDialog::picks)
                    || (self.hot_spot_dialog.as_ref()).is_some_and(HotSpotDialog::picks)
            }
            TreeView::FeModel => self.editor.as_ref().is_some_and(Editor::picks),
            TreeView::Geometry => {
                (self.mesh_item_editor.as_ref()).is_some_and(MeshItemEditor::picks)
            }
        }
    }

    /// The results shown on the Results tab.
    fn shown_results_view(&self) -> Option<&ResultsView> {
        self.results.get(self.current_result)?.results.as_ref()
    }

    fn close_history_windows(&mut self) {
        self.history_dialog = None;
        self.hot_spot_dialog = None;
        self.history_table = None;
    }

    /// Switches the tree tab. The Results tab is a workspace of its own, as in PrePoMax: the
    /// 3D view swaps between the FE model and the current results, each with its own camera.
    fn set_tree_view(&mut self, view: TreeView) {
        if self.tree_view == view {
            return;
        }
        let was_results = self.tree_view == TreeView::Results;
        if was_results == (view == TreeView::Results) {
            // Geometry and FE model share the camera; the scene changes.
            self.tree_view = view;
            self.results_changed = true;
            return;
        }
        // A fit still pending, e.g. right after loading, belongs to the workspace left.
        if let Some(command) = self.view_command.take() {
            let bounds = self.shown().and_then(Model::visible_bounds);
            self.viewport.apply(command, bounds);
        }
        self.tree_view = view;
        self.dialog = None;
        self.viewport.labels = Default::default();
        match &mut self.parked_camera {
            Some(camera) => self.viewport.swap_camera(camera),
            None => {
                let mut camera = plx_render::Camera::default();
                self.viewport.swap_camera(&mut camera);
                self.parked_camera = Some(camera);
                self.view_command = Some(ViewCommand::Fit);
            }
        }
        self.results_changed = true;
    }

    /// Shows another results file of the collection, like PrePoMax's Result box.
    fn select_result(&mut self, index: usize) {
        if index != self.current_result && index < self.results.len() {
            self.current_result = index;
            if let Some(view) = &self.results[index].results {
                self.tree.selected = Some((
                    TreeView::Results,
                    TreeItem::Component(view.field, view.component),
                ));
            }
            self.dialog = None;
            self.field_output_dialog = None;
            self.transformation_dialog = None;
            self.close_history_windows();
            self.viewport.labels = Default::default();
            self.results_changed = true;
        }
    }

    /// Closes the current results file, or all of them.
    fn close_results(&mut self, all: bool) {
        let closed: Vec<Model> = if all {
            std::mem::take(&mut self.results)
        } else if self.current_result < self.results.len() {
            vec![self.results.remove(self.current_result)]
        } else {
            Vec::new()
        };
        for model in closed {
            self.output.push(format!("{} closed", model.file_name()));
        }
        self.current_result = self
            .current_result
            .min(self.results.len().saturating_sub(1));
        self.field_output_dialog = None;
        self.transformation_dialog = None;
        self.close_history_windows();
        if self.tree_view == TreeView::Results {
            self.dialog = None;
            self.tree.selected = None;
            self.results_changed = true;
        }
    }

    /// PrePoMax's Results menu.
    fn results_menu(&mut self, ui: &mut egui::Ui) {
        let any = !self.results.is_empty();
        let shown = self.tree_view == TreeView::Results
            && (self.results.get(self.current_result)).is_some_and(|m| m.results.is_some());
        if ui
            .add_enabled(shown, egui::Button::new("Color Scale …"))
            .clicked()
        {
            self.open_color_scale();
        }
        ui.separator();
        if ui
            .add_enabled(any, egui::Button::new("Create Hot Spot …"))
            .clicked()
        {
            self.create(NewItem::ResultHotSpot);
        }
        ui.separator();
        if ui
            .add_enabled(any, egui::Button::new("Close Current Results"))
            .clicked()
        {
            self.close_results(false);
        }
        if ui
            .add_enabled(any, egui::Button::new("Close All Results"))
            .clicked()
        {
            self.close_results(true);
        }
    }

    /// The results row of the tool bar: PrePoMax's Result box with all opened results
    /// files, then the controls of the shown result.
    fn results_tool_bar(&mut self, ui: &mut egui::Ui) {
        let enabled = self.tree_view == TreeView::Results && !self.results.is_empty();
        ui.add_enabled_ui(enabled, |ui| self.results_tool_bar_row(ui, enabled));
    }

    fn results_tool_bar_row(&mut self, ui: &mut egui::Ui, enabled: bool) {
        ui.horizontal(|ui| {
            ui.label("Result");
            let mut selected = self.current_result;
            let current = self
                .results
                .get(selected)
                .map(|m| m.path.display().to_string());
            egui::ComboBox::from_id_salt("result file")
                .selected_text(current.clone().unwrap_or_default())
                .width(320.0)
                .truncate()
                .show_ui(ui, |ui| {
                    for (index, model) in self.results.iter().enumerate() {
                        ui.selectable_value(&mut selected, index, model.path.display().to_string());
                    }
                })
                .response
                .on_hover_text(current.unwrap_or_default());
            self.select_result(selected);
            ui.separator();
            // Greyed out, the row shows the current results file, or empty controls.
            let mut placeholder = None;
            let view = match self.results.get_mut(self.current_result) {
                Some(model) => model.results.as_mut(),
                None => None,
            }
            .unwrap_or_else(|| placeholder.insert(ResultsView::new(Vec::new(), None)));
            let transformed = !view.transformations.is_empty();
            let response = results_tool_bar(ui, view, transformed);
            if response.changed && enabled {
                self.results_changed = true;
            }
            if response.transformations && enabled {
                self.open_transformation_dialog();
            }
            if response.color_scale && enabled {
                self.open_color_scale();
            }
        });
    }

    fn open_transformation_dialog(&mut self) {
        let index = self.current_result;
        if let Some(model) = self.results.get(index)
            && let Some(view) = &model.results
        {
            let dialog = TransformationDialog::new(index, &view.transformations, model);
            self.transformation_dialog = Some(dialog);
            self.update_contour();
        }
    }

    fn transformation_picks(&self) -> bool {
        (self.transformation_dialog.as_ref())
            .is_some_and(|d| d.picks() && self.tree_view == TreeView::Results)
    }

    fn transformation_window(&mut self, ctx: &egui::Context) {
        if self.tree_view != TreeView::Results {
            self.transformation_dialog = None;
        }
        let Some(dialog) = &mut self.transformation_dialog else {
            return;
        };
        let Some(model) = self.results.get_mut(dialog.result) else {
            self.transformation_dialog = None;
            return;
        };
        let shown = (dialog.lines(model), dialog.points(model));
        let action = dialog.show(ctx, model);
        // The overlay shows the points and axis of the selected item.
        let moved = shown != (dialog.lines(model), dialog.points(model));
        let (transformations, close) = match action {
            TransformationAction::Open => {
                if moved {
                    self.update_contour();
                }
                return;
            }
            TransformationAction::Cancel => {
                self.transformation_dialog = None;
                self.viewport.preview = Default::default();
                self.update_contour();
                return;
            }
            TransformationAction::Apply {
                transformations,
                close,
            } => (transformations, close),
        };
        if let Some(view) = &mut model.results {
            view.transformations = transformations;
            self.results_changed = true;
            // The copies extend the scene; show all of it.
            self.view_command = Some(ViewCommand::Fit);
        }
        if close {
            self.transformation_dialog = None;
            self.viewport.preview = Default::default();
        }
        self.update_contour();
    }

    fn create(&mut self, kind: NewItem) {
        if kind == NewItem::Analysis {
            if let Some(model) = self.setup_model() {
                let dialog = crate::analysis_dialog::AnalysisDialog::create(&model.fe.analyses);
                self.analysis_dialog = Some(dialog);
                self.set_tree_view(TreeView::FeModel);
            }
            return;
        }
        if kind == NewItem::ResultHotSpot {
            if let Some(model) = self.results.get(self.current_result) {
                self.hot_spot_dialog = Some(HotSpotDialog::create(&model.hot_spots.definitions));
                self.set_tree_view(TreeView::Results);
            }
            return;
        }
        if let NewItem::Feature(kind) = kind {
            // Paths read results; features belong to the tab they are created on.
            let on_results = matches!(kind, FeatureKind::ResultPath | FeatureKind::ResultPlane);
            let results =
                (on_results || self.tree_view == TreeView::Results).then_some(self.current_result);
            if let Some(model) = self.feature_model(results) {
                self.feature_dialog = Some(FeatureDialog::create(kind, results, model));
                if results.is_some() {
                    self.set_tree_view(TreeView::Results);
                }
            }
            return;
        }
        if kind == NewItem::ResultHistoryOutput {
            self.refresh_moving_loads();
            if let Some(view) = self.shown_results_view() {
                self.history_dialog = Some(HistoryOutputDialog::create(view));
            }
            return;
        }
        if kind == NewItem::MeshSetupItem {
            if let Some(model) = &self.model
                && let Some(geometry) = &model.geometry
            {
                let units = model.fe.properties.units;
                self.mesh_item_editor = Some(MeshItemEditor::create(geometry, units));
                self.set_tree_view(TreeView::Geometry);
            }
            return;
        }
        if let NewItem::MeshSet(kind) = kind {
            if let Some(model) = self.setup_model() {
                self.editor = Some(Editor::create_set(kind, &model.fe, &model.mesh));
                self.set_tree_view(TreeView::FeModel);
            }
            return;
        }
        if kind == NewItem::ResultFieldOutput {
            if let Some(model) = self.results.get(self.current_result)
                && let Some(view) = &model.results
            {
                self.field_output_dialog = Some(FieldOutputDialog::create(view, &model.mesh));
            }
            return;
        }
        if let Some(model) = self.setup_model() {
            self.editor = Editor::create(kind, &model.fe);
            self.set_tree_view(TreeView::FeModel);
        }
    }

    /// PrePoMax's Mesh menu: the mesh setup and mesh generation for the geometry.
    fn mesh_menu(&mut self, ui: &mut egui::Ui) {
        let has_geometry = self.model.as_ref().is_some_and(|m| m.geometry.is_some());
        let has_mesh = (self.model.as_ref()).is_some_and(|m| m.mesh.element_count() > 0);
        if !has_geometry && !has_mesh {
            ui.label("Import a geometry first");
            return;
        }
        if has_geometry {
            if ui.button("Create Mesh Setup Item …").clicked() {
                self.create(NewItem::MeshSetupItem);
            }
            if ui.button("Default Mesh Parameters …").clicked() {
                self.open_mesh_setup();
            }
            ui.separator();
            let mesh = egui::Button::new("Mesh All Parts");
            if ui.add_enabled(self.meshing.is_none(), mesh).clicked() {
                self.generate_mesh(ui.ctx(), None);
            }
        }
        if has_mesh {
            if has_geometry {
                ui.separator();
            }
            if ui.button("Merge coincident nodes ...").clicked() {
                self.open_mesh_tool(MeshTool::MergeNodes);
            }
            if ui.button("Renumber nodes and elements ...").clicked() {
                self.open_mesh_tool(MeshTool::Renumber);
            }
        }
    }

    fn open_mesh_tool(&mut self, tool: MeshTool) {
        if let Some(model) = self.setup_model() {
            self.mesh_tool = Some(MeshToolDialog::new(tool, model));
            self.editor = None;
            self.set_tree_view(TreeView::FeModel);
        }
    }

    fn mesh_tool_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (&mut self.mesh_tool, &mut self.model) else {
            return;
        };
        match dialog.show(ctx, model) {
            MeshToolResult::Open => {}
            // On an error the dialog shows it and stays open.
            MeshToolResult::Apply => {
                if let Ok(done) = dialog.apply(model) {
                    self.output.push(done);
                    self.mesh_tool = None;
                    self.highlighted = None;
                    // The symbols of loads and supports sit on nodes that moved or changed.
                    self.symbols_shown = None;
                    self.results_changed = true;
                }
            }
            MeshToolResult::Cancel => self.mesh_tool = None,
        }
    }

    fn query_window(&mut self, ctx: &egui::Context) {
        let Some(query) = &mut self.query else {
            return;
        };
        let model = match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        };
        match query.show(ctx, model, &mut self.output) {
            QueryAction::Open => {}
            QueryAction::Changed => self.update_contour(),
            QueryAction::Add(annotation) => {
                if let Some(model) = self.shown_mut() {
                    model.query_annotations.push(annotation);
                }
                self.update_contour();
            }
            QueryAction::RemoveAll => self.annotations(AnnotationCommand::DeleteAll),
            QueryAction::Close => self.close_query(),
        }
    }

    fn open_query(&mut self) {
        self.query = Some(QueryWindow::default());
    }

    /// Closes the Query window; its annotations stay in the view, as in PrePoMax.
    fn close_query(&mut self) {
        self.query = None;
        self.viewport.preview = Default::default();
        self.update_contour();
    }

    fn has_annotations(&self) -> bool {
        self.shown()
            .is_some_and(|model| !model.query_annotations.is_empty())
    }

    /// Deletes query annotations of the shown model, from their context menu or with Remove
    /// Annotations.
    fn annotations(&mut self, command: AnnotationCommand) {
        // The label of an annotation dragged elsewhere keeps its place when one before it
        // goes.
        let labels = &mut self.viewport.labels.annotations;
        match command {
            AnnotationCommand::Delete(index) => {
                let owners = &self.viewport.overlay.annotation_owners;
                if let Some(marker) = owners.iter().position(|&owner| owner == index)
                    && marker < labels.len()
                {
                    labels.remove(marker);
                }
            }
            AnnotationCommand::DeleteAll => labels.clear(),
        }
        let Some(model) = self.shown_mut() else {
            return;
        };
        match command {
            AnnotationCommand::Delete(index) if index < model.query_annotations.len() => {
                model.query_annotations.remove(index);
            }
            AnnotationCommand::Delete(_) => {}
            AnnotationCommand::DeleteAll => model.query_annotations.clear(),
        }
        self.update_contour();
    }

    fn open_mesh_setup(&mut self) {
        if let Some(model) = &self.model
            && let Some(geometry) = &model.geometry
        {
            let units = model.fe.properties.units;
            self.mesh_setup = Some(MeshSetupWindow::new(&geometry.meshing, units));
        }
    }

    fn mesh_setup_window(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.mesh_setup else {
            return;
        };
        let (setup, mesh) = match window.show(ctx) {
            MeshSetupResult::Open => return,
            MeshSetupResult::Cancel => (None, false),
            MeshSetupResult::Ok(setup) => (Some(setup), false),
            MeshSetupResult::Mesh(setup) => (Some(setup), true),
        };
        self.mesh_setup = None;
        if let (Some(setup), Some(geometry)) =
            (setup, self.model.as_mut().and_then(|m| m.geometry.as_mut()))
        {
            geometry.meshing = setup;
        }
        if mesh {
            self.generate_mesh(ctx, None);
        }
    }

    /// The dialog of a mesh setup item; OK stores the item in the geometry.
    fn mesh_item_window(&mut self, ctx: &egui::Context) {
        let (Some(editor), Some(geometry)) = (
            &mut self.mesh_item_editor,
            self.model.as_mut().and_then(|m| m.geometry.as_mut()),
        ) else {
            self.mesh_item_editor = None;
            return;
        };
        let mut remesh = None;
        match editor.show(ctx, geometry, self.geometry.as_ref()) {
            MeshItemResult::Open => return,
            MeshItemResult::Cancel => {}
            result @ (MeshItemResult::Ok(..) | MeshItemResult::OkRemesh(..)) => {
                let (index, item, remeshes) = match result {
                    MeshItemResult::Ok(index, item) => (index, item, false),
                    MeshItemResult::OkRemesh(index, item) => (index, item, true),
                    MeshItemResult::Open | MeshItemResult::Cancel => return,
                };
                if remeshes {
                    // A local mesh size is picked on faces and edges: mesh every part.
                    remesh = Some(item.kind.parts().map(<[String]>::to_vec));
                }
                let index = match index.filter(|&i| i < geometry.mesh_items.len()) {
                    Some(index) => {
                        geometry.mesh_items[index] = item;
                        index
                    }
                    None => {
                        geometry.mesh_items.push(item);
                        geometry.mesh_items.len() - 1
                    }
                };
                self.tree.selected = Some((TreeView::Geometry, TreeItem::MeshItem(index)));
            }
        }
        self.mesh_item_editor = None;
        self.viewport.preview = Default::default();
        if let Some(parts) = remesh {
            self.generate_mesh(ctx, parts);
        }
    }

    /// Meshes the named parts of the geometry, or all with `None`, on a worker thread;
    /// [`Self::poll_meshing`] takes the result.
    fn generate_mesh(&mut self, ctx: &egui::Context, parts: Option<Vec<String>>) {
        if self.meshing.is_some() {
            return;
        }
        let Some(geometry) = self.model.as_ref().and_then(|m| m.geometry.clone()) else {
            return;
        };
        let what = match &parts {
            Some(parts) => parts.join(", "),
            None => geometry.source.clone(),
        };
        self.output.push(format!("Meshing {what} …"));
        self.meshing = Some(MeshingJob::start(geometry, parts, ctx));
        self.mesh_failure.clear();
    }

    fn poll_meshing(&mut self) {
        let Some(result) = self.meshing.as_ref().and_then(MeshingJob::poll) else {
            return;
        };
        let started = self.meshing.take().map(|job| job.started);
        let Some(model) = self.model.as_mut() else {
            return;
        };
        match result {
            Ok(generated) => {
                let had_mesh = model.mesh.element_count() > 0;
                let mut mesh = model.mesh.clone();
                let space = model.fe.properties.space;
                for mut part in generated.meshes {
                    if let Err(error) = space.prepare_generated_mesh(&mut part.mesh) {
                        self.output.push(format!("Meshing failed: {error}"));
                        return;
                    }
                    for warning in &part.warnings {
                        self.output.push(format!("Gmsh: {warning}"));
                    }
                    for p in &part.mesh.parts {
                        let parameters = model.geometry.as_ref().map(|g| g.parameters(&p.name));
                        self.output.push(format!(
                            "{}: {} elements (order {}, element size {} to {})",
                            p.name,
                            p.elements.len(),
                            if parameters.is_some_and(|s| s.second_order) {
                                2
                            } else {
                                1
                            },
                            parameters.map_or(0.0, |s| s.min_size),
                            parameters.map_or(0.0, |s| s.max_size),
                        ));
                    }
                    mesh = plx_mesher::merge_part(&mesh, part.mesh);
                }
                model.set_mesh(mesh);
                let lost = lost_selections(&model.fe, &model.mesh);
                if had_mesh && lost > 0 {
                    self.output.push(format!(
                        "Note: {lost} selections made of node or element numbers refer \
                         to the old mesh and must be selected again"
                    ));
                }
                self.output.push(format!(
                    "Mesh created: {} nodes, {} elements, {} parts ({} ms)",
                    model.mesh.node_count(),
                    model.mesh.element_count(),
                    model.parts.len(),
                    started.map_or(0, |s| s.elapsed().as_millis())
                ));
                self.highlighted = None;
                self.mesh_item_editor = None;
                self.set_tree_view(TreeView::FeModel);
                self.results_changed = true;
                if !had_mesh {
                    self.view_command = Some(ViewCommand::Fit);
                }
            }
            Err(error) => {
                let message = error.to_string();
                self.mesh_failure = plx_mesher::named_entities(&message).into_iter().collect();
                self.output.push(format!("Meshing failed: {message}"));
                // The geometry shows the faces and edges Gmsh names in red.
                if !self.mesh_failure.is_empty() {
                    self.output
                        .push("The affected faces and edges are marked red in the geometry".into());
                    self.tree.selected = None;
                    self.set_tree_view(TreeView::Geometry);
                }
            }
        }
    }

    /// PrePoMax's Model menu: create items of the FE model.
    /// PrePoMax's Interaction menu: constraints, contacts and the search for contact pairs.
    fn interaction_menu(&mut self, ui: &mut egui::Ui) {
        if self.setup_model().is_none() {
            ui.label("Open an .inp file first");
            return;
        }
        let mut kind = None;
        for (item, label) in [
            (NewItem::Constraint, "Create Constraint …"),
            (NewItem::SurfaceInteraction, "Create Surface Interaction …"),
            (NewItem::ContactPair, "Create Contact Pair …"),
            (NewItem::Tie, "Create Tie …"),
        ] {
            if ui.button(label).clicked() {
                kind = Some(item);
            }
        }
        if let Some(kind) = kind {
            self.create(kind);
        }
        ui.separator();
        if ui.button("Search Contact Pairs …").clicked() {
            self.open_contact_search();
        }
    }

    fn model_menu(&mut self, ui: &mut egui::Ui) {
        if self.setup_model().is_none() {
            ui.label("Create or open a model first");
            return;
        }
        if ui.button("Model Properties …").clicked() {
            self.edit_model_properties();
        }
        if ui.button("Edit CalculiX Keywords …").clicked() {
            self.open_keyword_editor();
        }
        ui.separator();
        let Some(model) = self.setup_model() else {
            return;
        };
        let last_step = model.fe.steps.len().checked_sub(1);
        let takes_loads = model
            .fe
            .steps
            .last()
            .is_some_and(|s| s.kind.supports_loads());
        let takes_defined_fields = model
            .fe
            .steps
            .last()
            .is_some_and(|s| s.kind.supports_defined_fields());
        let mut kind = None;
        for (item, label, enabled) in [
            (NewItem::Material, "Create Material …", true),
            (NewItem::Section, "Create Section …", true),
            (
                NewItem::InitialCondition,
                "Create Initial Condition …",
                true,
            ),
            (NewItem::Amplitude, "Create Amplitude …", true),
            (NewItem::Step, "Create Step …", true),
            (
                NewItem::BoundaryCondition(last_step.unwrap_or(0)),
                "Create Boundary Condition …",
                last_step.is_some(),
            ),
            (
                NewItem::Load(last_step.unwrap_or(0)),
                "Create Load …",
                takes_loads,
            ),
            (
                NewItem::FieldOutput(last_step.unwrap_or(0)),
                "Create Field Output …",
                last_step.is_some(),
            ),
            (
                NewItem::HistoryOutput(last_step.unwrap_or(0)),
                "Create History Output …",
                last_step.is_some(),
            ),
            (
                NewItem::DefinedField(last_step.unwrap_or(0)),
                "Create Defined Field …",
                takes_defined_fields,
            ),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                kind = Some(item);
            }
        }
        if let Some(kind) = kind {
            self.create(kind);
        }
    }

    /// PrePoMax's Model > Edit CalculiX Keywords.
    fn open_keyword_editor(&mut self) {
        let Some(model) = self.setup_model() else {
            return;
        };
        let heading = format!("PrePoLix: {}", model.file_name());
        match KeywordEditor::new(&model.mesh, &model.fe, &heading) {
            Ok(editor) => self.keyword_editor = Some(editor),
            Err(error) => self
                .output
                .push(format!("Keyword Editor not available: {error}")),
        }
    }

    /// PrePoMax's Material Library Editor, from the context menu of Materials.
    fn open_material_library(&mut self) {
        if let Some(model) = self.setup_model() {
            self.material_library = Some(MaterialLibraryEditor::new(
                &model.fe.materials,
                model.fe.properties.units,
            ));
        }
    }

    fn material_library_window(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.material_library else {
            return;
        };
        match editor.show(ctx) {
            LibraryResult::Open => {}
            LibraryResult::Ok(materials) => {
                self.material_library = None;
                if let Some(materials) = materials
                    && let Some(model) = self.model.as_mut()
                {
                    model.fe.materials = materials;
                    // Material indices may have changed.
                    self.tree.selected = None;
                    self.editor = None;
                }
            }
            LibraryResult::Cancel => self.material_library = None,
        }
    }

    fn keyword_editor_window(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.keyword_editor else {
            return;
        };
        match editor.show(ctx) {
            crate::keywords::EditorResult::Open => {}
            crate::keywords::EditorResult::Ok(keywords) => {
                self.keyword_editor = None;
                if let Some(model) = self.model.as_mut() {
                    model.fe.user_keywords = keywords;
                }
            }
            crate::keywords::EditorResult::Cancel => self.keyword_editor = None,
        }
    }

    fn analysis_menu(&mut self, ui: &mut egui::Ui) {
        let running = self.analysis.as_ref().is_some_and(Analysis::is_running);
        let current = self.current_analysis();
        let can_start = current.is_some() && !running;
        if ui
            .add_enabled(
                self.setup_model().is_some(),
                egui::Button::new("Create Analysis …"),
            )
            .clicked()
        {
            self.create(NewItem::Analysis);
        }
        // With several analyses the one selected in the tree runs.
        let name = (current.zip(self.setup_model()))
            .and_then(|(i, m)| m.fe.analyses.get(i))
            .map(|a| a.name.clone());
        let hint = "Runs the analysis selected in the tree, else the one run last.";
        let run_text = match &name {
            Some(name) => format!("Run Analysis: {name}"),
            None => "Run Analysis".into(),
        };
        if ui
            .add_enabled(can_start, egui::Button::new(run_text).shortcut_text("F5"))
            .on_hover_text(hint)
            .clicked()
        {
            self.start_analysis(current, false);
        }
        if ui
            .add_enabled(can_start, egui::Button::new("Check Model"))
            .on_hover_text(hint)
            .clicked()
        {
            self.start_analysis(current, true);
        }
        if ui
            .add_enabled(running, egui::Button::new("Kill Analysis"))
            .clicked()
            && let Some(analysis) = &mut self.analysis
        {
            analysis.kill();
        }
        if ui
            .add_enabled(self.analysis.is_some(), egui::Button::new("Monitor"))
            .clicked()
            && let Some(analysis) = &mut self.analysis
        {
            analysis.monitor = true;
        }
        let results = self.analysis.as_ref().and_then(Analysis::results);
        if ui
            .add_enabled(
                !running && results.is_some(),
                egui::Button::new("Open Results"),
            )
            .clicked()
        {
            self.open_analysis_results();
        }
        ui.separator();
        let rotor = self
            .setup_model()
            .is_some_and(|m| crate::campbell::applies(&m.fe));
        if ui
            .add_enabled(
                rotor || self.campbell.is_some(),
                egui::Button::new("Campbell diagram ..."),
            )
            .on_disabled_hover_text(
                "Needs an active Centrifugal load and a Complex Frequency step.",
            )
            .clicked()
        {
            // A closed window comes back with the diagram of its last sweep.
            if let Some(window) = &mut self.campbell {
                window.visible = true;
            } else if let Some(model) = self.setup_model() {
                self.campbell = Some(crate::campbell::CampbellWindow::new(&model.fe));
            }
        }
    }

    /// The Campbell diagram window with its sweep over the rotational speed.
    fn campbell_window(&mut self, ctx: &egui::Context) {
        let Some(mut window) = self.campbell.take() else {
            return;
        };
        if !window.visible {
            self.campbell = Some(window);
            return;
        }
        let can_start = self
            .setup_model()
            .is_some_and(|m| crate::campbell::applies(&m.fe) && m.mesh.element_count() > 0);
        let default_solver = self.settings.solver.default_solver();
        let open = window.show(ctx, can_start, |window| {
            let model = self
                .model
                .as_ref()
                .ok_or_else(|| "No model is open.".to_string())?;
            window.start(&self.settings.solver, model, default_solver)
        });
        window.visible = open || window.is_running();
        self.campbell = Some(window);
    }

    /// The analysis jobs the tree shows: the one that ran and the one with a bound result.
    fn job_states(&self) -> Vec<tree::JobState> {
        let Some(model) = self.model.as_ref() else {
            return Vec::new();
        };
        let analyses = &model.fe.analyses;
        let bound = model.bound_results.as_ref();
        let bound_to = bound.and_then(|b| analyses.position(&b.analysis));
        let mut jobs = Vec::new();
        let ran = (self.analysis.as_ref()).and_then(|a| Some((analyses.position(a.name())?, a)));
        if let Some((index, analysis)) = ran {
            jobs.push(tree::JobState {
                analysis: index,
                status: analysis.status(),
                results: analysis.results().is_some(),
                bound: bound_to == Some(index),
                ran: true,
            });
        }
        if let (Some(bound), Some(index)) = (bound, bound_to)
            && ran.is_none_or(|(ran, _)| ran != index)
        {
            jobs.push(tree::JobState {
                analysis: index,
                status: if bound.with_errors {
                    plx_job::JobStatus::FailedWithResults
                } else {
                    plx_job::JobStatus::Completed
                },
                results: true,
                bound: true,
                ran: false,
            });
        }
        jobs
    }

    /// The analysis the Analysis menu and F5 run: the one selected in the tree, else the
    /// one run last, else the first.
    fn current_analysis(&self) -> Option<usize> {
        let analyses = &self.setup_model()?.fe.analyses;
        if let Some((TreeView::FeModel, TreeItem::Analysis(index))) = &self.tree.selected
            && *index < analyses.len()
        {
            return Some(*index);
        }
        (self.analysis.as_ref())
            .and_then(|a| analyses.position(a.name()))
            .or_else(|| (!analyses.is_empty()).then_some(0))
    }

    /// Whether the last run is of the analysis `index`.
    fn ran(&self, index: usize) -> bool {
        let name = (self.setup_model()).and_then(|m| m.fe.analyses.get(index));
        (self.analysis.as_ref()).is_some_and(|a| name.is_some_and(|n| n.name == a.name()))
    }

    /// An entry of an analysis' context menu in the tree.
    fn analysis_action(&mut self, index: usize, action: AnalysisAction) {
        match action {
            AnalysisAction::Edit => {
                let running =
                    self.ran(index) && self.analysis.as_ref().is_some_and(Analysis::is_running);
                self.analysis_dialog = (self.setup_model()).and_then(|m| {
                    crate::analysis_dialog::AnalysisDialog::edit(&m.fe.analyses, index, running)
                });
            }
            AnalysisAction::Duplicate => {
                if let Some(model) = &mut self.model
                    && let Some(job) = model.fe.analyses.get(index)
                {
                    let mut copy = job.clone();
                    copy.name = model.fe.analyses.next_name();
                    let copy_index = model.fe.analyses.len();
                    model.fe.analyses.push(copy);
                    self.tree.selected = Some((TreeView::FeModel, TreeItem::Analysis(copy_index)));
                }
            }
            AnalysisAction::Run => self.start_analysis(Some(index), false),
            AnalysisAction::CheckModel => self.start_analysis(Some(index), true),
            AnalysisAction::Monitor => match (self.ran(index), &mut self.analysis) {
                (true, Some(analysis)) => analysis.monitor = true,
                _ => self.output.push(
                    "The analysis has not been started yet (Analysis > Run Analysis).".into(),
                ),
            },
            AnalysisAction::Results => {
                let ran = self.ran(index);
                match self.analysis.as_ref().and_then(Analysis::results) {
                    Some(_) if ran => self.open_analysis_results(),
                    _ => self.open_bound_results(),
                }
            }
            AnalysisAction::RemoveBoundResult => {
                if let Some(model) = &mut self.model
                    && model.bound_results.take().is_some()
                {
                    model.bound_results_file = None;
                    self.output.push(
                        "Bound result removed; saving the project leaves it out of the file."
                            .into(),
                    );
                }
            }
            AnalysisAction::Kill => {
                if self.ran(index)
                    && let Some(analysis) = &mut self.analysis
                {
                    analysis.kill();
                }
            }
        }
    }

    /// The dialog creating or editing an analysis.
    fn analysis_dialog_window(&mut self, ctx: &egui::Context) {
        use crate::analysis_dialog::AnalysisDialogResult;
        let (Some(dialog), Some(model)) = (&mut self.analysis_dialog, &mut self.model) else {
            self.analysis_dialog = None;
            return;
        };
        match dialog.show(ctx, &model.fe.analyses, &self.settings.solver) {
            AnalysisDialogResult::Open => {}
            AnalysisDialogResult::Cancel => self.analysis_dialog = None,
            AnalysisDialogResult::Settings => {
                let window = SettingsWindow::with_page(&self.settings, settings::Page::Solver);
                self.settings_window = Some(window);
            }
            AnalysisDialogResult::Ok => {
                let draft = dialog.draft.clone();
                match dialog.index {
                    None => {
                        let index = model.fe.analyses.len();
                        model.fe.analyses.push(draft);
                        self.tree.selected = Some((TreeView::FeModel, TreeItem::Analysis(index)));
                    }
                    Some(index) => {
                        let Some(job) = model.fe.analyses.get_mut(index) else {
                            self.analysis_dialog = None;
                            return;
                        };
                        let old = std::mem::replace(job, draft);
                        let new = &model.fe.analyses[index].name;
                        // The run and the bound result stay with the renamed analysis.
                        if old.name != *new {
                            if let Some(bound) = &mut model.bound_results
                                && bound.analysis == old.name
                            {
                                bound.analysis = new.clone();
                            }
                            if let Some(analysis) = &mut self.analysis
                                && analysis.name() == old.name
                            {
                                analysis.rename(new.clone());
                            }
                        }
                    }
                }
                self.analysis_dialog = None;
            }
        }
    }

    /// Deletes the analysis `index` with its bound result; a running one stays.
    fn delete_analysis(&mut self, index: usize) {
        let running = self.ran(index) && self.analysis.as_ref().is_some_and(Analysis::is_running);
        let Some(model) = self.model.as_mut() else {
            return;
        };
        if running || index >= model.fe.analyses.len() {
            return;
        }
        let job = model.fe.analyses.remove(index);
        if model
            .bound_results
            .as_ref()
            .is_some_and(|b| b.analysis == job.name)
        {
            model.bound_results = None;
            model.bound_results_file = None;
        }
        self.tree.selected = None;
        self.analysis_dialog = None;
    }

    /// Opens the results of the last run and binds them to the project, once per run.
    fn open_analysis_results(&mut self) {
        let Some(path) = self.analysis.as_ref().and_then(Analysis::results) else {
            return;
        };
        // Results of a new run take over what was defined on those of the run before.
        if self.analysis.as_ref().is_some_and(|a| !a.bound) {
            self.carried_settings = self.previous_settings(&path).map(|s| (path.clone(), s));
        }
        self.open_results = Some(path);
        self.bind_results();
    }

    /// What was defined on the results before those at `path` came: on the open bound
    /// results, on open results at that path, or saved with the bound results.
    fn previous_settings(&self, path: &std::path::Path) -> Option<plx_results::ResultsSettings> {
        let model = self.model.as_ref()?;
        let open = |file: &std::path::Path| self.results.iter().find(|r| r.path == file);
        (model.bound_results_file.as_deref().and_then(open))
            .or_else(|| open(path))
            .and_then(|results| self.view_settings(results))
            .or_else(|| model.bound_results.as_ref()?.settings.clone())
    }

    /// What the user defined on open results, with the camera of the results workspace.
    fn view_settings(&self, results: &Model) -> Option<plx_results::ResultsSettings> {
        let mut settings = results.results_settings()?;
        // The results workspace's camera, in use or parked while another is shown.
        let camera = match self.tree_view {
            TreeView::Results => Some(self.viewport.camera()),
            _ => self.parked_camera.as_ref(),
        };
        settings.view.camera = camera.map(|c| plx_results::saved_view::SavedCamera {
            target: c.target.to_array(),
            distance: c.distance,
            rotation: c.rotation.to_array(),
            scene_radius: c.scene_radius,
        });
        settings.view.mesh_edges = Some(self.viewport.options.mesh_edges);
        Some(settings)
    }

    /// Binds the results of the finished analysis to the project: its input, results and
    /// history output files are saved with the project from now on.
    fn bind_results(&mut self) {
        if self.bound_work.is_some() {
            return;
        }
        let Some(analysis) = (self.analysis.as_mut()).filter(|a| !a.is_running() && !a.bound)
        else {
            return;
        };
        let Some(frd) = analysis.results() else {
            return;
        };
        // A result removed from the project is not bound again until the next run.
        analysis.bound = true;
        let with_errors = analysis.status() == plx_job::JobStatus::FailedWithResults;
        let name = analysis.name().to_owned();
        let Some(model) = &self.model else {
            return;
        };
        // Compressing large results takes seconds: done beside the GUI.
        let (sender, receiver) = std::sync::mpsc::channel();
        let files = ["inp", "frd", "dat"].map(|e| frd.with_extension(e));
        let job_name = name.clone();
        std::thread::spawn(move || {
            let bound = plx_io::project::bind_results(&job_name, &files, with_errors)
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = sender.send(BoundWork::Bound(frd, bound));
        });
        self.output.push(format!("Binding the result of {name} …"));
        self.bound_work = Some((model.path.clone(), receiver));
    }

    /// Opens the results bound to the project, written into the work directory first.
    fn open_bound_results(&mut self) {
        if self.bound_work.is_some() {
            return;
        }
        let work_dir = self.settings.solver.work_dir();
        let Some(model) = &self.model else {
            return;
        };
        let Some(bound) = &model.bound_results else {
            return;
        };
        if let Some(path) = model.bound_results_file.as_ref().filter(|p| p.is_file()) {
            self.open_results = Some(path.clone());
            return;
        }
        let stem = model
            .path
            .file_stem()
            .map_or_else(|| "Project".into(), |s| s.to_string_lossy().into_owned());
        let dir = work_dir.join("Bound_Results").join(stem);
        let (sender, receiver) = std::sync::mpsc::channel();
        let bound = bound.clone();
        std::thread::spawn(move || {
            let restored =
                plx_io::project::restore_results(&bound, &dir).map_err(|e| e.to_string());
            let _ = sender.send(BoundWork::Restored(bound.analysis, dir, restored));
        });
        self.bound_work = Some((model.path.clone(), receiver));
    }

    /// Takes the result of binding or restoring bound results once the worker is done.
    fn poll_bound_results(&mut self, ctx: &egui::Context) {
        let Some((path, receiver)) = &self.bound_work else {
            return;
        };
        let work = match receiver.try_recv() {
            Ok(work) => work,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.bound_work = None;
                return;
            }
        };
        // The model may have been closed or replaced meanwhile.
        let model = (self.model.as_mut()).filter(|m| m.path == *path);
        self.bound_work = None;
        match (work, model) {
            (BoundWork::Bound(frd, Ok(bound)), Some(model)) => {
                self.output.push(format!(
                    "Result bound to {}: {} of files, {} in the project file once it is saved",
                    bound.analysis,
                    file_size(bound.size()),
                    file_size(bound.stored_size())
                ));
                model.bound_results = Some(*bound);
                // The open results whose settings are saved with them.
                model.bound_results_file = Some(frd);
            }
            (BoundWork::Restored(analysis, dir, Ok(file)), Some(model)) => {
                self.output.push(format!(
                    "Result bound to {analysis} restored to {}",
                    dir.display()
                ));
                model.bound_results_file = Some(file.clone());
                self.open_results = Some(file);
            }
            (BoundWork::Bound(_, Err(error)), _) => {
                self.output.push(format!("Result not bound: {error}"));
            }
            (BoundWork::Restored(_, _, Err(error)), _) => {
                self.output
                    .push(format!("Bound result not restored: {error}"));
            }
            (_, None) => {}
        }
    }

    /// Runs the analysis `index` of the model, or only checks the model.
    fn start_analysis(&mut self, index: Option<usize>, check_model: bool) {
        if self.analysis.as_ref().is_some_and(Analysis::is_running) {
            return;
        }
        let Some(model) = self.setup_model() else {
            return;
        };
        let Some(job) = index.and_then(|i| model.fe.analyses.get(i)).cloned() else {
            self.output.push(
                "The model has no analysis: double-click Analyses in the tree to create one."
                    .into(),
            );
            return;
        };
        let default_solver = self.settings.solver.default_solver();
        let Some(model) = self.setup_model() else {
            return;
        };
        if model.mesh.element_count() == 0 {
            self.output
                .push("The model has no mesh yet: Mesh > Mesh All Parts".into());
            return;
        }
        let findings = model.findings();
        match Analysis::start(
            &self.settings.solver,
            model,
            &job,
            default_solver,
            check_model,
        ) {
            Ok(mut analysis) => {
                // Problems the checks found go into the monitor first, so that an abort is
                // explained even before CalculiX says anything.
                for finding in
                    (findings.iter()).filter(|f| f.severity() == plx_model::Severity::Error)
                {
                    analysis.note(format!(
                        "Model check: {}: {}",
                        finding.problem.title(),
                        finding.detail
                    ));
                }
                self.output.push(format!(
                    "{} {} started: {}",
                    if check_model {
                        "Model check of"
                    } else {
                        "Analysis"
                    },
                    job.name,
                    self.settings.solver.work_dir().display()
                ));
                self.solver_findings.clear();
                // CalculiX skips keywords it does not know with only a warning; tested on
                // the executable while it already runs.
                let unknown = self.settings.solver.unknown_keywords(analysis.keywords());
                if !unknown.is_empty() {
                    let finding = plx_model::Finding::unknown_keywords(format!(
                        "CalculiX {}does not know {}",
                        (self.settings.solver.keywords.as_ref())
                            .and_then(|k| k.version.as_deref())
                            .map_or_else(String::new, |v| format!("{v} ")),
                        crate::solver_check::by_feature(&unknown).join("; ")
                    ));
                    let note = format!(
                        "Model check: {}: {}",
                        finding.problem.title(),
                        finding.detail
                    );
                    analysis.note(note.clone());
                    self.output.push(note);
                    self.solver_findings.push(finding);
                }
                self.analysis = Some(analysis);
            }
            Err(error) => self.output.push(error),
        }
    }

    /// Polls the running analysis and shows its monitor.
    fn run_analysis(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.start_analysis(self.current_analysis(), false);
        }
        let Some(analysis) = &mut self.analysis else {
            return;
        };
        if let Some(status) = analysis.poll() {
            self.output
                .push(format!("Analysis {}", Analysis::status_text(status)));
            if matches!(
                status,
                plx_job::JobStatus::Failed | plx_job::JobStatus::FailedWithResults
            ) && let Some(model) = self.model.as_ref()
            {
                self.solver_findings =
                    plx_model::diagnose_solver_output(analysis.output(), &model.mesh);
                for finding in (self.solver_findings.iter())
                    .filter(|f| f.item == plx_model::ModelItem::Analysis)
                {
                    analysis.note(format!(
                        "Possible cause: {}. {}",
                        finding.problem.title(),
                        finding.problem.fix()
                    ));
                }
                if !self.solver_findings.is_empty() {
                    analysis.note(
                        "The warning symbol on the analysis in the tree explains the cause.".into(),
                    );
                }
            }
        }
        if analysis.monitor
            && let MonitorEvent::OpenResults = analysis.window(ctx)
        {
            self.open_analysis_results();
        }
    }

    /// The project file of the open model, if it was opened from or saved to one that exists.
    /// Unsaved models and models from input or result files are not reopened at the next start.
    fn last_project(&self) -> Option<PathBuf> {
        self.model
            .as_ref()
            .filter(|model| model.is_project() && model.path.is_file())
            .map(|model| model.path.clone())
    }

    /// Saves mesh and FE model as a project, to the project file it came from, or to a file
    /// the user picks for a model opened from an input file or with `save_as`.
    fn save_project(&mut self, save_as: bool) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let path = if model.is_project() && !save_as {
            model.path.clone()
        } else {
            let Some(path) = pick_project_file(model, "Save Project", "") else {
                return;
            };
            path
        };
        // The bound results keep what the user defined on them, if they are open.
        let bound_file = model.bound_results_file.clone();
        let settings = (self.results.iter())
            .find(|r| bound_file.as_ref() == Some(&r.path))
            .map(|open| self.view_settings(open));
        let Some(model) = self.model.as_mut() else {
            return;
        };
        if let Some(bound) = &mut model.bound_results
            && let Some(settings) = settings
        {
            bound.settings = settings;
        }
        let geometry = model.geometry.as_ref();
        let results = model.bound_results.as_ref();
        match plx_io::project::save_project(&path, geometry, &model.mesh, &model.fe, results) {
            Ok(()) => {
                self.output
                    .push(timestamped(format!("{} saved", path.display())));
                model.path = path;
            }
            Err(error) => self.output.push(format!("Not saved: {error}")),
        }
    }

    /// Saves a copy of the project without the bound results, only mesh and definition, for
    /// example to send it; the open project stays on its own file and keeps its results.
    fn save_without_results(&mut self) {
        let Some(model) = self.model.as_ref() else {
            return;
        };
        let Some(path) = pick_project_file(model, "Save Without Results", "-without-results")
        else {
            return;
        };
        let geometry = model.geometry.as_ref();
        match plx_io::project::save_project(&path, geometry, &model.mesh, &model.fe, None) {
            Ok(()) => self.output.push(timestamped(format!(
                "{} saved without results",
                path.display()
            ))),
            Err(error) => self.output.push(format!("Not saved: {error}")),
        }
    }

    /// Writes the input file of the set-up model to a file the user picks.
    fn export_inp(&mut self) {
        if self.setup_model().is_none() {
            return;
        }
        let default_solver = self.settings.solver.default_solver();
        let Some(model) = self.setup_model() else {
            return;
        };
        let heading = format!("PrePoLix: {}", model.file_name());
        let name = (self.current_analysis())
            .and_then(|i| model.fe.analyses.get(i))
            .map_or_else(|| plx_model::FIRST_ANALYSIS.to_owned(), |a| a.name.clone());
        let mut fe = model.fe.clone();
        fe.resolve_default_solver(default_solver);
        // Checked before asking for the file; the global results are staged once it is known.
        if let Err(error) = plx_io::inp::write_inp(&model.mesh, &fe, &heading) {
            self.output.push(format!("Export not possible: {error}"));
            return;
        }
        let picked = rfd::FileDialog::new()
            .set_title("Export CalculiX Input File")
            .add_filter("Input file (*.inp)", &["inp"])
            .set_file_name(format!("{name}.inp"))
            .save_file();
        if let Some(path) = picked {
            // A submodel's global results go next to the input file, which names them.
            let dir = path.parent().unwrap_or(std::path::Path::new("."));
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
            if let Err(error) = crate::analysis::stage_global_results(
                &mut fe,
                dir,
                stem.as_deref().unwrap_or("Analysis"),
            ) {
                self.output.push(error);
                return;
            }
            let text = match plx_io::inp::write_inp(&model.mesh, &fe, &heading) {
                Ok(text) => text,
                Err(error) => {
                    self.output.push(format!("Export not possible: {error}"));
                    return;
                }
            };
            if fe.uses_global_results()
                && let Some(global) = fe.properties.submodel_input()
            {
                self.output.push(format!("{} copied", global.display()));
            }
            match std::fs::write(&path, text) {
                Ok(()) => self.output.push(format!("{} written", path.display())),
                Err(error) => self.output.push(format!("{}: {error}", path.display())),
            }
            // CalculiX reads the result files of defined fields from next to the input file.
            if let Some(dir) = path.parent() {
                let notes = crate::analysis::copy_result_files(&fe, dir);
                self.output.extend(notes);
            }
        }
    }

    /// PrePoMax's "Export deformed mesh": the mesh of the current results with the shown
    /// displacements times the deformation scale factor, without materials or steps, so
    /// that another model can import it.
    fn export_deformed_mesh(&mut self) {
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        let Some(view) = &model.results else {
            return;
        };
        let Some(displacements) = view.shown_displacements() else {
            self.output
                .push("Export not possible: the results have no displacements".into());
            return;
        };
        let scale = f64::from(view.scale());
        let shown = view
            .current_increment()
            .map(|i| format!(", step {}, increment {}", i.step, i.increment))
            .unwrap_or_default();
        let heading = format!(
            "PrePoLix: deformed mesh of {}{shown}, scale factor {scale}",
            model.file_name()
        );
        let text = match plx_io::inp::write_deformed_mesh_inp(
            &model.mesh,
            &displacements,
            scale,
            &heading,
        ) {
            Ok(text) => text,
            Err(error) => {
                self.output.push(format!("Export not possible: {error}"));
                return;
            }
        };
        let stem = model
            .path
            .file_stem()
            .map_or("Results".into(), |s| s.to_string_lossy().into_owned());
        let picked = rfd::FileDialog::new()
            .set_title("Export Deformed Mesh")
            .add_filter("Input file (*.inp)", &["inp"])
            .set_file_name(format!("{stem}_deformed.inp"))
            .save_file();
        if let Some(path) = picked {
            match std::fs::write(&path, text) {
                Ok(()) => self
                    .output
                    .push(format!("Deformed mesh exported to {}", path.display())),
                Err(error) => self.output.push(format!("{}: {error}", path.display())),
            }
        }
    }

    /// Opens the Export VTU or Export VTK dialog of the current results.
    fn open_vtk_export(&mut self, format: plx_io::vtk::VtkFormat) {
        if self.shown_results_view().is_some() {
            self.vtk_export_dialog = Some(crate::vtk_export_dialog::VtkExportDialog::new(
                self.current_result,
                format,
            ));
        }
    }

    /// The Export VTU or Export VTK dialog; Export asks for the file and writes it.
    fn vtk_export_window(&mut self, ctx: &egui::Context) {
        use crate::vtk_export_dialog::DialogResult;
        let Some(mut dialog) = self.vtk_export_dialog.take() else {
            return;
        };
        let Some(model) = self.results.get(dialog.result) else {
            return;
        };
        let Some(view) = &model.results else {
            return;
        };
        match dialog.show(ctx, &view.increments, view.increment) {
            DialogResult::Open => self.vtk_export_dialog = Some(dialog),
            DialogResult::Cancel => {}
            DialogResult::Export => {
                let increments = dialog.scope.select(&view.increments, view.increment);
                let format = dialog.format;
                let several = increments.len() > 1;
                let extension = if several {
                    format.collection_extension()
                } else {
                    format.extension()
                };
                let stem = model
                    .path
                    .file_stem()
                    .map_or("Results".into(), |s| s.to_string_lossy().into_owned());
                let filter = match (several, format) {
                    (false, _) => format!("VTK file (*.{extension})"),
                    (true, plx_io::vtk::VtkFormat::Xml) => "ParaView Data (*.pvd)".into(),
                    (true, plx_io::vtk::VtkFormat::Legacy) => {
                        "VTK file series (*.vtk.series)".into()
                    }
                };
                let filter_extension = extension.rsplit('.').next().unwrap_or(extension);
                let picked = rfd::FileDialog::new()
                    .set_title(dialog.title())
                    .add_filter(filter, &[filter_extension])
                    .set_file_name(format!("{stem}.{extension}"))
                    .save_file();
                let Some(mut path) = picked else {
                    // Back to the dialog to choose again.
                    self.vtk_export_dialog = Some(dialog);
                    return;
                };
                let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase());
                if !name.is_some_and(|n| n.ends_with(&format!(".{extension}"))) {
                    let mut with = path.into_os_string();
                    with.push(format!(".{extension}"));
                    path = with.into();
                }
                let title = format!("PrePoLix results of {}", model.file_name());
                let result = plx_io::vtk::export(
                    &path,
                    format,
                    &model.mesh,
                    &increments,
                    dialog.binary,
                    &title,
                );
                match result {
                    Ok(summary) => {
                        let mut line = format!("Results exported to {}", summary.main.display());
                        if summary.files > 1 {
                            line.push_str(&format!(" ({} increments", summary.files));
                            if summary.physical_times == Some(false) {
                                line.push_str(
                                    "; their values do not increase, so the time series is \
                                     numbered 0, 1, 2, ...",
                                );
                            }
                            line.push(')');
                        }
                        self.output.push(line);
                    }
                    Err(error) => self
                        .output
                        .push(format!("Export not possible: {}: {error}", path.display())),
                }
            }
        }
    }

    /// A click in the 3D view picks for the open dialog. Without one, a click on a part
    /// selects it in the tree, as in PrePoMax, and a click into empty space clears the tree
    /// selection and with it the highlighted region.
    fn click(&mut self, click: Click) {
        if let Some(query) = &mut self.query {
            let model = match self.tree_view {
                TreeView::Results => self.results.get(self.current_result),
                TreeView::Geometry => self.geometry.as_ref(),
                TreeView::FeModel => self.model.as_ref(),
            };
            if let Some(model) = model {
                let hit = model.pick_click(&click);
                let precision = hit.as_ref().map_or(0.0, |h| click.precision_at(h.point));
                match query.click(model, hit.as_ref(), precision, &mut self.output) {
                    QueryClick::Nothing => {}
                    QueryClick::Picked => self.update_contour(),
                    QueryClick::Added(annotation) => {
                        if let Some(model) = self.shown_mut() {
                            model.query_annotations.push(annotation);
                        }
                        self.update_contour();
                    }
                }
            }
            return;
        }
        if self.feature_picks()
            && let Some(dialog) = &mut self.feature_dialog
        {
            let model = match dialog.results {
                Some(index) => self.results.get(index),
                None => self.model.as_ref(),
            };
            if let Some(model) = model {
                let hit = model.pick(click.origin, click.direction);
                dialog.click(
                    model,
                    hit.as_ref().map(|h| (h, click.precision_at(h.point))),
                    Operation::from_modifiers(click.shift, click.ctrl),
                );
            }
            return;
        }
        if self.transformation_picks()
            && let (Some(dialog), Some(model)) = (
                &mut self.transformation_dialog,
                self.results.get(self.current_result),
            )
        {
            let hit = model.pick_click(&click);
            dialog.click(
                model,
                hit.as_ref().map(|h| (h, click.precision_at(h.point))),
            );
            return;
        }
        if let Some(dialog) = &mut self.section_dialog
            && dialog.picks()
        {
            let model = match self.tree_view {
                TreeView::Results => self.results.get(self.current_result),
                TreeView::Geometry => self.geometry.as_ref(),
                TreeView::FeModel => self.model.as_ref(),
            };
            if let Some(model) = model {
                let hit = model.pick_click(&click);
                dialog.click(
                    model,
                    hit.as_ref().map(|h| (h, click.precision_at(h.point))),
                );
            }
            return;
        }
        if !self.picking() {
            let hit = self.shown().and_then(|model| model.pick_click(&click));
            let view = self.tree_view;
            match hit {
                // Ctrl adds a part or takes it out again, Shift adds it.
                Some(hit) if click.ctrl => self.tree.toggle_part(view, hit.part),
                Some(hit) if click.shift => self.tree.add_parts(view, [hit.part]),
                Some(hit) => self.select_part(hit.part),
                // A click beside the model with Ctrl or Shift keeps the selection.
                None if click.ctrl || click.shift => {}
                // On the Results tab the tree shows the current field, which stays.
                None if self.tree_view != TreeView::Results => self.tree.selected = None,
                None if matches!(self.tree.selected, Some((_, TreeItem::Part(_)))) => {
                    self.tree.selected = None;
                }
                None => {}
            }
            return;
        }
        if self.tree_view == TreeView::Results {
            if let Some(model) = self.results.get(self.current_result) {
                let hit = model.pick_click(&click);
                let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
                let operation = Operation::from_modifiers(click.shift, click.ctrl);
                if let Some(dialog) = &mut self.hot_spot_dialog {
                    dialog.click(model, pick, operation);
                } else if let Some(dialog) = &mut self.history_dialog {
                    dialog.click(model, pick, operation);
                }
            }
            return;
        }
        if self.tree_view == TreeView::Geometry {
            if let (Some(editor), Some(view)) = (&mut self.mesh_item_editor, &self.geometry) {
                let hit = view.pick_click(&click);
                let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
                editor.click(
                    view,
                    pick,
                    Operation::from_modifiers(click.shift, click.ctrl),
                );
            }
            return;
        }
        let (Some(editor), Some(model)) = (&mut self.editor, &self.model) else {
            return;
        };
        let hit = model.pick_click(&click);
        let pick = hit.as_ref().map(|hit| (hit, click.precision_at(hit.point)));
        editor.click(
            model,
            pick,
            Operation::from_modifiers(click.shift, click.ctrl),
        );
    }

    /// Selects a part clicked in the 3D view in the tree shown.
    fn select_part(&mut self, index: usize) {
        let view = self.tree_view;
        self.tree.select_part(view, index);
        self.tree.reveal = true;
    }

    /// The context menu of the 3D view: on a part it starts with the part's menu from the
    /// tree, so it makes no difference where the part is right-clicked.
    fn viewport_menu(&mut self, response: &egui::Response, right_click: Option<Click>) {
        if let Some(click) = right_click {
            self.menu_part = None;
            if !self.picking() {
                self.menu_part = (self.shown())
                    .and_then(|model| model.pick_click(&click))
                    .map(|hit| hit.part);
            }
            // A right click on one of several selected parts keeps them, as in the tree.
            if let Some(part) = self.menu_part
                && !self.tree.selected_parts(self.tree_view).contains(&part)
            {
                self.select_part(part);
            }
        }
        let part = (self.menu_part).and_then(|index| {
            let info = self.shown()?.parts.get(index)?;
            Some((index, info.visible, info.contours))
        });
        let mut tree_response = TreeResponse::default();
        let mut command = None;
        let selected = self.tree.selected_parts(self.tree_view);
        response.context_menu(|ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if let Some((index, visible, contours)) = part {
                let view = self.tree_view;
                tree::part_menu(
                    ui,
                    index,
                    visible,
                    contours,
                    view,
                    &selected,
                    &mut tree_response,
                );
                ui.separator();
            }
            command = crate::viewport::view_menu(ui);
        });
        if command.is_some() {
            self.view_command = command;
        }
        let view = self.tree_view;
        self.tree_response(&response.ctx, view, tree_response);
    }

    fn box_select(&mut self, area: &BoxSelect) {
        if !self.picking() || self.section_picks() {
            return;
        }
        let operation = Operation::from_modifiers(area.shift, area.ctrl);
        if self.feature_picks()
            && let Some(dialog) = &mut self.feature_dialog
        {
            let model = match dialog.results {
                Some(index) => self.results.get(index),
                None => self.model.as_ref(),
            };
            if let Some(model) = model {
                dialog.box_select(model, area, operation);
            }
            return;
        }
        if self.tree_view == TreeView::Results {
            if let Some(model) = self.results.get(self.current_result) {
                if let Some(dialog) = &mut self.hot_spot_dialog {
                    dialog.box_select(model, area, operation);
                } else if let Some(dialog) = &mut self.history_dialog {
                    dialog.box_select(model, area, operation);
                }
            }
            return;
        }
        if self.tree_view == TreeView::Geometry {
            if let (Some(editor), Some(view)) = (&mut self.mesh_item_editor, &self.geometry) {
                editor.box_select(view, area, operation);
            }
            return;
        }
        if let (Some(editor), Some(model)) = (&mut self.editor, &self.model) {
            editor.box_select(
                model,
                area,
                Operation::from_modifiers(area.shift, area.ctrl),
            );
        }
    }

    /// Shows what a click would select where the mouse rests.
    fn hover(&mut self, hover: Option<Click>) {
        if let Some(query) = &self.query {
            self.viewport.preview = match (hover, self.shown()) {
                (Some(click), Some(model)) => model
                    .pick_click(&click)
                    .map(|hit| query.preview(model, &hit, click.precision_at(hit.point)))
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        if self.feature_picks()
            && let Some(dialog) = &self.feature_dialog
            && let Some(model) = self.feature_model(dialog.results)
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick(click.origin, click.direction)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        if self.transformation_picks()
            && let (Some(dialog), Some(model)) = (
                &self.transformation_dialog,
                self.results.get(self.current_result),
            )
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick_click(&click)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        if let (Some(dialog), Some(model)) = (&self.section_dialog, self.shown())
            && dialog.picks()
        {
            self.viewport.preview = hover
                .and_then(|click| {
                    let hit = model.pick_click(&click)?;
                    Some(dialog.preview(model, &hit, click.precision_at(hit.point)))
                })
                .unwrap_or_default();
            return;
        }
        let hover = hover.filter(|_| self.picking());
        if self.tree_view == TreeView::Results {
            let model = self.results.get(self.current_result);
            self.viewport.preview = match (hover, model) {
                (Some(click), Some(model)) => model
                    .pick_click(&click)
                    .map(|hit| {
                        let precision = click.precision_at(hit.point);
                        match (&self.hot_spot_dialog, &self.history_dialog) {
                            (Some(dialog), _) => dialog.preview(model, &hit, precision),
                            (None, Some(dialog)) => dialog.preview(model, &hit, precision),
                            (None, None) => Default::default(),
                        }
                    })
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        if self.tree_view == TreeView::Geometry {
            let shown = (self.mesh_item_editor.as_ref()).zip(self.geometry.as_ref());
            self.viewport.preview = match (hover, shown) {
                (Some(click), Some((editor, view))) => view
                    .pick_click(&click)
                    .map(|hit| editor.preview(view, &hit, click.precision_at(hit.point)))
                    .unwrap_or_default(),
                _ => Default::default(),
            };
            return;
        }
        let preview = match (hover, &self.editor, &self.model) {
            (Some(click), Some(editor), Some(model)) => model
                .pick_click(&click)
                .map(|hit| editor.preview(model, &hit, click.precision_at(hit.point)))
                .unwrap_or_default(),
            _ => Default::default(),
        };
        self.viewport.preview = preview;
    }

    /// Opens the dialog of the derived field output that computes the given field.
    fn edit_field_output(&mut self, field: usize) {
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        let Some(view) = &model.results else { return };
        if let Some(index) = view.field_output_index(field) {
            self.field_output_dialog = FieldOutputDialog::edit(view, &model.mesh, index);
        }
    }

    fn delete_field_output(&mut self, field: usize) {
        let Some(model) = self.results.get_mut(self.current_result) else {
            return;
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let Some(index) = view.field_output_index(field) else {
            return;
        };
        let name = view.field_outputs[index].name.clone();
        let warnings = view.remove_field_output(index, &model.mesh);
        self.output.push(format!("Field output {name} deleted"));
        for warning in warnings {
            self.output.push(format!("Warning: {warning}"));
        }
        self.tree.selected = None;
        self.field_output_dialog = None;
        self.close_history_windows();
        self.results_changed = true;
    }

    fn field_output_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.field_output_dialog else {
            return;
        };
        let (output, next) = match dialog.show(ctx) {
            DialogAction::Open => return,
            DialogAction::Cancel => {
                self.field_output_dialog = None;
                return;
            }
            DialogAction::Ok { output, next } => (output, next),
        };
        let Some(model) = self.results.get_mut(self.current_result) else {
            self.field_output_dialog = None;
            return;
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let name = output.name.clone();
        let edit = dialog.edit;
        match view.set_field_output(edit, output, &model.mesh) {
            Ok(warnings) => {
                let verb = if edit.is_some() {
                    "modified"
                } else {
                    "created"
                };
                self.output.push(format!("Field output {name} {verb}"));
                for warning in warnings {
                    self.output.push(format!("Warning: {warning}"));
                }
                // PrePoMax shows a new field output right away.
                if edit.is_none()
                    && let Some(field) = (view.current_increment())
                        .and_then(|i| i.fields.iter().position(|f| f.name == name))
                {
                    view.field = field;
                    view.component = 0;
                    self.tree.selected = Some((TreeView::Results, TreeItem::Component(field, 0)));
                }
                self.results_changed = true;
                if next {
                    dialog.next(&name);
                } else {
                    self.field_output_dialog = None;
                }
            }
            Err(error) => dialog.error = Some(error),
        }
    }

    fn edit_history_output(&mut self, set: usize) {
        self.refresh_moving_loads();
        let Some(model) = self.results.get(self.current_result) else {
            return;
        };
        let Some(view) = &model.results else { return };
        if let Some(index) = view.history_output_index(set) {
            self.history_dialog = HistoryOutputDialog::edit(view, &model.mesh, index);
        }
    }

    fn delete_history_output(&mut self, set: usize) {
        let Some(model) = self.results.get_mut(self.current_result) else {
            return;
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let Some(index) = view.history_output_index(set) else {
            return;
        };
        let name = view.history_outputs[index].name.clone();
        let warnings = view.remove_history_output(index, &model.mesh);
        self.output.push(format!("History output {name} deleted"));
        for warning in warnings {
            self.output.push(format!("Warning: {warning}"));
        }
        self.tree.selected = None;
        self.close_history_windows();
    }

    fn history_output_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (
            &mut self.history_dialog,
            self.results.get_mut(self.current_result),
        ) else {
            return;
        };
        let (output, next) = match dialog.show(ctx, model) {
            DialogAction::Open => return,
            DialogAction::Cancel => {
                self.history_dialog = None;
                return;
            }
            DialogAction::Ok { output, next } => (output, next),
        };
        let Some(view) = &mut model.results else {
            return;
        };
        let edit = dialog.edit;
        match view.set_history_output(edit, output.clone(), &model.mesh) {
            Ok(warnings) => {
                let verb = if edit.is_some() {
                    "modified"
                } else {
                    "created"
                };
                self.output
                    .push(format!("History output {} {verb}", output.name));
                for warning in warnings {
                    self.output.push(format!("Warning: {warning}"));
                }
                // The table may show another component now.
                self.history_table = None;
                if next {
                    dialog.next(&output, view);
                } else {
                    self.history_dialog = None;
                }
            }
            Err(error) => dialog.error = Some(error),
        }
    }

    fn history_table_window(&mut self, ctx: &egui::Context) {
        if let Some(data) = &self.history_plot {
            let model = match self.tree_view {
                TreeView::Results => self.results.get_mut(self.current_result),
                _ => None,
            };
            let shown = model
                .as_ref()
                .is_some_and(|m| m.path_plots.iter().any(|p| p.is_table(data)));
            let response = crate::xy_plot::window(ctx, data, shown);
            // The annotation belongs to the shown results and stays after the window closes.
            if response.toggle_view
                && let Some(model) = model
            {
                match model.path_plots.iter().position(|p| p.is_table(data)) {
                    Some(index) => {
                        model.path_plots.remove(index);
                    }
                    None => model.path_plots.push(PathPlot::table(data.clone())),
                }
            }
            match response.export {
                Some(crate::xy_plot::Export::Clipboard) => {
                    (self.screenshot).request_region(screenshot::Target::Clipboard, response.plot)
                }
                Some(crate::xy_plot::Export::Png) => {
                    (self.screenshot).request_region(screenshot::Target::File, response.plot)
                }
                Some(crate::xy_plot::Export::Svg) => {
                    let svg =
                        crate::xy_plot::to_svg(data, response.plot.width(), response.plot.height());
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Save Plot As")
                        .add_filter("SVG image", &["svg"])
                        .set_file_name("plot.svg")
                        .save_file()
                    {
                        let path = if path.extension().is_none() {
                            path.with_extension("svg")
                        } else {
                            path
                        };
                        self.output.push(match std::fs::write(&path, svg) {
                            Ok(()) => format!("Plot saved: {}", path.display()),
                            Err(err) => {
                                format!("Plot could not be saved ({}): {err}", path.display())
                            }
                        });
                    }
                }
                None => {}
            }
            if response.closed {
                self.history_plot = None;
            }
        }
        let Some(mut table) = self.history_table.take() else {
            return;
        };
        let Some(view) = self.shown_results_view() else {
            return;
        };
        let kind = view
            .current_increment()
            .map_or(plx_results::AnalysisKind::Static, |i| i.kind);
        let unit = (view.history_output_index(table.set))
            .and_then(|i| view.history_outputs.get(i))
            .and_then(|o| o.unit());
        match table.show(ctx, &view.history, kind, unit) {
            crate::history_table::TableAction::Close => return,
            crate::history_table::TableAction::Plot(data) => self.history_plot = Some(data),
            crate::history_table::TableAction::None => {}
        }
        self.history_table = Some(table);
    }

    /// PrePoMax's Search Contact Pairs; it replaces an open item dialog.
    fn open_contact_search(&mut self) {
        if let Some(model) = self.setup_model() {
            self.contact_search = Some(ContactSearchDialog::new(&model.fe));
            self.editor = None;
            self.set_tree_view(TreeView::FeModel);
        }
    }

    fn contact_search_window(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(model)) = (&mut self.contact_search, &mut self.model) else {
            return;
        };
        match dialog.show(ctx, model) {
            SearchResult::Open => {}
            SearchResult::Ok(ties, pairs, joints) => {
                let mut created = format!(
                    "Contact search: {} ties and {} contact pairs created",
                    ties.len(),
                    pairs.len()
                );
                if !joints.is_empty() {
                    created.push_str(&format!(", {} Node Ties", joints.len()));
                }
                // The first new item shows in the tree, even in a collapsed branch.
                let first = if !pairs.is_empty() {
                    Some(TreeItem::ContactPair(model.fe.contact_pairs.len()))
                } else if !ties.is_empty() {
                    Some(TreeItem::Tie(model.fe.ties.len()))
                } else {
                    (!joints.is_empty()).then_some(TreeItem::NodeTie(model.fe.node_ties.len()))
                };
                if let Some(item) = first {
                    self.tree.selected = Some((TreeView::FeModel, item));
                    self.tree.reveal = true;
                }
                model.fe.ties.extend(ties);
                model.fe.contact_pairs.extend(pairs);
                model.fe.node_ties.extend(joints);
                self.output.push(created);
                self.contact_search = None;
                self.highlighted = None;
            }
            SearchResult::Cancel => {
                self.contact_search = None;
                self.highlighted = None;
            }
        }
    }

    /// PrePoMax's question before deleting, for the context menu and the Delete key alike.
    fn confirm_delete_window(&mut self, ctx: &egui::Context) {
        let Some((view, items)) = self.confirm_delete.clone() else {
            return;
        };
        let mut answer = None;
        // Like PrePoMax, the parts are named in the question.
        let parts: Vec<String> = (items.iter())
            .filter_map(|item| match item {
                TreeItem::Part(index) => (self.tree_model(view))
                    .and_then(|m| m.parts.get(*index))
                    .map(|p| p.name.clone()),
                _ => None,
            })
            .collect();
        let analysis = (items.iter())
            .find_map(|item| match item {
                TreeItem::Analysis(index) => self.model.as_ref()?.fe.analyses.get(*index),
                _ => None,
            })
            .map(|a| {
                let bound = (self.model.as_ref())
                    .and_then(|m| m.bound_results.as_ref())
                    .is_some_and(|b| b.analysis == a.name);
                format!(
                    "Delete the analysis {}?{}",
                    a.name,
                    if bound {
                        "\nThe result bound to it is removed from the project."
                    } else {
                        ""
                    }
                )
            });
        let question = match parts.as_slice() {
            [] if analysis.is_some() => analysis.unwrap_or_default(),
            [] => "Delete the selected item?".into(),
            [name] => format!("Delete the selected part?\n{name}"),
            names => format!("Delete the selected parts?\n{}", names.join(", ")),
        };
        egui::Modal::new(egui::Id::new("confirm delete")).show(ctx, |ui| {
            ui.label(question);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        // Enter confirms and Escape cancels, as with the default buttons of a message box.
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Enter) {
                answer = Some(true);
            } else if i.key_pressed(egui::Key::Escape) {
                answer = Some(false);
            }
        });
        match answer {
            Some(true) => {
                self.confirm_delete = None;
                // The tree may have changed meanwhile, e.g. by another tab's selection.
                let selected = self.tree.selected_parts(view);
                let parts: Vec<usize> = (items.iter())
                    .filter_map(|item| match item {
                        TreeItem::Part(index) => Some(*index),
                        _ => None,
                    })
                    .collect();
                if !parts.is_empty() && parts.iter().all(|p| selected.contains(p)) {
                    self.delete_parts(view, &parts);
                } else if let [item] = items.as_slice()
                    && self.tree.selected.as_ref() == Some(&(view, item.clone()))
                {
                    self.delete_item(view, item.clone());
                }
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    /// The model a tree shows.
    fn tree_model(&self, view: TreeView) -> Option<&Model> {
        match view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        }
    }

    fn delete_parts(&mut self, view: TreeView, parts: &[usize]) {
        match view {
            TreeView::Geometry => self.delete_geometry_parts(parts),
            TreeView::FeModel => self.delete_mesh_parts(parts),
            TreeView::Results => {}
        }
    }

    fn delete_item(&mut self, view: TreeView, item: TreeItem) {
        if let TreeItem::Analysis(index) = item {
            self.delete_analysis(index);
        } else if let TreeItem::Part(index) = item {
            self.delete_parts(view, &[index]);
        } else if let TreeItem::ResultFieldOutput(field) = item {
            self.delete_field_output(field);
        } else if let TreeItem::HistorySet(set) = item {
            self.delete_history_output(set);
        } else if let TreeItem::HotSpot(index) = item {
            self.delete_hot_spot(index);
        } else if let TreeItem::MeshItem(index) = item {
            if let Some(geometry) = self.model.as_mut().and_then(|m| m.geometry.as_mut())
                && index < geometry.mesh_items.len()
            {
                geometry.mesh_items.remove(index);
                self.tree.selected = None;
                self.mesh_item_editor = None;
            }
        } else if let Some(feature) = tree::feature(&item) {
            let model = match view {
                TreeView::Results => self.results.get_mut(self.current_result),
                _ => self.model.as_mut(),
            };
            if let Some(model) = model
                && crate::features::delete(&mut model.fe, feature)
            {
                self.tree.selected = None;
                self.feature_dialog = None;
                if let Some(view) = &mut model.results
                    && feature.kind == FeatureKind::ResultPlane
                {
                    view.plane_result = match view.plane_result {
                        Some(i) if i == feature.index => None,
                        Some(i) if i > feature.index => Some(i - 1),
                        other => other,
                    };
                }
            }
        } else if let Some(kind) = tree::set_kind(&item) {
            if let (Some(model), Some(name)) = (self.model.as_mut(), tree::set_name(&item)) {
                model
                    .fe
                    .mesh_sets
                    .retain(|s| s.kind != kind || s.name != name);
                kind.remove(&mut model.mesh, name);
                self.tree.selected = None;
                self.editor = None;
                self.highlighted = None;
            }
        } else if let Some(model) = self.model.as_mut()
            && crate::setup::delete(&mut model.fe, &item)
        {
            self.tree.selected = None;
            self.editor = None;
        }
    }

    /// PrePoMax's import into a model with geometry or a mesh: the new parts join the
    /// geometry. The parts already there keep their names, and the selections on the
    /// geometry and the mesh follow Gmsh's new numbering of the faces, edges and vertices.
    fn geometry_added(
        &mut self,
        paths: &[PathBuf],
        result: Result<Box<plx_mesher::CadAddition>, String>,
    ) {
        let files: Vec<String> = (paths.iter())
            .map(|p| {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            })
            .collect();
        let files = files.join(", ");
        let addition = match result {
            Ok(addition) => *addition,
            Err(error) => {
                self.output
                    .push(format!("Error importing {files}: {error}"));
                return;
            }
        };
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let plx_mesher::CadAddition {
            import,
            renumbered,
            added,
        } = addition;
        let view = Model::geometry_view(&model.path, import.display);
        match geometry_check(model.fe.properties.space, &view) {
            Ok(faces) if !faces.is_empty() => {
                let faces: Vec<String> = faces.iter().map(i32::to_string).collect();
                self.output.push(format!(
                    "Note: the normal of face {} points in -z; the elements are flipped \
                     when meshing.",
                    faces.join(", ")
                ));
            }
            Ok(_) => {}
            Err(error) => {
                self.output
                    .push(format!("Error importing {files}: {error}"));
                return;
            }
        }
        if model.geometry.is_some() {
            model.fe.renumber_cad(&renumbered);
            if model.has_cad() {
                let mut mesh = model.mesh.clone();
                mesh.cad = mesh.cad.renumbered(&renumbered);
                model.set_mesh(mesh);
            }
        }
        model.geometry = Some(import.geometry);
        for warning in &import.warnings {
            self.output.push(format!("Warning: {warning}"));
        }
        self.output.push(format!(
            "{files} imported: {} part(s) added ({})",
            added.len(),
            added.join(", ")
        ));
        self.geometry = Some(view);
        self.after_parts_changed();
        self.set_tree_view(TreeView::Geometry);
        self.view_command = Some(ViewCommand::Fit);
    }

    /// PrePoMax's import of a mesh into a model: the mesh of each input file is added as
    /// parts. The parts already there keep their numbers and names; a part, set or surface
    /// of the new mesh whose name is taken gets a number appended.
    fn meshes_added(&mut self, files: Vec<(PathBuf, Result<Box<plx_io::inp::InpImport>, String>)>) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let mut any = false;
        for (path, result) in files {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let import = match result {
                Ok(import) => *import,
                Err(error) => {
                    self.output.push(format!("Error importing {name}: {error}"));
                    continue;
                }
            };
            if import.mesh.element_count() == 0 {
                self.output.push(format!(
                    "Error importing {name}: it contains no elements that can be displayed"
                ));
                continue;
            }
            let space = plx_model::ModelSpace::of_mesh(&import.mesh);
            if model.mesh.element_count() == 0 && model.geometry.is_none() {
                // Into an empty model, the mesh decides what the model is, as opening it does.
                model.fe.properties.space = space;
            } else if space.is_2d() != model.fe.properties.space.is_2d() {
                self.output.push(format!(
                    "Error importing {name}: its elements are {}, the model is {}",
                    space.label(),
                    model.fe.properties.space.label()
                ));
                continue;
            }
            let (nodes, elements) = (import.mesh.node_count(), import.mesh.element_count());
            let mut added = Vec::new();
            model.edit_mesh(|mesh, _| added = mesh.append(import.mesh));
            self.output.push(format!(
                "{name} imported: {nodes} nodes, {elements} elements, {} part(s) added ({})",
                added.len(),
                added.join(", ")
            ));
            for warning in &import.warnings {
                self.output.push(format!("Warning: {warning}"));
            }
            // The heading is not part of a mesh, so skipping it is no news.
            let keywords: Vec<String> = (import.skipped_keywords.keys())
                .filter(|k| *k != "HEADING")
                .map(|k| format!("*{k}"))
                .collect();
            if !keywords.is_empty() {
                self.output.push(format!(
                    "Only the mesh is read; skipped: {}",
                    keywords.join(", ")
                ));
            }
            any = true;
        }
        if any {
            self.after_parts_changed();
            self.set_tree_view(TreeView::FeModel);
            self.view_command = Some(ViewCommand::Fit);
        }
    }

    /// PrePoMax's Delete of geometry parts: the geometry loses the solids or faces; a mesh
    /// already generated from them stays, as parts of the FE model.
    fn delete_geometry_parts(&mut self, indices: &[usize]) {
        let names: Vec<String> = (self.geometry.as_ref())
            .map(|g| {
                (indices.iter())
                    .filter_map(|&i| g.parts.get(i))
                    .map(|p| p.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let Some(geometry) = &model.geometry else {
            return;
        };
        if names.is_empty() {
            return;
        }
        let name = names.join(", ");
        // All in one go, so that Gmsh numbers the faces, edges and vertices anew once.
        let smaller = match plx_mesher::delete_parts_renumbered(geometry, &names) {
            Ok((smaller, tags)) => {
                // Gmsh numbers the faces, edges and vertices anew; the mesh and the
                // selections on the geometry follow.
                if model.has_cad() {
                    let mut mesh = model.mesh.clone();
                    mesh.cad = mesh.cad.renumbered(&tags);
                    model.fe.renumber_cad(&tags);
                    model.set_mesh(mesh);
                }
                smaller
            }
            Err(error) => {
                self.output
                    .push(format!("{name} cannot be deleted: {error}"));
                return;
            }
        };
        self.geometry = match &smaller {
            Some(geometry) => match plx_mesher::tessellate(geometry) {
                Ok(display) => Some(Model::geometry_view(&model.path, display)),
                Err(error) => {
                    self.output
                        .push(format!("Geometry cannot be displayed: {error}"));
                    None
                }
            },
            None => None,
        };
        model.geometry = smaller;
        self.output.push(deleted_message(&names));
        self.after_parts_changed();
    }

    /// PrePoMax's Delete of mesh parts: their elements go, with the nodes no other part has.
    fn delete_mesh_parts(&mut self, indices: &[usize]) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let names: Vec<String> = (indices.iter())
            .filter_map(|&i| model.parts.get(i))
            .map(|p| p.name.clone())
            .collect();
        if names.is_empty() {
            return;
        }
        // By name, which stays while the others go.
        let mut mesh = model.mesh.clone();
        for name in &names {
            mesh = plx_mesher::delete_mesh_part(&mesh, name);
        }
        model.set_mesh(mesh);
        self.output.push(deleted_message(&names));
        self.after_parts_changed();
    }

    /// PrePoMax's Merge of mesh parts: the selected parts become one, named after the
    /// first; the model's regions on the others follow.
    fn merge_mesh_parts(&mut self, parts: &[usize]) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let mut merged = None;
        model.edit_mesh(|mesh, fe| {
            merged = mesh.merge_parts(parts);
            if let Some((kept, gone)) = &merged {
                for name in gone {
                    fe.rename_part(name, kept);
                }
            }
        });
        let Some((kept, gone)) = merged else {
            return;
        };
        self.output
            .push(format!("Parts {} merged into {kept}", gone.join(", ")));
        self.after_parts_changed();
    }

    /// PrePoMax's Edit menu.
    fn edit_menu(&mut self, ui: &mut egui::Ui) {
        let blocked = self.undo_blocked();
        let history = self.model.as_ref().map(|m| &m.history);
        let can_undo = blocked.is_none() && history.is_some_and(|h| h.can_undo());
        let can_redo = blocked.is_none() && history.is_some_and(|h| h.can_redo());
        let reason = blocked.unwrap_or("Nothing to undo.");
        let undo = egui::Button::new("Undo").shortcut_text("Ctrl+Z");
        if ui
            .add_enabled(can_undo, undo)
            .on_disabled_hover_text(reason)
            .clicked()
        {
            self.undo();
        }
        let reason = blocked.unwrap_or("Nothing to redo.");
        let redo = egui::Button::new("Redo").shortcut_text("Ctrl+Shift+Z");
        if ui
            .add_enabled(can_redo, redo)
            .on_disabled_hover_text(reason)
            .clicked()
        {
            self.redo();
        }
    }

    /// Why the changes of the FE model cannot be undone or redone right now, if so: an open
    /// dialog may refer to what an undo takes away.
    fn undo_blocked(&self) -> Option<&'static str> {
        if self.tree_view == TreeView::Results {
            return Some("Undo and Redo work on the Geometry and FE Model tabs.");
        }
        if self.meshing.is_some() {
            return Some("Wait until meshing has finished.");
        }
        let dialog_open = self.dialog.is_some()
            || self.editor.is_some()
            || self.model_dialog.is_some()
            || self.mesh_setup.is_some()
            || self.mesh_item_editor.is_some()
            || self.confirm_delete.is_some()
            || self.keyword_editor.is_some()
            || self.contact_search.is_some()
            || self.mesh_tool.is_some()
            || self.material_library.is_some()
            || self.feature_dialog.is_some()
            || self.analysis_dialog.is_some();
        dialog_open.then_some("Close the open dialog first.")
    }

    /// Edit > Undo: reverts the last change of the FE model. The tree selection and what is
    /// drawn may refer to items it takes away, so they are built anew.
    fn undo(&mut self) {
        if self.undo_blocked().is_some() {
            return;
        }
        if let Some(model) = &mut self.model
            && crate::undo::undo(model)
        {
            self.output.push("Undo".into());
            self.after_parts_changed();
        }
    }

    /// Edit > Redo: makes the last undone change of the FE model again.
    fn redo(&mut self) {
        if self.undo_blocked().is_some() {
            return;
        }
        if let Some(model) = &mut self.model
            && crate::undo::redo(model)
        {
            self.output.push("Redo".into());
            self.after_parts_changed();
        }
    }

    fn after_parts_changed(&mut self) {
        self.tree.selected = None;
        self.menu_part = None;
        self.dialog = None;
        self.mesh_item_editor = None;
        self.highlighted = None;
        self.symbols_shown = None;
        self.frame_cache.clear();
        self.results_changed = true;
    }

    fn editor_window(&mut self, ctx: &egui::Context) {
        let (Some(editor), Some(model)) = (&mut self.editor, &mut self.model) else {
            return;
        };
        match editor.show(ctx, model) {
            EditorResult::Open => {}
            EditorResult::Ok => {
                if let Some(editor) = self.editor.take() {
                    let created = editor.new_interaction_item(&model.fe);
                    let sets = model.fe.mesh_sets.clone();
                    if let Some((kind, name)) = editor.replaced_set() {
                        kind.remove(&mut model.mesh, name);
                    }
                    editor.apply(&mut model.fe);
                    if model.fe.mesh_sets != sets {
                        model.fe.sync_mesh_sets(&sets, &mut model.mesh);
                    }
                    // The new item shows in the tree, even in a collapsed branch.
                    if let Some(item) = created {
                        self.tree.selected = Some((TreeView::FeModel, item));
                        self.tree.reveal = true;
                    }
                }
                self.highlighted = None;
            }
            EditorResult::Cancel => {
                self.editor = None;
                self.highlighted = None;
            }
        }
    }

    /// The motion preview of a boundary condition dialog moves the nodes of its region in
    /// the 3D view; without one they are back in place.
    fn update_motion_preview(&mut self, ctx: &egui::Context) {
        let Some(model) = &mut self.model else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let motion = (self.editor.as_mut()).and_then(|e| e.motion_offsets(now, model));
        if motion.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / 30.0));
        }
        if model.motion != motion {
            model.motion = motion;
            self.results_changed |= self.tree_view == TreeView::FeModel;
        }
    }

    /// Highlights the region of the open dialog, or of the item selected in the tree; a
    /// selected part is outlined in the FE model and in the results alike.
    fn update_highlight(&mut self) {
        let on_results = self.tree_view == TreeView::Results;
        // The current results show the region of an open history output dialog, else the
        // part selected in the Results tree.
        let current = self.current_result;
        for (index, model) in self.results.iter_mut().enumerate() {
            let shown = on_results && index == current;
            let dialog = (self.history_dialog.as_ref()).filter(|_| shown);
            let hot_spot = (self.hot_spot_dialog.as_ref()).filter(|_| shown);
            let transformation = (self.transformation_dialog.as_ref())
                .filter(|d| on_results && d.result == index && index == current);
            let feature = (self.feature_dialog.as_ref())
                .filter(|d| on_results && d.results == Some(index) && index == current);
            let highlight = match (dialog, &self.tree.selected) {
                _ if feature.is_some() => feature.map(|d| d.highlight(model)).unwrap_or_default(),
                _ if transformation.is_some() => transformation
                    .map(TransformationDialog::highlight)
                    .unwrap_or_default(),
                _ if hot_spot.is_some() => hot_spot.map(|d| d.highlight(model)).unwrap_or_default(),
                (Some(dialog), _) => dialog.highlight(model),
                (None, Some((TreeView::Results, TreeItem::Part(_)))) if index == current => {
                    Highlight::parts(self.tree.selected_parts(TreeView::Results))
                }
                _ => Highlight::default(),
            };
            if highlight != model.highlight {
                model.highlight = highlight;
                self.results_changed |= on_results && index == current;
            }
        }
        self.update_geometry_highlight();
        let Some(model) = &mut self.model else {
            return;
        };
        let feature = self.feature_dialog.as_ref().filter(|d| d.results.is_none());
        let highlight = if let Some(dialog) = &self.section_dialog {
            self.highlighted = None;
            dialog.highlight()
        } else if let Some(dialog) = feature {
            self.highlighted = None;
            dialog.highlight(model)
        } else if let Some(dialog) = &self.contact_search {
            self.highlighted = None;
            dialog.highlight(model)
        } else if let Some(editor) = &self.editor {
            self.highlighted = None;
            editor.highlight(model)
        } else {
            let parts = self.tree.selected_parts(TreeView::FeModel);
            let selection = (self.tree.selected.clone(), parts.clone());
            if self.highlighted.as_ref() == Some(&selection) {
                return;
            }
            self.highlighted = Some(selection);
            match &self.tree.selected {
                Some((TreeView::FeModel, TreeItem::Part(_))) => Highlight::parts(parts),
                Some((TreeView::FeModel, item)) => crate::setup::item_highlight(model, item),
                _ => Default::default(),
            }
        };
        if highlight != model.highlight {
            model.highlight = highlight;
            self.results_changed |= !on_results;
        }
    }

    /// The geometry shows the item of an open mesh setup dialog, else the selected part or
    /// mesh setup item.
    fn update_geometry_highlight(&mut self) {
        let Some(view) = &mut self.geometry else {
            return;
        };
        let items = (self.model.as_ref())
            .and_then(|m| m.geometry.as_ref())
            .map_or(&[][..], |g| g.mesh_items.as_slice());
        let highlight = match (&self.mesh_item_editor, &self.tree.selected) {
            (Some(editor), _) => editor.highlight(view),
            (None, Some((TreeView::Geometry, TreeItem::Part(_)))) => {
                Highlight::parts(self.tree.selected_parts(TreeView::Geometry))
            }
            (None, Some((TreeView::Geometry, TreeItem::MeshItem(index)))) => items
                .get(*index)
                .map(|item| crate::meshing::item_highlight(view, &item.kind))
                .unwrap_or_default(),
            // Without a selection, the faces and edges meshing failed on.
            _ => crate::meshing::entities_highlight(view, &self.mesh_failure),
        };
        if highlight != view.highlight {
            view.highlight = highlight;
            self.results_changed |= self.tree_view == TreeView::Geometry;
        }
    }

    /// Draws the symbols of the boundary conditions and loads of one step in the FE model,
    /// as PrePoMax does for the step chosen to show: the step of the edited or selected
    /// item, else the last step. The selected or edited item is drawn in red.
    fn update_symbols(&mut self, ctx: &egui::Context) {
        // As in PrePoMax, symbols rest while the exploded view is edited or moves.
        let exploding = self.exploded_dialog.is_some()
            || (self.model.as_ref()).is_some_and(|m| m.explosion.is_animating());
        let items = if exploding {
            Vec::new()
        } else {
            self.symbol_items()
        };
        let shown = (self.model.as_ref())
            .map(|m| m.parts.iter().map(|p| p.visible).collect())
            .unwrap_or_default();
        let version = (self.model.as_ref()).map_or(0, |m| m.explosion.version());
        let key = (items, shown, version);
        if self.symbols_shown.as_ref() == Some(&key) {
            return;
        }
        self.viewport.symbols = (self.model.as_ref())
            .filter(|_| !key.0.is_empty())
            .map(|model| symbols::build(model, &key.0))
            .unwrap_or_default();
        self.symbols_shown = Some(key);
        ctx.request_repaint();
    }

    fn symbol_items(&self) -> Vec<symbols::Item> {
        let Some(model) = self
            .model
            .as_ref()
            .filter(|_| self.tree_view == TreeView::FeModel)
        else {
            return Vec::new();
        };
        let edited = self.editor.as_ref().and_then(Editor::step_item);
        let selected = match &self.tree.selected {
            Some((TreeView::FeModel, item)) => Some(item),
            _ => None,
        };
        // Constraints belong to no step: PrePoMax draws them with the symbols of any step.
        let mut items = Vec::new();
        let edited_constraint = self.editor.as_ref().and_then(|e| e.editing_constraint());
        for (i, constraint) in model.fe.constraints.iter().enumerate() {
            if !constraint.active() || edited_constraint == Some(Some(i)) {
                continue;
            }
            // Like PrePoMax, a hidden constraint is still drawn while selected.
            let is_selected =
                edited_constraint.is_none() && selected == Some(&TreeItem::Constraint(i));
            if !constraint.visible() && !is_selected {
                continue;
            }
            if let Some((kind, region)) = symbols::Kind::of_constraint(constraint) {
                items.push(symbols::Item {
                    kind,
                    region,
                    selected: is_selected,
                });
            }
        }
        items.extend(self.editor.as_ref().and_then(Editor::constraint_item));
        let step_of = |item: &TreeItem| match *item {
            TreeItem::Step(s)
            | TreeItem::StepGroup(s, _)
            | TreeItem::BoundaryCondition(s, _)
            | TreeItem::Load(s, _)
            | TreeItem::FieldOutput(s, _)
            | TreeItem::HistoryOutput(s, _)
            | TreeItem::DefinedField(s, _) => Some(s),
            _ => None,
        };
        let index = (edited.as_ref().map(|(s, ..)| *s))
            .or_else(|| selected.and_then(step_of))
            .or_else(|| model.fe.steps.len().checked_sub(1));
        let Some((index, step)) = index.and_then(|i| Some((i, model.fe.steps.get(i)?))) else {
            return items;
        };
        // The edited item replaces its saved version.
        let replaced = |load: bool, i: usize| {
            edited.as_ref().is_some_and(|(_, edited_index, item)| {
                *edited_index == Some(i) && matches!(item.kind, symbols::Kind::Load(_)) == load
            })
        };
        let is_selected = |item: TreeItem| edited.is_none() && selected == Some(&item);
        // Like PrePoMax, deactivated items have no symbols, hidden ones only while selected;
        // the edited one is drawn anyway.
        for (i, bc) in step.boundary_conditions.iter().enumerate() {
            let selected = is_selected(TreeItem::BoundaryCondition(index, i));
            if step.active && bc.active && (bc.visible || selected) && !replaced(false, i) {
                items.push(symbols::Item {
                    kind: symbols::Kind::Boundary(bc.kind),
                    region: bc.region.clone(),
                    selected,
                });
            }
        }
        for (i, load) in step.loads.iter().enumerate() {
            let selected = is_selected(TreeItem::Load(index, i));
            if step.active && load.active && (load.visible || selected) && !replaced(true, i) {
                items.push(symbols::Item {
                    kind: symbols::Kind::Load(load.kind.clone()),
                    region: load.region.clone(),
                    selected,
                });
            }
        }
        items.extend(edited.map(|(_, _, item)| item));
        items
    }

    fn shown_model(&self) -> ShownModel {
        ShownModel(self.tree_view, self.current_result)
    }

    fn model_mut(&mut self, which: ShownModel) -> Option<&mut Model> {
        match which.0 {
            TreeView::Results => self.results.get_mut(which.1),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        }
    }

    /// Whether the shown model is exploded.
    fn exploded_applied(&self) -> bool {
        self.shown().is_some_and(|m| m.explosion.applied.is_some())
    }

    /// PrePoMax's toolbar button: explodes the shown model by the last used parameters at
    /// half the scale, or puts it back together, animated either way.
    fn toggle_exploded(&mut self) {
        if self.exploded_dialog.is_some() {
            return;
        }
        let last = self.last_exploded.clone();
        let Some(model) = self.shown_mut() else {
            return;
        };
        let (from, to, parameters) = match model.explosion.applied.take() {
            Some(parameters) => (parameters.scale(), 0.0, parameters),
            None if model.parts.len() > 1 => {
                let parameters = crate::exploded::Parameters {
                    scale_factor: 0.5,
                    ..last
                };
                model.explosion.applied = Some(parameters.clone());
                (0.0, parameters.scale(), parameters)
            }
            None => return,
        };
        animate_explosion(model, &parameters, from, to);
        if model.explosion.applied.is_some() {
            self.last_exploded = parameters;
        }
        self.viewport.preview = Default::default();
    }

    /// Opens the exploded view dialog on the shown model and previews it at once.
    fn open_exploded_dialog(&mut self) {
        if self.exploded_dialog.is_some() {
            return;
        }
        let which = self.shown_model();
        let last = self.last_exploded.clone();
        let Some(model) = self.model_mut(which) else {
            return;
        };
        let center = model.mesh.bounds().map_or(glam::DVec3::ZERO, |(a, b)| {
            (glam::DVec3::from(a) + glam::DVec3::from(b)) * 0.5
        });
        let mut dialog = ExplodedDialog::new(model.explosion.applied.clone(), &last, center);
        preview_explosion(model, &mut dialog, true);
        self.exploded_dialog = Some((dialog, which));
    }

    fn exploded_window(&mut self, ctx: &egui::Context) {
        let Some((mut dialog, which)) = self.exploded_dialog.take() else {
            return;
        };
        // Switching to another model ends the dialog as if it was cancelled.
        let (result, change) = if which == self.shown_model() {
            dialog.show(ctx)
        } else {
            (ExplodedResult::Cancel, None)
        };
        let Some(model) = self.model_mut(which) else {
            return;
        };
        // Offsets shown without an animation move nothing in the next tick, so rebuild here.
        let mut moved = false;
        if let Some(change) = change {
            let animate = change == crate::exploded::dialog::Change::Parameter;
            preview_explosion(model, &mut dialog, animate);
            moved = !animate;
        }
        match result {
            ExplodedResult::Open => {
                self.exploded_dialog = Some((dialog, which));
                self.results_changed |= moved;
                return;
            }
            ExplodedResult::Ok => {
                let parameters = dialog.draft.clone();
                let offsets = model.explosion_offsets(&parameters);
                let nothing = offsets.iter().all(|o| *o == glam::DVec3::ZERO);
                // A scale factor of zero shows the model assembled, which is no exploded view.
                model.explosion.applied = (!nothing).then(|| parameters.clone());
                if !model.explosion.is_animating() {
                    model.explosion.show(offsets, false);
                    moved = true;
                }
                self.last_exploded = parameters;
            }
            ExplodedResult::Cancel => {
                match &dialog.before {
                    Some(before) => {
                        let offsets = model.explosion_offsets(before);
                        model.explosion.show(offsets, true);
                    }
                    None => model.explosion.show(Vec::new(), true),
                }
                model.explosion.applied = dialog.before.clone();
            }
            ExplodedResult::Disable => {
                model.explosion.show(Vec::new(), true);
                model.explosion.applied = None;
            }
        }
        self.results_changed |= moved;
        self.viewport.preview = Default::default();
    }

    /// Advances the exploded view animations; the shown model is rebuilt when it moved.
    fn update_explosion(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let shown = self.shown_model();
        let mut animating = false;
        let mut moved = false;
        let models = (self.model.iter_mut().map(|m| (TreeView::FeModel, 0, m)))
            .chain(self.geometry.iter_mut().map(|m| (TreeView::Geometry, 0, m)))
            .chain((self.results.iter_mut().enumerate()).map(|(i, m)| (TreeView::Results, i, m)));
        for (view, index, model) in models {
            let changed = model.explosion.tick(now);
            animating |= model.explosion.is_animating();
            let is_shown = view == shown.0 && (view != TreeView::Results || index == shown.1);
            moved |= changed && is_shown;
        }
        if moved {
            self.results_changed = true;
            self.viewport.preview = Default::default();
        }
        if animating {
            ctx.request_repaint();
        }
    }

    fn section_picks(&self) -> bool {
        self.section_dialog
            .as_ref()
            .is_some_and(SectionDialog::picks)
    }

    /// Opens the section view dialog on the current section, or on a new one facing away
    /// from the viewer.
    fn open_section_dialog(&mut self) {
        if self.section_dialog.is_some() {
            return;
        }
        if let Some(model) = self.shown() {
            let forward = self.viewport.camera().forward().as_dvec3();
            let dialog = SectionDialog::new(self.section.clone(), model, forward);
            self.section_dialog = Some(dialog);
        }
    }

    fn section_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.section_dialog.take() else {
            return;
        };
        let Some(model) = self.shown() else {
            return;
        };
        match dialog.show(ctx, model) {
            SectionResult::Open => {
                self.section_dialog = Some(dialog);
                return;
            }
            SectionResult::Ok => self.section = Some(dialog.draft),
            SectionResult::Cancel => self.section = dialog.before,
            SectionResult::Disable => self.section = None,
        }
        self.viewport.preview = Default::default();
    }

    /// The model whose features a dialog edits: a results file, or the FE model.
    fn feature_model(&self, results: Option<usize>) -> Option<&Model> {
        match results {
            Some(index) => self.results.get(index),
            None => self.model.as_ref(),
        }
    }

    fn feature_picks(&self) -> bool {
        (self.feature_dialog.as_ref()).is_some_and(|d| d.picks() && self.feature_dialog_shown())
    }

    /// Whether the open feature dialog belongs to what the 3D view shows.
    fn feature_dialog_shown(&self) -> bool {
        match self.feature_dialog.as_ref().map(|d| d.results) {
            Some(Some(index)) => {
                self.tree_view == TreeView::Results && index == self.current_result
            }
            Some(None) => self.tree_view == TreeView::FeModel,
            None => false,
        }
    }

    fn feature_window(&mut self, ctx: &egui::Context) {
        if self.feature_dialog.is_some() && !self.feature_dialog_shown() {
            self.feature_dialog = None;
            self.viewport.preview = Default::default();
        }
        let Some(mut dialog) = self.feature_dialog.take() else {
            return;
        };
        let model = match dialog.results {
            Some(index) => self.results.get_mut(index),
            None => self.model.as_mut(),
        };
        let Some(model) = model else {
            return;
        };
        let cut = self.plane_values.as_ref();
        let result = dialog.show(ctx, model, cut);
        let plot_name = dialog.plot_name(&model.fe);
        if dialog.take_plot_toggle()
            && let Some(name) = plot_name.clone()
        {
            match model.path_plots.iter().position(|p| p.is_path(&name)) {
                Some(index) => {
                    model.path_plots.remove(index);
                }
                None => {
                    model.path_plots.push(PathPlot::new(name.clone()));
                    dialog.set_plot_key(name);
                }
            }
        }
        match result {
            FeatureResult::Open => self.feature_dialog = Some(dialog),
            FeatureResult::Ok => {
                // The plot follows a renamed path.
                if let (Some(old), Some(path)) = (plot_name, dialog.path_draft()) {
                    for plot in model.path_plots.iter_mut().filter(|p| p.is_path(&old)) {
                        plot.path = path.name.clone();
                    }
                }
                dialog.apply(&mut model.fe, &model.mesh);
                self.viewport.preview = Default::default();
            }
            FeatureResult::Cancel => {
                // The plot of a path that was not created goes with it.
                if let Some(old) = plot_name.filter(|_| dialog.creates()) {
                    model.path_plots.retain(|p| !p.is_path(&old));
                }
                self.viewport.preview = Default::default();
            }
        }
    }

    /// Results take the coordinate systems of their features, for transformed field outputs.
    fn sync_result_features(&mut self) {
        for model in &mut self.results {
            let Some(view) = &mut model.results else {
                continue;
            };
            if view.coordinate_systems == model.fe.coordinate_systems {
                continue;
            }
            let systems = model.fe.coordinate_systems.clone();
            let warnings = view.set_coordinate_systems(systems, &model.mesh);
            self.output.extend(warnings);
            self.results_changed = true;
        }
    }

    /// The plots of result paths in the 3D view, for the shown increment. Plots of deleted
    /// paths are dropped; that of the path in the open dialog follows its edits.
    fn update_path_plots(&mut self) {
        let dialog = self
            .feature_dialog
            .as_ref()
            .filter(|_| self.feature_dialog_shown());
        let Some(model) = (match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => None,
        }) else {
            self.viewport.path_plots.clear();
            return;
        };
        let edited = dialog.and_then(|d| Some((d.plot_name(&model.fe)?, d.path_draft()?.clone())));
        let path = |fe: &plx_model::FeModel, name: &str| match &edited {
            Some((edited, draft)) if edited == name => Some(draft.clone()),
            _ => fe.result_paths.iter().find(|p| p.name == name).cloned(),
        };
        let edited_hot_spot = (self.hot_spot_dialog.as_ref())
            .map(|d| (d.plot_name(&model.hot_spots.definitions), d.hot_spot()));
        let hot_spot =
            |definitions: &[plx_results::hot_spot::HotSpot], name: &str| match &edited_hot_spot {
                Some((edited, draft)) if edited == name => Some(draft.clone()),
                _ => definitions.iter().find(|h| h.name == name).cloned(),
            };
        let mut plots = std::mem::take(&mut model.path_plots);
        plots.retain(|plot| match plot.kind {
            path_plot::PlotKind::Path => path(&model.fe, &plot.path).is_some(),
            path_plot::PlotKind::HotSpot => {
                hot_spot(&model.hot_spots.definitions, &plot.path).is_some()
            }
            path_plot::PlotKind::Table => true,
        });
        let boxes = (plots.iter_mut().enumerate())
            .filter_map(|(index, plot)| match plot.kind {
                path_plot::PlotKind::Path => {
                    let path = path(&model.fe, &plot.path)?;
                    plot.plot_box(index, model, &path)
                }
                path_plot::PlotKind::HotSpot => {
                    let hot_spot = hot_spot(&model.hot_spots.definitions, &plot.path)?;
                    plot.hot_spot_box(index, model, &hot_spot)
                }
                path_plot::PlotKind::Table => plot.table_box(index),
            })
            .collect();
        model.path_plots = plots;
        self.viewport.path_plots = boxes;
    }

    /// Applies what the user did to the path plots in the 3D view.
    fn path_plot_events(&mut self, events: Vec<PlotEvent>) {
        let Some(model) = (match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => None,
        }) else {
            return;
        };
        for event in events {
            match event {
                PlotEvent::Changed { index, placement } => {
                    if let Some(plot) = model.path_plots.get_mut(index) {
                        plot.placement = placement;
                    }
                }
                PlotEvent::Edit(index) => {
                    let extremes = (self.viewport.path_plots.iter())
                        .find(|b| b.index == index)
                        .and_then(path_plot::PlotBox::extremes);
                    if let (Some(plot), Some(extremes)) = (model.path_plots.get(index), extremes) {
                        self.plot_editor = Some(path_plot::EditDialog::new(
                            self.current_result,
                            index,
                            &plot.placement,
                            extremes,
                            plot.kind == path_plot::PlotKind::Table,
                        ));
                    }
                }
                PlotEvent::Delete(index) => {
                    if index < model.path_plots.len() {
                        model.path_plots.remove(index);
                    }
                }
            }
        }
    }

    /// Opens the Color Scale dialog of the shown results.
    fn open_color_scale(&mut self) {
        if self.tree_view != TreeView::Results {
            return;
        }
        let Some(view) = (self.results.get(self.current_result)).and_then(|m| m.results.as_ref())
        else {
            return;
        };
        let extremes = view.legend().map(|l| (l.min, l.max));
        let dialog = crate::color_scale_dialog::ColorScaleDialog::new(
            self.current_result,
            &view.color_scale,
            extremes,
        );
        self.color_scale_dialog = Some((dialog, view.color_scale.clone()));
    }

    /// The Color Scale dialog; its changes show at once.
    fn color_scale_window(&mut self, ctx: &egui::Context) {
        use crate::color_scale_dialog::DialogResult;
        let Some((mut dialog, applied)) = self.color_scale_dialog.take() else {
            return;
        };
        if self.tree_view != TreeView::Results || dialog.result != self.current_result {
            return;
        }
        let Some(view) = (self.results.get_mut(dialog.result)).and_then(|m| m.results.as_mut())
        else {
            return;
        };
        // The number of colours can also change in the tool bar while the dialog is open.
        if view.color_scale.levels != applied.levels {
            dialog.draft.levels = view.color_scale.levels;
        }
        let result = dialog.show(ctx);
        let scale = match result {
            DialogResult::Open | DialogResult::Ok => dialog.draft.clone(),
            DialogResult::Cancel => dialog.original().clone(),
        };
        if view.color_scale != scale {
            view.color_scale = scale.clone();
            self.results_changed = true;
        }
        if let DialogResult::Open = result {
            self.color_scale_dialog = Some((dialog, scale));
        }
    }

    /// The Edit Annotation dialog of a path plot; its changes show at once.
    fn plot_editor_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.plot_editor.take() else {
            return;
        };
        if self.tree_view != TreeView::Results || dialog.result != self.current_result {
            return;
        }
        let Some(plot) = (self.results.get_mut(dialog.result))
            .and_then(|model| model.path_plots.get_mut(dialog.index))
        else {
            return;
        };
        // Moving and resizing with the mouse goes on while the dialog is open.
        dialog.draft.offset = plot.placement.offset;
        dialog.draft.size = plot.placement.size;
        dialog.draft.height = plot.placement.height;
        match dialog.show(ctx) {
            path_plot::EditResult::Open => {
                plot.placement = dialog.draft.clone();
                self.plot_editor = Some(dialog);
            }
            path_plot::EditResult::Ok => plot.placement = dialog.draft.clone(),
            path_plot::EditResult::Cancel => plot.placement = dialog.original().clone(),
        }
    }

    /// Reference points, coordinate systems and the result path drawn over the shown model.
    fn update_feature_overlay(&mut self) {
        let Some(model) = self.shown() else {
            self.viewport.overlay.features.clear();
            self.viewport.overlay.result_path = None;
            return;
        };
        let dialog = self
            .feature_dialog
            .as_ref()
            .filter(|_| self.feature_dialog_shown());
        let selected = match &self.tree.selected {
            Some((view, item)) if *view == self.tree_view => tree::feature(item),
            _ => None,
        };
        let edited = dialog.and_then(FeatureDialog::item);
        let mut features = crate::features::marks(model, selected, edited);
        features.extend(dialog.and_then(|d| d.mark(model)));
        let result_path = match dialog {
            Some(dialog) if dialog.kind() == FeatureKind::ResultPath => dialog.path_line(model),
            _ => selected
                .filter(|s| s.kind == FeatureKind::ResultPath)
                .and_then(|s| model.fe.result_paths.get(s.index))
                .and_then(|p| crate::features::path_line(model, p)),
        };
        self.viewport.overlay.features = features;
        self.viewport.overlay.result_path = result_path;
    }

    /// The plane whose cut the shown results show alone: that of a plane result being
    /// edited, or of the checked one; a point on it and its unit normal.
    fn shown_plane_cut(&self) -> Option<(glam::DVec3, glam::DVec3)> {
        if self.tree_view != TreeView::Results {
            return None;
        }
        let model = self.results.get(self.current_result)?;
        let view = model.results.as_ref()?;
        let edited = (self.feature_dialog.as_ref())
            .filter(|d| d.results == Some(self.current_result))
            .and_then(FeatureDialog::result_plane);
        let name = match edited {
            Some(name) => name,
            None => &model.fe.result_planes.get(view.plane_result?)?.plane,
        };
        let (point, normal) = model.fe.plane(name)?.resolve(&model.fe).ok()?;
        Some((point.into(), normal.into()))
    }

    /// Cuts the scene at the section plane being edited or shown, or at the plane of shown
    /// plane results, when it or the scene changed.
    fn update_section(&mut self) {
        // Planes of features follow them.
        if let Some(fe) = self.shown().map(|m| m.fe.clone()) {
            if let Some(dialog) = &mut self.section_dialog {
                dialog.draft.resolve(&fe);
            }
            if let Some(section) = &mut self.section {
                section.resolve(&fe);
            }
        }
        // As in PrePoMax, the section view rests while the exploded view is edited or moves.
        let exploding = self.exploded_dialog.is_some()
            || self.shown().is_some_and(|m| m.explosion.is_animating());
        // Plane results show the cut alone, in place of the section view.
        let plane = self.shown_plane_cut().map(|(point, normal)| {
            let definition = PlaneDefinition::PointNormal { point, normal };
            let cut = SectionView {
                definition,
                flipped: false,
                lighten: false,
            };
            (cut, true)
        });
        let section = (self.section_dialog.as_ref())
            .map(|d| &d.draft)
            .or(self.section.as_ref())
            .map(|s| (s.clone(), false));
        let visible = (self.shown())
            .map(|m| m.parts.iter().map(|p| p.visible).collect())
            .unwrap_or_default();
        let wanted = plane
            .or(section)
            .filter(|_| !exploding)
            .map(|(cut, alone)| (self.viewport.scene_version(), visible, cut, alone));
        if wanted == self.section_shown {
            return;
        }
        // Only the model in view is cut; picking in the others sees everything.
        let models = self.model.iter_mut().chain(&mut self.geometry);
        for model in models.chain(&mut self.results) {
            model.clip = None;
        }
        let shown = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        };
        let (mut values, mut plane_values) = (None, None);
        match (shown, &wanted) {
            (Some(model), Some((_, _, section, alone))) => {
                // The model origin is the centre of its bounds, where a principal plane's
                // manipulator sits.
                let anchor = section.anchor(model.origin());
                let normal = section.normal();
                let clip = plx_render::clip_plane(anchor, normal, model.origin());
                let faces = model.section_meshes(anchor, normal, section.lighten);
                model.clip = Some(clip);
                self.viewport.set_section(Some((clip, &faces)));
                let alone = *alone && model.results.is_some();
                self.viewport.set_sections_only(alone);
                if alone {
                    values = model.section_values(anchor, normal, true);
                    plane_values = model.section_values(anchor, normal, false);
                }
            }
            _ => {
                self.viewport.set_section(None);
                self.viewport.set_sections_only(false);
            }
        }
        // The legend of the shown results takes the range on the plane; the others their own.
        let shown_range = values
            .as_ref()
            .map(|v| v.extremes.map(|[min, max]| (min.1, max.1)));
        let on_results = self.tree_view == TreeView::Results;
        for (index, model) in self.results.iter_mut().enumerate() {
            let range = match shown_range {
                Some(range) if on_results && index == self.current_result => range,
                _ => None,
            };
            if let Some(view) = &mut model.results
                && view.section_range != range
            {
                view.section_range = range;
                self.results_changed = true;
            }
        }
        self.section_values = values;
        self.plane_values = plane_values;
        self.section_shown = wanted;
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(window) = &mut self.settings_window else {
            return;
        };
        let new = match window.show(ctx) {
            WindowResult::Open => return,
            WindowResult::Apply(settings) => settings,
            WindowResult::Ok(settings) => {
                self.settings_window = None;
                settings
            }
            WindowResult::Cancel => {
                self.settings_window = None;
                return;
            }
        };
        if new.gmsh != self.settings.gmsh && !plx_mesher::set_library_path(new.gmsh.library()) {
            self.output
                .push("The changed Gmsh library is loaded after a restart".into());
        }
        if new.graphics.theme != self.settings.graphics.theme {
            crate::style::set_theme(ctx, new.graphics.theme);
        }
        if new != self.settings {
            self.settings = new;
            self.update_contour();
        }
    }

    /// PrePoMax-style properties dialog of the double-clicked tree item.
    fn properties_window(&mut self, ctx: &egui::Context) {
        let Some(item) = self.dialog.clone() else {
            return;
        };
        let mut open = true;
        let mut close = false;
        let mut accept = false;
        // Parts of the geometry keep their names, which the mesh setup refers to.
        let part = match item {
            TreeItem::Part(index) if self.tree_view != TreeView::Geometry => Some(index),
            _ => None,
        };
        // The model by its fields, so the window can edit the typed name alongside.
        let shown = match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        };
        egui::Window::new(properties::title(shown, &item))
            .id(egui::Id::new("properties window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let (name, error) = &mut self.part_name;
                let name = part.map(|_| name);
                // Element types are chosen for the FE model, not for results.
                let types = (part.is_some() && self.tree_view == TreeView::FeModel)
                    .then_some(())
                    .zip(shown)
                    .map(|((), model)| properties::ElementTypes {
                        types: &mut self.part_types,
                        space: model.fe.properties.space,
                    });
                properties::show(ui, shown, &item, name, types);
                if let Some(error) = error.as_ref().filter(|_| part.is_some()) {
                    ui.add_space(4.0);
                    ui.colored_label(crate::style::error(), error);
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if part.is_some() {
                        close = ui.button("Cancel").clicked();
                        accept = ui.button("OK").clicked();
                    } else {
                        close = ui.button("Close").clicked();
                    }
                });
            });
        accept |= part.is_some() && ctx.input(|i| i.key_pressed(egui::Key::Enter));
        if let (true, Some(index)) = (accept, part) {
            let name = self.part_name.0.clone();
            let types = std::mem::take(&mut self.part_types);
            let fe_model = self.tree_view == TreeView::FeModel;
            let mut retyped = Vec::new();
            let renamed = match self.shown_mut() {
                Some(model) => model.rename_part(index, &name).and_then(|()| {
                    let current = (model.parts.get(index))
                        .map(|p| p.element_types.clone())
                        .unwrap_or_default();
                    for ((old, _), new) in current.iter().zip(&types) {
                        if fe_model && !old.eq_ignore_ascii_case(new) {
                            model.set_element_type(index, new)?;
                            retyped.push(format!("{old} to {new}"));
                        }
                    }
                    Ok(())
                }),
                None => Ok(()),
            };
            self.part_types = types;
            if !retyped.is_empty() {
                self.output.push(format!(
                    "{name}: element type changed from {}",
                    retyped.join(", ")
                ));
                self.results_changed = true;
                self.highlighted = None;
            }
            match renamed {
                Ok(()) => close = true,
                Err(error) => self.part_name.1 = Some(error),
            }
        }
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dialog = None;
        }
    }

    /// The unit system CAD geometry is read in: that of an empty model it goes into, such as
    /// one just created with File > New, else that of new models.
    fn import_units(&self) -> UnitSystem {
        match &self.model {
            Some(model) if model.mesh.element_count() == 0 && model.geometry.is_none() => {
                model.fe.properties.units
            }
            _ => self.settings.new_model.units,
        }
    }

    /// PrePoMax's File > New: asks for the model space and unit system of the new model,
    /// with the last choice proposed. `then_import` opens the geometry import afterwards.
    fn new_model(&mut self, then_import: bool) {
        self.model_dialog = Some(ModelPropertiesDialog::new_model(
            self.settings.new_model.clone(),
            then_import,
        ));
    }

    fn edit_model_properties(&mut self) {
        if let Some(model) = &self.model {
            self.model_dialog = Some(ModelPropertiesDialog::edit(model.fe.properties.clone()));
        }
    }

    fn model_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.model_dialog else {
            return;
        };
        let mesh = (self.model.as_ref())
            .filter(|_| dialog.editing)
            .map(|m| &m.mesh);
        let geometry = self.geometry.as_ref().filter(|_| dialog.editing);
        let result = dialog.show(ctx, mesh, geometry);
        let (editing, then_import, convert) = (dialog.editing, dialog.then_import, dialog.convert);
        match result {
            DialogResult::Open => return,
            DialogResult::Cancel => {}
            DialogResult::Ok(properties) if editing => {
                self.set_model_properties(properties, convert)
            }
            DialogResult::Ok(properties) => {
                // New models propose the space and units, never a submodel's global results.
                self.settings.new_model = plx_model::ModelProperties {
                    kind: plx_model::ModelKind::General,
                    global_results: None,
                    ..properties.clone()
                };
                self.create_model(properties);
                self.import_requested = then_import;
            }
        }
        self.model_dialog = None;
    }

    /// Starts an empty model, to import geometry into or to save as a project.
    fn create_model(&mut self, properties: plx_model::ModelProperties) {
        self.close_model();
        let mut model = Model::new(
            std::path::Path::new("Untitled"),
            plx_mesh::FeMesh::default(),
        );
        self.output.push(format!(
            "New model: {}, {}",
            properties.space.label(),
            properties.units.label()
        ));
        model.fe.properties = properties;
        self.model = Some(model);
        self.set_tree_view(TreeView::Geometry);
        self.viewport.set_parts(&[]);
        self.frame_cache.clear();
        self.update_contour();
    }

    /// Takes over changed model properties; a new model space retypes the mesh's surface
    /// elements, as PrePoMax does. A new unit system converts the model with `convert`.
    fn set_model_properties(&mut self, properties: plx_model::ModelProperties, convert: bool) {
        let Some(model) = &mut self.model else {
            return;
        };
        let old = model.fe.properties.clone();
        if convert && old.units != properties.units {
            self.convert_units(properties.units);
        }
        let Some(model) = &mut self.model else {
            return;
        };
        model.fe.properties = properties.clone();
        if old.space != properties.space {
            let mut mesh = model.mesh.clone();
            if properties.space.convert_mesh(&mut mesh) {
                model.set_mesh(mesh);
                self.results_changed = true;
                self.highlighted = None;
            }
            self.output
                .push(format!("Model space: {}", properties.space.label()));
        }
        if old.units != properties.units {
            self.output
                .push(format!("Unit system: {}", properties.units.label()));
        }
        self.results_changed = true;
    }

    /// Converts the open model into another unit system: its values, its mesh and its
    /// geometry, which are scaled for the new length unit.
    fn convert_units(&mut self, units: UnitSystem) {
        let Some(model) = &mut self.model else {
            return;
        };
        let conversion = plx_model::convert::Conversion::new(model.fe.properties.units, units);
        let notes = model.fe.convert_units(units);
        let factor = conversion.length_factor();
        if factor != 1.0 {
            if model.mesh.node_count() > 0 {
                let mut mesh = model.mesh.clone();
                mesh.scale(factor);
                model.set_mesh(mesh);
                self.highlighted = None;
            }
            if let Some(geometry) = &mut model.geometry {
                geometry.convert_sizes(&conversion);
                match plx_mesher::scale_geometry(geometry, factor) {
                    Ok(scaled) => {
                        *geometry = scaled;
                        match plx_mesher::tessellate(geometry) {
                            Ok(display) => {
                                self.geometry = Some(Model::geometry_view(&model.path, display))
                            }
                            Err(error) => self
                                .output
                                .push(format!("Geometry cannot be displayed: {error}")),
                        }
                    }
                    Err(error) => self
                        .output
                        .push(format!("Geometry could not be converted: {error}")),
                }
            }
            self.frame_cache.clear();
            self.view_command = Some(ViewCommand::Fit);
        }
        self.output.extend(notes);
        self.output.push(format!(
            "Model converted to {} (lengths × {factor})",
            units.label()
        ));
    }

    /// Removes the model and all results, like PrePoMax's File > New.
    fn close_model(&mut self) {
        if let Some(model) = self.model.take() {
            self.output.push(format!("{} closed", model.file_name()));
        }
        self.geometry = None;
        self.campbell = None;
        self.mesh_setup = None;
        self.mesh_item_editor = None;
        self.meshing = None;
        self.close_results(true);
        self.tree.selected = None;
        self.dialog = None;
        self.section = None;
        self.section_dialog = None;
        self.exploded_dialog = None;
        self.editor = None;
        self.highlighted = None;
        self.parked_camera = None;
        self.results_changed = true;
    }

    /// Plays the animation and shows its window; marks the scene for rebuilding.
    fn animate(&mut self, ctx: &egui::Context) {
        if let Some((file, encoder)) = &self.animation_encoding
            && let Some(result) = encoder.poll()
        {
            self.output
                .push(result.unwrap_or_else(|err| format!("{file}: {err}")));
            self.animation_encoding = None;
        }
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        };
        let Some(view) = model
            .and_then(|m| m.results.as_mut())
            .filter(|view| view.animation.is_some())
        else {
            if self.recording.take().is_some() {
                self.output.push("Animation export cancelled".into());
            }
            return;
        };
        if let Some(recording) = self.recording.take() {
            let ((frame, playing), file) = (recording.restore, recording.file.clone());
            let (recording, step) = recording.update(ctx, self.viewport.rect);
            self.recording = recording;
            let shown = match step {
                animation_export::Step::Wait => return,
                animation_export::Step::Show(frame) => frame,
                animation_export::Step::Finished(encoder) => {
                    self.animation_encoding = Some((file, encoder));
                    frame
                }
                animation_export::Step::Cancelled => {
                    self.output.push("Animation export cancelled".into());
                    frame
                }
            };
            if let Some(animation) = &mut view.animation {
                animation.go_to(shown);
                // Back to the state from before once the recording ended.
                animation.playing = self.recording.is_none() && playing;
            }
            view.show_animation_frame();
            self.frame_changed = true;
            return;
        }
        let Some(animation) = &mut view.animation else {
            return;
        };
        if animation.playing {
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            if animation.tick(dt) {
                view.show_animation_frame();
                self.frame_changed = true;
            }
            ctx.request_repaint();
        }
        match animation_window(ctx, view, self.animation_encoding.is_some()) {
            WindowEvent::None => {}
            WindowEvent::Export(format) => {
                let ffmpeg = animation_export::ffmpeg_executable(&self.settings.post.ffmpeg);
                if let Some(recording) =
                    start_animation_export(view, format, ffmpeg, &mut self.output)
                {
                    let first = recording.first_frame();
                    self.recording = Some(recording);
                    if let Some(animation) = &mut view.animation {
                        animation.go_to(first);
                    }
                    view.show_animation_frame();
                    self.frame_changed = true;
                }
            }
            WindowEvent::Frame => {
                view.show_animation_frame();
                self.frame_changed = true;
            }
            WindowEvent::Settings => self.results_changed = true,
            WindowEvent::Close => {
                view.stop_animation();
                self.results_changed = true;
            }
        }
    }

    /// Shows the sound window of the shown results and drives its audio output.
    fn play_sound(&mut self, ctx: &egui::Context) {
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            _ => self.model.as_mut(),
        };
        let Some(view) = model.and_then(|m| m.results.as_mut()) else {
            self.audio = None;
            return;
        };
        let shown = view.increment;
        let Some(sound) = &mut view.sound else {
            // Closing the window releases the audio device.
            self.audio = None;
            if view.superposition.take().is_some() {
                self.results_changed = true;
            }
            return;
        };
        let playing = self.audio.as_ref().is_some_and(|a| a.synth().sounding());
        let actions = sound::window(ctx, sound, shown, playing);
        if actions.play {
            if self.audio.is_none() {
                match sound::Player::open() {
                    Ok(player) => self.audio = Some(player),
                    Err(error) => sound.message = Some(error),
                }
            }
            if let Some(audio) = &self.audio {
                audio.synth().start(sound.tones());
                sound.message = None;
            }
        }
        if let Some(audio) = &self.audio {
            if actions.stop {
                audio.synth().stop();
            } else if actions.changed && playing {
                audio.synth().update(sound.tones());
            }
        }
        if actions.export {
            let picked = rfd::FileDialog::new()
                .set_title("Save Sound")
                .add_filter("WAV file (*.wav)", &["wav"])
                .set_file_name(format!("Modes-Step-{}.wav", sound.step))
                .save_file();
            if let Some(mut path) = picked {
                if path.extension().is_none() {
                    path.set_extension("wav");
                }
                let data = sound::wav(sound.tones(), sound.export_seconds());
                match std::fs::write(&path, data) {
                    Ok(()) => self
                        .output
                        .push(timestamped(format!("{} saved", path.display()))),
                    Err(error) => sound.message = Some(format!("Not saved: {error}")),
                }
            }
        }
        if playing || actions.play {
            // The play button turns back when a struck sound has died away.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let now = ctx.input(|i| i.time);
        if actions.play {
            sound.started = Some(now);
            sound.shape_clock = (0.0, f64::NEG_INFINITY);
        }
        if actions.play || actions.changed {
            sound.mix = None;
        }
        // A mode clicked in the window swings in the 3D view until another increment is
        // shown; a playing sound with its overlay takes precedence.
        if let Some(increment) = actions.show {
            sound.preview = Some(increment);
            sound.shape_clock = (0.0, f64::NEG_INFINITY);
        }
        if sound
            .preview
            .is_some_and(|p| p != view.increment && actions.show.is_none())
        {
            sound.preview = None;
        }
        let audible = self.audio.as_ref().is_some_and(|a| a.synth().sounding());
        let playing_overlay = sound.show_shape && audible;
        let only = (!playing_overlay).then_some(sound.preview).flatten();
        let overlay = (playing_overlay || only.is_some()) && !actions.close;
        // The overlay takes the place of an animation.
        if overlay && view.animation.is_some() {
            view.stop_animation();
        }
        if overlay
            && let Some((field, component)) =
                (view.current()).map(|(f, c)| (f.name.clone(), c.name.clone()))
            && let Some(sound) = &mut view.sound
        {
            if sound
                .mix
                .as_ref()
                .is_some_and(|m| !m.shows(&field, &component, only))
            {
                sound.mix = None;
            }
            if sound.mix.is_none() {
                let mix = sound::ShapeMix::new(sound, &view.increments, &field, &component, only);
                sound.mix = mix;
            }
            // A new frame at the chosen rate; the swings advance with the chosen speed, so
            // changing it while playing does not make the shape jump.
            let period = 1.0 / sound.shape_fps.max(1.0) as f64;
            let (swings, last) = sound.shape_clock;
            let since = now - last;
            if since >= period || view.superposition.is_none() {
                let step = if last.is_finite() { since } else { 0.0 };
                let swings = swings + step * sound.shape_speed as f64;
                sound.shape_clock = (swings, now);
                let time = now - sound.started.unwrap_or(now);
                view.superposition = sound.mix.as_ref().map(|m| m.frame(swings, time));
                self.results_changed = true;
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(period));
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(period - since));
            }
        } else if view.superposition.take().is_some() {
            self.results_changed = true;
        }
        if let Some(increment) = actions.show
            && increment != view.increment
        {
            view.stop_animation();
            view.select_increment(increment);
            self.results_changed = true;
        }
        if actions.close {
            view.sound = None;
            self.audio = None;
        }
    }

    /// Draws the shells of the shown FE model thick or thin as the setting asks, again after
    /// their sections changed.
    fn update_thick_shells(&mut self) {
        if self.tree_view != TreeView::FeModel {
            return;
        }
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let wanted = self.settings.graphics.shell_thickness;
        let current = &model.thick_shells;
        if wanted && model.shell_sections().eq(current) || !wanted && current.is_empty() {
            return;
        }
        model.thick_shells = if wanted {
            model.shell_sections().cloned().collect()
        } else {
            Vec::new()
        };
        self.results_changed = true;
    }

    /// Rebuilds the scene after the result selection or deformation changed.
    fn rebuild_if_results_changed(&mut self) {
        let frame_only = !self.results_changed && self.frame_changed;
        if !(std::mem::take(&mut self.results_changed) | std::mem::take(&mut self.frame_changed)) {
            return;
        }
        let model = match self.tree_view {
            TreeView::Results => self.results.get_mut(self.current_result),
            TreeView::Geometry => self.geometry.as_mut(),
            TreeView::FeModel => self.model.as_mut(),
        };
        let Some(model) = model else {
            self.frame_cache.clear();
            self.viewport.set_parts(&[]);
            self.update_contour();
            return;
        };
        let frame = model
            .results
            .as_ref()
            .and_then(|v| v.animation.as_ref())
            .map(|a| a.frame);
        if !frame_only {
            self.frame_cache.clear();
        }
        let fresh;
        let meshes: &Vec<RenderMesh> = match frame {
            Some(frame) => self
                .frame_cache
                .entry(frame)
                .or_insert_with(|| model.render_meshes()),
            None => {
                fresh = model.render_meshes();
                &fresh
            }
        };
        self.viewport.set_parts(meshes);
        for (index, (part, mesh)) in model.parts.iter_mut().zip(meshes).enumerate() {
            part.bounds = mesh.bounds();
            self.viewport.set_part_visible(index, part.visible);
        }
        if model.explosion.is_shown()
            && let Some(bounds) = model.visible_bounds()
        {
            self.viewport.cover(bounds);
        }
        self.update_contour();
    }

    /// Contour settings and annotations of the 3D view for the current result.
    fn update_contour(&mut self) {
        let model = match self.tree_view {
            TreeView::Results => self.results.get(self.current_result),
            TreeView::Geometry => self.geometry.as_ref(),
            TreeView::FeModel => self.model.as_ref(),
        };
        let Some(model) = model else {
            self.viewport.options.contour = None;
            self.viewport.overlay = Overlay {
                show_view_triad: self.settings.graphics.view_triad,
                ..Overlay::default()
            };
            return;
        };
        let view = model.results.as_ref();
        // Results have their own unit system; the geometry is in that of the FE model.
        let units = match view {
            Some(view) => view.units,
            None => self
                .model
                .as_ref()
                .map_or(model.fe.properties.units, |m| m.fe.properties.units),
        };
        let axis = self.axis_line(model);
        let legend = view.and_then(ResultsView::legend);
        self.viewport.options.contour =
            (legend.as_ref()).map(|legend| plx_render::ContourPalette::new(&legend.palette()));
        let (graphics, post) = (&self.settings.graphics, &self.settings.post);
        // The open transformation dialog shows the points and axis of the selected item.
        let transformation = (self.transformation_dialog.as_ref())
            .filter(|d| self.tree_view == TreeView::Results && d.result == self.current_result);
        let marker = |label: &str, extreme: Option<(usize, f32, usize)>| {
            let (index, value, item) = extreme?;
            Some(Marker {
                position: model.node_position_on(index, item)?,
                text: format!(
                    "{label}: {}\nNode id: {}",
                    format_legend_value(value),
                    model.mesh.node_ids()[index]
                ),
            })
        };
        // On the section plane the extremes lie between nodes.
        let section = (self.section_values.as_ref())
            .and_then(|v| v.extremes)
            .filter(|_| view.is_some_and(|v| v.section_range.is_some()));
        let section_marker = |label: &str, (position, value): (glam::DVec3, f32)| {
            Some(Marker {
                position: model.to_render(position.to_array()),
                text: format!("{label}: {}\nSection plane", format_legend_value(value)),
            })
        };
        let picked = self.query.as_ref().map_or(&[][..], QueryWindow::picked);
        let query = crate::query::marks(model, picked);
        self.viewport.overlay = Overlay {
            legend,
            annotations: query.markers,
            annotation_owners: query.owners,
            status: view
                .filter(|_| post.status_block)
                .map_or_else(Vec::new, |v| v.status_lines(&model.file_name())),
            maximum: view.filter(|_| post.max_label).and_then(|v| match section {
                Some([_, max]) => section_marker("Max", max),
                None => marker("Max", v.maximum()),
            }),
            minimum: view.filter(|_| post.min_label).and_then(|v| match section {
                Some([min, _]) => section_marker("Min", min),
                None => marker("Min", v.minimum()),
            }),
            global_origin: graphics.global_axes.then(|| model.global_origin()),
            show_scale_bar: graphics.scale_bar,
            length_unit: units.unit(plx_model::Quantity::Length),
            show_view_triad: graphics.view_triad,
            nodes: (model.highlight.nodes.iter())
                .filter_map(|&id| model.node_position(model.mesh.node_index(id)?))
                .chain(transformation.iter().flat_map(|d| d.points(model)))
                .chain(query.points)
                .collect(),
            edges: render_lines(model, &model.highlight.lines),
            secondary_edges: render_lines(model, &model.highlight.secondary_lines),
            axis,
            paths: self.overlay_paths(),
            lines: (transformation
                .map_or_else(Vec::new, |d| d.lines(model))
                .into_iter())
            .chain(query.lines)
            .collect(),
            features: Vec::new(),
            result_path: None,
        };
        self.update_feature_overlay();
    }
}

/// Lines between nodes in render coordinates; lines with an unknown node are left out.
fn render_lines(model: &Model, lines: &[[plx_mesh::NodeId; 2]]) -> Vec<[glam::Vec3; 2]> {
    (lines.iter())
        .filter_map(|ends| {
            let [a, b] = ends.map(|id| {
                model
                    .mesh
                    .node_index(id)
                    .and_then(|n| model.node_position(n))
            });
            Some([a?, b?])
        })
        .collect()
}

/// What the results tool bar asks for.
struct ToolBarResponse {
    /// Anything that affects the scene changed.
    changed: bool,
    /// The transformations button was clicked.
    transformations: bool,
    /// The colour scale button was clicked.
    color_scale: bool,
}

/// PrePoMax's results tool bar: deformation, colour bands, transformations and the increment
/// with its navigation buttons.
fn results_tool_bar(
    ui: &mut egui::Ui,
    view: &mut ResultsView,
    transformed: bool,
) -> ToolBarResponse {
    let mut changed = false;
    let mut transformations = false;
    let mut color_scale = false;
    ui.horizontal(|ui| {
        ui.label("Deformation");
        let before = view.deformation;
        egui::ComboBox::from_id_salt("deformation")
            .selected_text(view.deformation.label())
            .width(150.0)
            .show_ui(ui, |ui| {
                for choice in Deformation::CHOICES {
                    let response =
                        ui.selectable_value(&mut view.deformation, choice, choice.label());
                    view.deformation_chosen |= response.clicked();
                }
            });
        changed |= view.deformation != before;
        ui.label("Factor");
        let user = view.deformation == Deformation::UserDefined;
        let factor = if user {
            view.user_scale
        } else {
            f64::from(view.scale())
        };
        // A text field of fixed width: a long factor is cut off inside the frame instead of
        // widening the field and pushing the items on its right around.
        let id = ui.id().with("deformation factor");
        let focused = ui.memory(|m| m.has_focus(id));
        let mut text = focused
            .then(|| ui.data_mut(|d| d.get_temp::<String>(id)))
            .flatten()
            .unwrap_or_else(|| numeric::format_physical(factor));
        let response = ui
            .add_enabled(
                user,
                egui::TextEdit::singleline(&mut text)
                    .id(id)
                    .desired_width(80.0),
            )
            .on_hover_text(numeric::format_physical(factor));
        if response.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, text.clone()));
        }
        if user && response.lost_focus() {
            ui.data_mut(|d| d.remove_temp::<String>(id));
            if let Some(value) = numeric::parse_number(&text).filter(|v| v.is_finite())
                && value != view.user_scale
            {
                view.user_scale = value;
                changed = true;
            }
        }
        changed |= ui
            .checkbox(&mut view.show_undeformed, "Show Undeformed")
            .changed();
        ui.separator();
        ui.label("Color levels");
        changed |= ui
            .add(
                numeric::drag_value(&mut view.color_scale.levels)
                    .range(2..=plx_render::contour::MAX_LEVELS),
            )
            .changed();
        color_scale = (ui.button("…"))
            .on_hover_text("Color scale: spectrum and limits")
            .clicked();
        ui.separator();
        let tip = "Transformations: symmetries and patterns";
        transformations = icons::button(ui, Icon::Transformation, tip, true, transformed).clicked();
        ui.separator();

        ui.label("Step, Increment");
        let mut increment = view.increment;
        let selected = view
            .current_increment()
            .map(ResultsView::increment_label)
            .unwrap_or_default();
        egui::ComboBox::from_id_salt("increment")
            .selected_text(selected)
            .width(70.0)
            .show_ui(ui, |ui| {
                for (index, inc) in view.increments.iter().enumerate() {
                    ui.selectable_value(&mut increment, index, ResultsView::increment_label(inc));
                }
            });
        ui.spacing_mut().item_spacing.x = 1.0;
        let last = view.increments.len().saturating_sub(1);
        let current = view.increment;
        for (icon, tooltip, target) in [
            (Icon::First, "First Increment", 0),
            (
                Icon::Previous,
                "Previous Increment",
                current.saturating_sub(1),
            ),
            (Icon::Next, "Next Increment", (current + 1).min(last)),
            (Icon::Last, "Last Increment", last),
        ] {
            let enabled = target != current;
            if icons::button(ui, icon, tooltip, enabled, false).clicked() {
                increment = target;
            }
        }
        if increment != view.increment {
            view.switch_increment(increment);
            changed = true;
        }
        ui.add_space(4.0);
        let animating = view.animation.is_some();
        if icons::button(ui, Icon::Animate, "Animation", true, animating).clicked() {
            if animating {
                view.stop_animation();
            } else {
                view.start_animation(AnimationKind::ScaleFactor);
            }
            changed = true;
        }
        // Only modes of a frequency step have a sound; otherwise the button is greyed out.
        let sounding = view.sound.is_some();
        let is_mode = view
            .current_increment()
            .is_some_and(|i| i.kind == plx_results::AnalysisKind::Frequency);
        let tip = "Sound of the mode shapes";
        if icons::button(ui, Icon::Sound, tip, sounding || is_mode, sounding).clicked() {
            view.sound = match view.sound {
                Some(_) => None,
                None => ModeSound::new(&view.increments, view.increment),
            };
        }
    });
    ToolBarResponse {
        changed,
        transformations,
        color_scale,
    }
}

enum WindowEvent {
    None,
    /// Another frame is shown.
    Frame,
    /// Settings changed that affect every frame.
    Settings,
    /// Export of the animation as a file of this format.
    Export(Format),
    Close,
}

/// Asks for the file and starts recording the frames of the animation of `view` in the order
/// of its playback; `None` when cancelled or FFmpeg is missing for a video.
fn start_animation_export(
    view: &mut ResultsView,
    format: Format,
    ffmpeg: PathBuf,
    output: &mut Vec<String>,
) -> Option<animation_export::Recording> {
    let animation = view.animation.as_mut()?;
    if format.needs_ffmpeg()
        && let Err(err) = animation_export::ffmpeg_version(&ffmpeg)
    {
        output.push(format!(
            "{err}. Exporting {} needs FFmpeg: install it or set its path under Tools > \
             Settings > Post-processing; GIF works without it.",
            format.extension().to_uppercase()
        ));
        return None;
    }
    let path = rfd::FileDialog::new()
        .set_title("Export Animation")
        .add_filter(format.filter_name(), &[format.extension()])
        .set_file_name(format!("animation.{}", format.extension()))
        .save_file()?;
    let path = if path.extension().is_none() {
        path.with_extension(format.extension())
    } else {
        path
    };
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let restore = (animation.frame, animation.playing);
    animation.playing = false;
    let sequence = animation_export::frame_sequence(animation.frame_count(), animation.playback);
    let encoder = animation_export::Encoder::start(animation_export::Job {
        format,
        path,
        fps: animation.fps,
        repeat: animation.playback != Playback::Once,
        ffmpeg,
    });
    Some(animation_export::Recording::new(
        sequence, encoder, file, restore,
    ))
}

/// PrePoMax's animation dialog: kind, frames, speed, playback mode, colour limits and the
/// player controls.
/// `exporting` greys out the export button while an exported animation is being written.
fn animation_window(ctx: &egui::Context, view: &mut ResultsView, exporting: bool) -> WindowEvent {
    let mut event = WindowEvent::None;
    let mut open = true;
    let mut restart = None;
    egui::Window::new("Animation")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .default_pos(ctx.content_rect().right_bottom() + egui::vec2(-130.0, -230.0))
        .show(ctx, |ui| {
            let Some(animation) = &mut view.animation else {
                return;
            };
            egui::Grid::new("animation settings")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Type");
                    ui.horizontal(|ui| {
                        for (kind, label) in [
                            (AnimationKind::ScaleFactor, "Scale factor"),
                            (AnimationKind::Increments, "Step increments"),
                            (AnimationKind::Steps, "Steps"),
                        ] {
                            if ui.radio(animation.kind == kind, label).clicked()
                                && animation.kind != kind
                            {
                                restart = Some(kind);
                            }
                        }
                    });
                    ui.end_row();
                    if animation.kind == AnimationKind::ScaleFactor {
                        ui.label("Frames");
                        let frames = numeric::drag_value(&mut animation.frames).range(2..=200);
                        if ui.add(frames).changed() {
                            animation.go_to(animation.frame);
                            event = WindowEvent::Settings;
                        }
                        ui.end_row();
                    }
                    ui.label("Frames per second");
                    ui.add(numeric::drag_value(&mut animation.fps).range(1.0..=60.0));
                    ui.end_row();
                    ui.label("Playback");
                    ui.horizontal(|ui| {
                        for (playback, label) in [
                            (Playback::Once, "Once"),
                            (Playback::Loop, "Loop"),
                            (Playback::Swing, "Swing"),
                        ] {
                            ui.radio_value(&mut animation.playback, playback, label);
                        }
                    });
                    ui.end_row();
                    ui.label("Color scale");
                    ui.horizontal(|ui| {
                        for (limits, label) in [
                            (ColorLimits::CurrentFrame, "Current frame"),
                            (ColorLimits::AllFrames, "All frames"),
                        ] {
                            if ui
                                .radio_value(&mut animation.limits, limits, label)
                                .changed()
                            {
                                event = WindowEvent::Settings;
                            }
                        }
                    });
                    ui.end_row();
                });
            ui.separator();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                let last = animation.frame_count() - 1;
                let frame = animation.frame;
                let mut target = None;
                if icons::button(ui, Icon::First, "First Frame", frame > 0, false).clicked() {
                    target = Some(0);
                }
                if icons::button(ui, Icon::Previous, "Previous Frame", frame > 0, false).clicked() {
                    target = Some(frame - 1);
                }
                let (icon, tip) = if animation.playing {
                    (Icon::Pause, "Pause")
                } else {
                    (Icon::Animate, "Play")
                };
                if icons::button(ui, icon, tip, true, false).clicked() {
                    if animation.playing {
                        animation.playing = false;
                    } else {
                        animation.play();
                    }
                }
                if icons::button(ui, Icon::Next, "Next Frame", frame < last, false).clicked() {
                    target = Some(frame + 1);
                }
                if icons::button(ui, Icon::Last, "Last Frame", frame < last, false).clicked() {
                    target = Some(last);
                }
                ui.add_space(8.0);
                let mut slider = frame;
                let response = ui.add(egui::Slider::new(&mut slider, 0..=last).show_value(false));
                if response.changed() {
                    target = Some(slider);
                }
                ui.add_space(8.0);
                ui.label(format!("Frame {} of {}", frame + 1, last + 1));
                ui.add_space(8.0);
                let tip = if exporting {
                    "Export Animation (the last export is still being written)"
                } else {
                    "Export Animation"
                };
                let export = icons::button(ui, Icon::Save, tip, !exporting, false);
                egui::Popup::menu(&export).show(|ui| {
                    for format in Format::ALL {
                        if ui.button(format.menu_label()).clicked() {
                            event = WindowEvent::Export(format);
                        }
                    }
                });
                if let Some(target) = target {
                    animation.playing = false;
                    if animation.go_to(target) && matches!(event, WindowEvent::None) {
                        event = WindowEvent::Frame;
                    }
                }
            });
        });
    if let Some(kind) = restart {
        view.start_animation(kind);
        event = WindowEvent::Settings;
    }
    if !open {
        event = WindowEvent::Close;
    }
    event
}

/// Prefixes an output message with the local time, `HH:MM:SS`.
fn timestamped(message: String) -> String {
    format!("{} {message}", chrono::Local::now().format("%H:%M:%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn import_into_saved_new_model_keeps_the_project_file() {
        let saved = Model::new(
            std::path::Path::new("/tmp/neu.plx"),
            plx_mesh::FeMesh::default(),
        );
        let mut imported = Model::new(
            std::path::Path::new("/tmp/teil.step"),
            plx_mesh::FeMesh::default(),
        );
        take_over_empty_model(&mut imported, &saved);
        assert!(imported.is_project());
        assert_eq!(imported.path, saved.path);
    }

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn the_project_stored_at_exit_is_read_back() {
        let path = std::env::temp_dir().join("prepolix_test_last_project.plx");
        std::fs::write(&path, "").unwrap();
        let mut storage = MemoryStorage::default();
        eframe::set_value(&mut storage, LAST_PROJECT_KEY, &Some(path.clone()));
        assert_eq!(stored_last_project(&storage), Some(path.clone()));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(stored_last_project(&storage), None);
        eframe::set_value(&mut storage, LAST_PROJECT_KEY, &None::<PathBuf>);
        assert_eq!(stored_last_project(&storage), None);
        assert_eq!(stored_last_project(&MemoryStorage::default()), None);
    }
}
