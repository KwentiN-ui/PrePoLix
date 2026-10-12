#import "../../template.typ": *

= Running CalculiX <analysis>

== Analyses <analyses>

The analyses of a model are listed under #ui("Analyses") at the bottom of the #ui("FE Model")
tree. An analysis is a job CalculiX runs on the model; its name is also the name of its files in
the work directory (`<name>.inp`, `<name>.frd`, `<name>.dat`). A new model, and a project saved
before analyses were part of it, has one analysis, #ui("Analysis-1"). The analyses are saved with
the project, and creating, editing or deleting one can be undone.

Double-click #ui("Analyses"), or use #ui("Create ...") in its context menu or
#menu("Analysis", "Create Analysis ..."), to create an analysis. Double-click an analysis, or
use #ui("Edit ...") in its context menu, to edit or rename it.

#screenshot("analysis-dialog.png", [Editing an analysis])

#fields(
  [Name], [Name of the analysis and of its files: letters, digits, `-`, `_` and `.`, unique among
    the analyses. A running analysis cannot be renamed. A bound result (@bind-result) stays with
    the renamed analysis.],
  [FEM solver], [Always CalculiX.],
  [Executable], [The CalculiX executable from the settings, shown only.],
  [Executable arguments], [Command line arguments CalculiX is started with. The default is the
    name of the analysis, which CalculiX takes as job name, and it follows a rename. Change it
    only to run the solver differently; the job name must stay in the arguments.],
  [Work directory], [The work directory from the settings, shown only.],
  [Threads], [The number of threads from the settings (`OMP_NUM_THREADS`), shown only.],
)

The link #ui("CalculiX settings") in the dialog opens #menu("Tools", "Settings ...",
"CalculiX") (@settings), where the executable, the work directory and the threads are set for all
analyses, as in PrePoMax.

#ui("Duplicate") in the context menu of an analysis creates a copy under the next free name, and
#ui("Delete") (or the #key("Delete") key) removes it after a question. Deleting an analysis also
removes the result bound to it. A running analysis cannot be deleted.

== Running an analysis

#fields(
  [Run Analysis (#key("F5"))], [Writes `<name>.inp` to the work directory and starts CalculiX.
    Old result files of the analysis in that directory are deleted first.],
  [Check Model], [Lets CalculiX only read and check the model (`*NO ANALYSIS`), without solving
    it.],
  [Kill Analysis], [Stops a running analysis.],
  [Monitor], [Shows the monitor window.],
  [Open Results], [Opens the result file of the last run and binds it to the project
    (@bind-result).],
)

These commands are in the context menu of each analysis and in the #ui("Analysis") menu. The
menu runs the analysis selected in the tree, otherwise the one run last, otherwise the first; its
#ui("Run Analysis") entry names it. One analysis runs at a time. The status icon of an analysis
shows its last run or its bound result.
Before CalculiX starts, PrePoLix refuses to run a model without mesh, without step or with all
steps deactivated, and it stops if the input file cannot be written (for example because a region
is empty).

=== Monitor

The monitor opens when a run starts. It shows the status and running time, the current step,
increment and iteration, and the output of CalculiX. #ui("Results") opens the results when the run
is finished.

#screenshot("monitor.png", [Monitor after a completed run])

The #ui("Filter") field above the output shows only the lines that contain the text you type,
ignoring upper and lower case. With "average force", for example, you see the average force of
every increment one below the other. The filter applies while CalculiX is still running, and
#ui("Clear") or an empty field shows all lines again.

#screenshot("monitor-filter.png", [Monitor showing only the lines with "average force"])

The status at the end is #ui("completed"), #ui("finished with errors, results available"),
#ui("failed") or #ui("killed"). If CalculiX failed, PrePoLix looks for known error messages
in the output and adds a line "Possible cause: ..." with a remedy, for example for distorted
elements, missing material data, rigid body motion or missing convergence.

== Results saved with the project <bind-result>

When you open the results of a run (#ui("Results") in the monitor or in the context menu of
the analysis, or #menu("Analysis", "Open Results")), PrePoLix binds them to the project: the
input file, the result file (`.frd`) and the history output (`.dat`) of the run are saved in the
project file with the next #menu("File", "Save"). The output pane shows how much the files add to
the project file; they are stored compressed, at about a third of their size (a 236 MB result file
adds about 100 MB). Binding a large result takes a few seconds and runs in the background. A new
run replaces the bound result as soon as you open its results.

When a project with a bound result is opened, PrePoLix writes the files to
`Bound_Results/<project name>` in the work directory and opens the result on the #ui("Results")
tab. The analysis the result belongs to then shows the status of the bound run, its tooltip says that a result is
bound, and #ui("Results") in its context menu opens the bound result again.

What you defined on the bound result is saved with it and comes back as you left it: derived
field and history outputs, transformations, hot spots, the reference points, coordinate systems
and planes of the results, result paths and plane results, and the increment, field and component
shown. The view comes back too: the camera of the #ui("Results") tab, #menu("View", "Mesh Edges"),
the deformation and its factor, #ui("Show Undeformed"), the color levels, hidden parts, the
annotations of the query tool and the plots of result paths. PrePoLix takes all of this from the
open result each time the project is saved.

#screenshot("bound-result-restored.png", [A bound result reopened with its symmetry
  transformation and the component shown when the project was saved])

#screenshot("bind-result.png", [Context menu of an analysis with a bound result])

When the analysis runs again, its new results take over all of this from the results before,
whether they were bound or only open, so the view, transformations, paths and plots stay as they
were. Annotations of the query tool and hot spots on picked nodes or faces refer to nodes and
elements of the mesh; they are kept only if the mesh did not change. After remeshing they are
dropped, reference points placed on picked nodes or faces stay at their last position, and the
output pane lists what was dropped.

#screenshot("rerun-keeps-view.png", [The results of a new run, shown from the front without mesh
  edges like the results before])

#ui("Remove Bound Result") in the context menu of the analysis removes the bound result;
save the project to drop it from the file. Opening the results of the same run again does not bind
them again; the next run does.

To send only the mesh and the model definition, use #menu("File", "Save Without Results ...").
It saves a copy without the bound result under a new name (proposed as
`<project name>-without-results.plx`); the open project stays on its own file and keeps its
result. The entry is available while a result is bound.

== Model check <model-check>

PrePoLix checks the model continuously while you work. Items with a problem get a warning sign in
the tree; items with an error and their containers are shown in red. Click the warning sign to see
what is wrong and how to fix it.

#screenshot("model-check.png", [Model check: parts without a section])

Errors include elements without material, materials without the data the step needs (elasticity,
density, conductivity, specific heat), invalid material constants, distorted elements, a missing
initial temperature in a transient heat transfer step, an increment larger than the step, rigid
body motion of a part in a static step, truss structures that are mechanisms and missing
references (a deleted material, part or set that an item still uses). Warnings include parts held
only by contact, conflicting boundary conditions, loads on fixed nodes, rotations without effect
and steps without load.

When a run starts, PrePoLix also checks that the configured CalculiX knows every keyword of the
input file (see #ui("Test CalculiX") in @settings). An unknown keyword, for example of a step
type that an older CalculiX lacks, appears as error "Keyword unknown to CalculiX" on the
analysis and in the monitor, with the feature that needs it.

The checks do not stop a run; their errors are listed at the top of the monitor. Missing
references do stop it, because the input file cannot be written.

== Exporting the input file <export-inp>

#menu("File", "Export CalculiX Input File ...") writes the same input file as a run, for example
to run CalculiX by hand or on another computer. Deactivated items appear as comments.

== Submodels <submodel>

A submodel computes a detail of a larger model more precisely. Its cut boundaries follow the
displacements of the global model.

+ Compute the global model and keep its result file (`.frd`).
+ In the submodel, open #menu("Model", "Model Properties ...") and set #ui("Model Type") to
  #ui("Submodel"). Choose the global result file under #ui("Global results .frd").
+ In a static step, create a boundary condition of type #ui("Submodel") on the cut faces. Choose
  the #ui("Global step") and the displacement components.
+ Run the analysis. PrePoLix copies the global results next to the input file and writes
  `*SUBMODEL` and `*BOUNDARY, SUBMODEL`.

#limitation[Only displacements can be transferred. The global result file must not be the result
file of the submodel's own analysis.]

== Exporting a deformed mesh <export-deformed>

#menu("File", "Export Deformed Mesh (.inp) ...") writes the mesh of the displayed result with its
nodes moved by the displacements times the current deformation scale factor. A typical use is an
imperfection for a buckling analysis: show the buckling mode, set #ui("Deformation") to #ui("User
defined") with the imperfection amplitude as factor, export, and import the file into the model with
#ui("Import Geometry") (@mesh-import). The file contains only the mesh.
