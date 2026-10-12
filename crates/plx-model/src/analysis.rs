//! The analysis of a model, PrePoMax's analysis job: its name, which is also the name of its
//! input and results files, and the arguments CalculiX is started with. A project has exactly
//! one; the executable and the work directory are settings of the program, as in PrePoMax.

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

/// Name of the analysis of a new model, PrePoMax's first default.
pub const FIRST_ANALYSIS: &str = "Analysis-1";

impl Default for AnalysisJob {
    fn default() -> Self {
        Self::new(FIRST_ANALYSIS)
    }
}

/// Why `name` cannot name an analysis: it names files in the work directory, which CalculiX
/// finds by the job name.
pub fn analysis_name_error(name: &str) -> Option<String> {
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
        assert!(analysis_name_error("Analysis-2").is_none());
        assert!(analysis_name_error("bracket_v2.1").is_none());
        assert!(analysis_name_error("").is_some());
        assert!(analysis_name_error("my job").is_some());
        assert!(analysis_name_error("a/b").is_some());
        assert!(analysis_name_error("-i").is_some());
        assert_eq!(AnalysisJob::default().name, FIRST_ANALYSIS);
    }
}
