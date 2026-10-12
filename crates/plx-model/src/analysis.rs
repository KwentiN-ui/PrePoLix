//! PrePoMax's analyses: the jobs CalculiX runs on the model, each with its name, which is
//! also the name of its input and results files, and the arguments CalculiX is started with.
//! The executable and the work directory are settings of the program, as in PrePoMax.

use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};

/// An analysis of the model, PrePoMax's `AnalysisJob`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalysisJob {
    /// Name of the job: CalculiX writes `<name>.inp`, `<name>.frd` and `<name>.dat`.
    pub name: String,
    /// Command line arguments of CalculiX, separated by spaces; `None` for the job name alone,
    /// PrePoMax's default. Kept apart so that the default follows a renamed analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

impl AnalysisJob {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            arguments: None,
        }
    }

    /// The arguments as they are typed: the job name unless the user changed them.
    pub fn arguments_text(&self) -> &str {
        self.arguments.as_deref().unwrap_or(&self.name)
    }

    /// Takes typed arguments; the job name itself is stored as the default.
    pub fn set_arguments(&mut self, text: &str) {
        let text = text.trim();
        self.arguments = (text != self.name && !text.is_empty()).then(|| text.to_owned());
    }

    /// The arguments CalculiX is started with.
    pub fn command_arguments(&self) -> Vec<String> {
        self.arguments_text()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
}

/// The analyses of a model. A new model, and a project saved before analyses were part of
/// it, has PrePoMax's first default `Analysis-1`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Analyses(pub Vec<AnalysisJob>);

/// Name of the first analysis of a model, PrePoMax's first default.
pub const FIRST_ANALYSIS: &str = "Analysis-1";

impl Default for Analyses {
    fn default() -> Self {
        Self(vec![AnalysisJob::new(FIRST_ANALYSIS)])
    }
}

impl Deref for Analyses {
    type Target = Vec<AnalysisJob>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Analyses {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Analyses {
    /// The analysis called `name`.
    pub fn position(&self, name: &str) -> Option<usize> {
        self.0.iter().position(|a| a.name == name)
    }

    /// Next free default name such as `Analysis-2`.
    pub fn next_name(&self) -> String {
        crate::next_name("Analysis", self.0.iter().map(|a| a.name.as_str()))
    }
}

/// Why `name` cannot name an analysis: it names files in the work directory, which CalculiX
/// finds by the job name. `others` are the names of the other analyses.
pub fn analysis_name_error<'a>(
    name: &str,
    others: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Some("The name must not be empty.".into());
    }
    if trimmed != name {
        return Some("The name must not start or end with a space.".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Some(format!(
            "The name names the job's files: use letters, digits, '-', '_' and '.' only, not '{c}'."
        ));
    }
    if name.starts_with(['-', '.']) {
        return Some("The name must not start with '-' or '.'.".into());
    }
    if others.into_iter().any(|o| o.eq_ignore_ascii_case(name)) {
        return Some(format!("An analysis named {name} exists already."));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_arguments_follow_the_name() {
        let mut job = AnalysisJob::new("Analysis-1");
        assert_eq!(job.command_arguments(), ["Analysis-1"]);
        job.name = "Bracket".into();
        assert_eq!(job.arguments_text(), "Bracket");
        job.set_arguments(" Bracket ");
        assert_eq!(job.arguments, None);
        job.set_arguments("-i Bracket");
        assert_eq!(job.command_arguments(), ["-i", "Bracket"]);
        job.set_arguments("");
        assert_eq!(job.arguments, None);
    }

    #[test]
    fn names_must_be_file_names() {
        let others = ["Analysis-1"];
        assert!(analysis_name_error("Analysis-2", others).is_none());
        assert!(analysis_name_error("bracket_v2.1", others).is_none());
        assert!(analysis_name_error("", others).is_some());
        assert!(analysis_name_error("my job", others).is_some());
        assert!(analysis_name_error("a/b", others).is_some());
        assert!(analysis_name_error("-i", others).is_some());
        assert!(analysis_name_error("analysis-1", others).is_some());
    }

    #[test]
    fn projects_without_analyses_get_the_first() {
        let analyses: Analyses = Default::default();
        assert_eq!(analyses.len(), 1);
        assert_eq!(analyses[0].name, FIRST_ANALYSIS);
        assert_eq!(analyses.next_name(), "Analysis-2");
    }
}
