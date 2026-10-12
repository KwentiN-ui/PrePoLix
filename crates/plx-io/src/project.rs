//! Project files (`.plx`): the mesh and the FE model in RON, written by serde, and the
//! results the user bound to the analysis.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use plx_mesh::FeMesh;
use plx_model::{FeModel, Geometry, PROJECT_FORMAT};
use plx_results::ResultsSettings;
use serde::{Deserialize, Serialize};

/// A prepolix project as saved in a `.plx` file: the mesh and the analysis set up on it.
/// Like PrePoMax's `.pmx` it stores what the user defined, not the CalculiX input file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub format: u32,
    /// CAD geometry the mesh is generated from; projects made from an input file have none.
    #[serde(default)]
    pub geometry: Option<Geometry>,
    pub mesh: FeMesh,
    #[serde(default)]
    pub model: FeModel,
    /// Results of a finished analysis the user bound to the project, restored when it is
    /// opened; projects without them are only mesh and definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<BoundResults>,
}

/// The files of a finished analysis kept in the project file: the input file CalculiX ran,
/// its results (`.frd`) and history output (`.dat`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BoundResults {
    /// The analysis the results belong to.
    pub analysis: String,
    /// CalculiX reported errors but wrote results.
    #[serde(default)]
    pub with_errors: bool,
    pub files: Vec<BoundFile>,
    /// What the user defined on the results, restored with them; as of the last save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<ResultsSettings>,
}

/// A file of [`BoundResults`]: its name and its content, deflated and base64 encoded so that
/// it is a compact string in the project's RON.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundFile {
    pub name: String,
    /// Size of the file itself.
    pub size: u64,
    pub data: String,
}

impl BoundResults {
    /// Size of the files as stored in the project.
    pub fn stored_size(&self) -> u64 {
        self.files.iter().map(|f| f.data.len() as u64).sum()
    }

    /// Size of the files themselves.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: not a valid project file: {message}")]
    Format { path: PathBuf, message: String },
    #[error("{path} comes from a newer PrePoLix version (format {format})")]
    Newer { path: PathBuf, format: u32 },
}

/// Writes the project; a crash while saving leaves an existing file intact.
pub fn save_project(
    path: &Path,
    geometry: Option<&Geometry>,
    mesh: &FeMesh,
    model: &FeModel,
    results: Option<&BoundResults>,
) -> Result<(), ProjectError> {
    let project = Project {
        format: PROJECT_FORMAT,
        geometry: geometry.cloned(),
        mesh: mesh.clone(),
        model: model.clone(),
        results: results.cloned(),
    };
    let text = ron::to_string(&project).map_err(|e| ProjectError::Format {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    let io = |source| ProjectError::Io {
        path: path.to_owned(),
        source,
    };
    let temporary = path.with_extension("plx.tmp");
    std::fs::write(&temporary, text).map_err(io)?;
    std::fs::rename(&temporary, path).map_err(io)
}

pub fn read_project(path: &Path) -> Result<Project, ProjectError> {
    let text = std::fs::read_to_string(path).map_err(|source| ProjectError::Io {
        path: path.to_owned(),
        source,
    })?;
    let mut project: Project = ron::from_str(&text).map_err(|e| ProjectError::Format {
        path: path.to_owned(),
        message: e.to_string(),
    })?;
    if project.format > PROJECT_FORMAT {
        return Err(ProjectError::Newer {
            path: path.to_owned(),
            format: project.format,
        });
    }
    project.model.migrate();
    Ok(project)
}

/// Reads the files of a finished analysis into [`BoundResults`]; files that do not exist are
/// left out, the results file has to be there.
pub fn bind_results(
    analysis: &str,
    files: &[PathBuf],
    with_errors: bool,
) -> Result<BoundResults, ProjectError> {
    let mut bound = BoundResults {
        analysis: analysis.to_owned(),
        with_errors,
        files: Vec::new(),
        settings: None,
    };
    for path in files {
        let io = |source| ProjectError::Io {
            path: path.to_owned(),
            source,
        };
        let content = match std::fs::read(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !is_results_file(path) => {
                continue;
            }
            Err(e) => return Err(io(e)),
        };
        // Level 3: a third of the time of the default level for a few percent more.
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(3));
        encoder.write_all(&content).map_err(io)?;
        let compressed = encoder.finish().map_err(io)?;
        bound.files.push(BoundFile {
            name: file_name(path).unwrap_or_default(),
            size: content.len() as u64,
            data: BASE64.encode(compressed),
        });
    }
    Ok(bound)
}

/// Writes the files of bound results into `dir`; returns the results file (`.frd`).
pub fn restore_results(bound: &BoundResults, dir: &Path) -> Result<PathBuf, ProjectError> {
    let format = |message: String| ProjectError::Format {
        path: dir.to_owned(),
        message,
    };
    let io = |path: &Path, source| ProjectError::Io {
        path: path.to_owned(),
        source,
    };
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let mut results = None;
    for file in &bound.files {
        // Only a plain file name: a project cannot write elsewhere.
        let name = Path::new(&file.name);
        if file_name(name).as_deref() != Some(file.name.as_str()) {
            return Err(format(format!(
                "bound file name {:?} is not valid",
                file.name
            )));
        }
        let compressed = BASE64
            .decode(&file.data)
            .map_err(|e| format(format!("bound file {}: {e}", file.name)))?;
        let mut content = Vec::with_capacity(file.size as usize);
        DeflateDecoder::new(compressed.as_slice())
            .read_to_end(&mut content)
            .map_err(|e| format(format!("bound file {}: {e}", file.name)))?;
        let path = dir.join(name);
        std::fs::write(&path, content).map_err(|e| io(&path, e))?;
        if is_results_file(&path) {
            results = Some(path);
        }
    }
    results.ok_or_else(|| format("the bound results have no results file (.frd)".into()))
}

fn is_results_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("frd"))
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use plx_model::{
        Algorithm2d, Algorithm3d, Constraint, ContactPair, Friction, InteractionProperty, Material,
        MeshSetupItem, MeshSetupKind, MeshingParameters, Region, Step, SurfaceBehavior,
        SurfaceInteraction, Tie, UserKeyword,
    };

    use super::*;
    use crate::inp::read_inp;

    #[test]
    fn projects_read_back_what_was_saved() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mesh = read_inp(&root.join("testdata/block_c3d20r.inp"))
            .unwrap()
            .mesh;
        let model = FeModel {
            properties: Default::default(),
            materials: vec![Material {
                name: "Steel".into(),
                density: Some(7.85e-9),
                elastic: None,
                ..Default::default()
            }],
            sections: Vec::new(),
            steps: vec![Step::new_static("Step-1"), Step::new_coupled("Step-2")],
            user_keywords: vec![UserKeyword {
                position: vec![14, 0],
                text: "*Amplitude, Name=A\n0, 0, 1, 1".into(),
                active: false,
            }],
            reference_points: vec![plx_model::ReferencePoint::new("RP-1", [1.0, 2.0, 3.0])],
            coordinate_systems: vec![plx_model::CoordinateSystem {
                kind: plx_model::CoordinateSystemKind::Cylindrical,
                ..plx_model::CoordinateSystem::new("Coordinate_System-1")
            }],
            planes: vec![plx_model::Plane {
                source: plx_model::PlaneSource::ThreePoints {
                    points: [
                        plx_model::PointRef::ReferencePoint("RP-1".into()),
                        plx_model::PointRef::Coordinates([1.0, 0.0, 0.0]),
                        plx_model::PointRef::Coordinates([0.0, 1.0, 0.0]),
                    ],
                },
                ..plx_model::Plane::new("Plane-1")
            }],
            result_planes: vec![plx_model::ResultPlane {
                name: "Plane_Result-1".into(),
                plane: "Plane-1".into(),
            }],
            result_paths: vec![plx_model::ResultPath {
                start: plx_model::PointRef::ReferencePoint("RP-1".into()),
                ..plx_model::ResultPath::new("Path-1")
            }],
            mesh_sets: vec![plx_model::MeshSet::new(
                "Corner",
                plx_model::SetKind::Nodes,
                Region::Nodes(vec![1]),
            )],
            element_types: vec![plx_model::PartElementType {
                part: "EALL".into(),
                type_name: "C3D20".into(),
            }],
            constraints: Vec::new(),
            ties: vec![Tie {
                master: Region::Faces(vec![(1, 2)]),
                slave: Region::Surface("TOP".into()),
                ..Tie::new("Tie-1")
            }],
            node_ties: vec![plx_model::NodeTie {
                region: Region::Nodes(vec![1, 2]),
                rotations: false,
                ..plx_model::NodeTie::new("Node_Tie-1")
            }],
            surface_interactions: vec![SurfaceInteraction {
                name: "Surface_Interaction-1".into(),
                properties: vec![
                    InteractionProperty::SurfaceBehavior(SurfaceBehavior::Tabular(vec![
                        [0.0, 0.0],
                        [1e5, 1.0],
                    ])),
                    InteractionProperty::Friction(Friction::default()),
                ],
            }],
            contact_pairs: vec![ContactPair {
                adjustment_size: Some(0.01),
                master_color: [255, 0, 0],
                ..ContactPair::new("Contact_Pair-1", "Surface_Interaction-1")
            }],
            amplitudes: vec![plx_model::Amplitude {
                time_span: plx_model::AmplitudeTime::Total,
                points: vec![[0.0, 0.0], [1.0, 2.5]],
                ..plx_model::Amplitude::new("Amplitude-1")
            }],
            initial_conditions: vec![plx_model::InitialCondition {
                name: "Initial_Temperature-1".into(),
                active: true,
                region: Region::Parts(vec!["A".into()]),
                kind: plx_model::InitialConditionKind::Temperature(20.0),
            }],
        };
        let dir = std::env::temp_dir().join(format!("plx-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("block.plx");
        let geometry = Geometry {
            source: "block.step".into(),
            brep: "DBRep_DrawableShape\n\"quoted\" lines\n".into(),
            meshing: MeshingParameters::for_diagonal(100.0),
            mesh_items: vec![
                MeshSetupItem {
                    name: "Local_Mesh_Size-1".into(),
                    kind: MeshSetupKind::LocalMeshSize {
                        faces: vec![3, 4],
                        edges: vec![7],
                        size: 0.5,
                    },
                },
                MeshSetupItem {
                    name: "Tetrahedral_Gmsh-1".into(),
                    kind: MeshSetupKind::TetrahedralGmsh {
                        parts: vec!["SOLID-1".into()],
                        algorithm_2d: Algorithm2d::Delaunay,
                        algorithm_3d: Algorithm3d::Hxt,
                    },
                },
                MeshSetupItem {
                    name: "Extrude_Mesh-1".into(),
                    kind: MeshSetupKind::ExtrudeMesh(plx_model::ExtrudeMesh {
                        parts: vec!["SOLID-1".into()],
                        faces: vec![2],
                        layers: plx_model::ExtrudeLayers::MultiLayered {
                            sizes: vec![0.2, 0.8],
                            elements: vec![1, 3],
                        },
                        ..Default::default()
                    }),
                },
            ],
            part_names: vec!["SOLID-1".into()],
        };
        save_project(&path, Some(&geometry), &mesh, &model, None).unwrap();
        let project = read_project(&path).unwrap();
        assert_eq!(project.model, model);
        assert_eq!(project.geometry, Some(geometry));
        assert_eq!(project.mesh.elements(), mesh.elements());
        assert_eq!(project.mesh.coords(), mesh.coords());
        assert_eq!(project.mesh.node_sets, mesh.node_sets);

        let newer = std::fs::read_to_string(&path).unwrap().replacen(
            &format!("format:{PROJECT_FORMAT}"),
            "format:99",
            1,
        );
        std::fs::write(&path, newer).unwrap();
        assert!(matches!(
            read_project(&path),
            Err(ProjectError::Newer { format: 99, .. })
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn steps_of_older_projects_are_active() {
        let mut step = Step::new_static("Step-1");
        step.active = false;
        let text = ron::to_string(&step).unwrap();
        assert!(text.contains("active:false,"), "{text}");
        let older: Step = ron::from_str(&text.replace("active:false,", "")).unwrap();
        assert!(older.active);
    }

    #[test]
    fn ties_of_older_projects_move_to_the_contact_pairs() {
        // Ties and node ties were constraints; a project saved then still opens with them.
        let mut tie = plx_model::NodeTie::new("Node_Tie-1");
        tie.region = Region::Nodes(vec![3, 7]);
        let older = FeModel {
            constraints: vec![
                Constraint::Tie(Tie::new("Tie-1")),
                Constraint::NodeTie(tie.clone()),
            ],
            ..FeModel::default()
        };
        let mut model: FeModel = ron::from_str(&ron::to_string(&older).unwrap()).unwrap();
        model.migrate();
        assert_eq!(model.constraints, []);
        assert_eq!(model.ties, [Tie::new("Tie-1")]);
        assert_eq!(model.node_ties, [tie]);
    }

    #[test]
    fn analyses_are_saved_and_older_projects_get_the_first() {
        let mut model = FeModel::default();
        let mut second = plx_model::AnalysisJob::new("Bracket");
        second.set_arguments("-i Bracket");
        model.analyses.push(second);
        let text = ron::to_string(&model).unwrap();
        let read: FeModel = ron::from_str(&text).unwrap();
        assert_eq!(read.analyses, model.analyses);
        // Projects saved before analyses were part of the model have Analysis-1.
        let start = text.find("analyses:").unwrap();
        let end = start + text[start..].find(']').unwrap() + 1;
        let older = format!("{}{}", &text[..start], &text[end..]).replace(",)", ")");
        let read: FeModel = ron::from_str(&older).unwrap();
        assert_eq!(read.analyses.len(), 1);
        assert_eq!(read.analyses[0].name, plx_model::FIRST_ANALYSIS);
        // All analyses deleted stays so.
        model.analyses.clear();
        let read: FeModel = ron::from_str(&ron::to_string(&model).unwrap()).unwrap();
        assert!(read.analyses.is_empty());
    }

    #[test]
    fn hot_spots_of_older_projects_are_skipped() {
        // Hot spots were part of the FE model for a while; they are defined on the results
        // now, and projects saved with them still open.
        let text = ron::to_string(&FeModel::default()).unwrap();
        let hot_spots = "hot_spots:[(name:\"Hot_Spot-1\",toe:Nodes([3,7]),\
            direction:(1.0,0.0,0.0),thickness:10.0,extrapolation:Custom([2.0,6.0]),\
            component:Perpendicular)],";
        let older = text.replacen("user_keywords:", &format!("{hot_spots}user_keywords:"), 1);
        assert_ne!(older, text);
        let model: FeModel = ron::from_str(&older).unwrap();
        assert_eq!(model, FeModel::default());
    }

    #[test]
    fn path_plots_saved_before_hot_spot_plots_still_load() {
        let older = "(path:\"Path-1\",offset:None,size:(400.0,240.0),on_path:false,\
            height:80.0,title:\"\",x_label:\"Distance\",y_label:\"\",y_range:None)";
        let plot: plx_results::saved_view::SavedPathPlot = ron::from_str(older).unwrap();
        assert!(!plot.hot_spot);
        let saved = ron::to_string(&plot).unwrap();
        assert_eq!(
            ron::from_str::<plx_results::saved_view::SavedPathPlot>(&saved).unwrap(),
            plot
        );
    }

    #[test]
    fn table_plots_keep_missing_values() {
        let plot = plx_results::saved_view::SavedPathPlot {
            path: "SP_A.STRESSES.MISES".into(),
            offset: Some([20.0, 30.0]),
            size: [400.0, 240.0],
            on_path: false,
            height: 80.0,
            title: String::new(),
            x_label: "Increment".into(),
            y_label: String::new(),
            y_range: None,
            hot_spot: false,
            table: Some(plx_results::saved_view::SavedTablePlot {
                x_label: "Increment".into(),
                x: vec![1.0, 2.0, 3.0],
                curves: vec![("12_1".into(), vec![0.5, f64::NAN, 1.5])],
            }),
        };
        let saved = ron::to_string(&plot).unwrap();
        let loaded: plx_results::saved_view::SavedPathPlot = ron::from_str(&saved).unwrap();
        let table = loaded.table.unwrap();
        assert_eq!(table.x, [1.0, 2.0, 3.0]);
        let values = &table.curves[0].1;
        assert!(values[0] == 0.5 && values[1].is_nan() && values[2] == 1.5);
    }

    #[test]
    fn bound_results_are_restored_from_the_project() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mesh = read_inp(&root.join("testdata/block_c3d20r.inp"))
            .unwrap()
            .mesh;
        let dir = std::env::temp_dir().join(format!("plx-bound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frd = dir.join("Analysis-1.frd");
        let text = "    1C\n".repeat(1000) + " 9999\n";
        std::fs::write(&frd, &text).unwrap();
        // No history output: the missing .dat is left out.
        let files = [frd.with_extension("dat"), frd.clone()];
        let bound = bind_results("Analysis-1", &files, false).unwrap();
        assert_eq!(bound.files.len(), 1);
        assert_eq!(bound.size(), text.len() as u64);
        assert!(
            bound.stored_size() < bound.size() / 10,
            "{}",
            bound.stored_size()
        );
        assert!(bind_results("Analysis-1", &[dir.join("missing.frd")], false).is_err());

        let path = dir.join("block.plx");
        let model = FeModel::default();
        let mut bound = bound;
        bound.settings = Some(ResultsSettings {
            result_paths: vec![plx_model::ResultPath::new("Path-1")],
            transformations: vec![plx_results::Transformation::symmetry(
                plx_results::transformation::SymmetryPlane::Y,
            )],
            shown: Some(("STRESS".into(), "MISES".into())),
            // The colour scale the results were shown with.
            view: plx_results::saved_view::SavedView {
                color_scale: Some(plx_model::ColorScale {
                    spectrum: plx_model::ColorSpectrum::Viridis,
                    manual: Some([-1.5, 250.0]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        });
        save_project(&path, None, &mesh, &model, Some(&bound)).unwrap();
        let project = read_project(&path).unwrap();
        assert_eq!(project.results.as_ref(), Some(&bound));

        let restored = restore_results(&bound, &dir.join("restored")).unwrap();
        assert_eq!(restored, dir.join("restored/Analysis-1.frd"));
        assert_eq!(std::fs::read_to_string(&restored).unwrap(), text);

        // Without bound results the project file has no trace of them.
        save_project(&path, None, &mesh, &model, None).unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("Analysis-1")
        );
        assert_eq!(read_project(&path).unwrap().results, None);

        let mut evil = bound.clone();
        evil.files[0].name = "../evil.frd".into();
        assert!(restore_results(&evil, &dir.join("restored")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
