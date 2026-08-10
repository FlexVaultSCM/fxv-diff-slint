//! The content this viewer browses, and the pretend host behind it.
//!
//! Everything here stands in for something a real application would have: a repository to read
//! diffs from, a way to name what it found, and a way to fetch lines it has not been given.

// == Internal Crates
use fxv_diff_slint::{FileChange, FileDiff, parse_unified_diff};

/// Diffs to browse, most taken from this repository's own history so they contain real code
/// rather than something contrived.
pub const SAMPLES: &[(&str, &str)] = &[
    (
        "Gaps and tabs",
        include_str!("../samples/synthetic_gaps.diff"),
    ),
    (
        "Add parsing (15 files)",
        include_str!("../samples/add_parsing_commit.diff"),
    ),
    (
        "Adds, renames, binary",
        include_str!("../samples/mixed_operations.diff"),
    ),
];

/// The right-hand file a sample's gaps can be opened from, where we have it. Without one the
/// gaps are still shown, but there is nothing to fill them with.
pub const SAMPLE_SOURCES: &[(&str, &str)] = &[(
    "Gaps and tabs",
    include_str!("../samples/synthetic_gaps.after.txt"),
)];

/// Plain files for the standalone tab, which shows a file with no diff behind it.
///
/// Two of the widget's own sources, so the content is real, plus the file the gaps sample was
/// taken against, which carries a tab and some lines wide enough to need scrolling.
pub const PLAIN_FILES: &[(&str, &str)] = &[
    (
        "synthetic_gaps.after.txt",
        include_str!("../samples/synthetic_gaps.after.txt"),
    ),
    (
        "span.rs",
        include_str!("../../../crates/fxv-diff-slint/src/span.rs"),
    ),
    (
        "selection.rs",
        include_str!("../../../crates/fxv-diff-slint/src/selection.rs"),
    ),
];

/// One entry of the diff picker: a single file, named together with the diff it came from.
///
/// The picker is flat rather than a sample chooser feeding a file chooser, so reaching a file is
/// one selection. That costs parsing every sample at startup to find out what is in it, which is
/// nothing at this size.
pub struct Choice {
    pub label: String,
    pub sample: usize,
    pub file: usize,
}

impl Choice {
    /// Lists every file of every sample as one entry, parsing each sample to find out what is
    /// in it.
    ///
    /// `qualify` names the sample as well as the file, which only helps when there is more than
    /// one sample to tell apart.
    pub fn list(samples: &[(String, String)], qualify: bool) -> Vec<Choice> {
        let mut choices = Vec::new();
        for (sample, (name, text)) in samples.iter().enumerate() {
            let Ok(parsed) = parse_unified_diff(text) else {
                // A sample that will not parse still deserves an entry, so selecting it shows
                // the parse error rather than the file silently not being in the list.
                choices.push(Choice {
                    label: name.clone(),
                    sample,
                    file: 0,
                });
                continue;
            };
            for (file, diff) in parsed.files.iter().enumerate() {
                let described = Choice::describe(diff);
                choices.push(Choice {
                    label: if qualify {
                        format!("{name}: {described}")
                    } else {
                        described
                    },
                    sample,
                    file,
                });
            }
        }
        choices
    }

    /// The file this choice's gaps can be filled from, if its sample came with one.
    pub fn source(&self) -> Option<&'static str> {
        let name = &SAMPLES.get(self.sample)?.0;
        SAMPLE_SOURCES
            .iter()
            .find(|(s, _)| s == name)
            .map(|(_, text)| *text)
    }

    /// A label for the picker, noting anything that happened to the file beyond an edit.
    fn describe(file: &FileDiff) -> String {
        let note = match file.change {
            FileChange::Added => "  [added]",
            FileChange::Removed => "  [removed]",
            FileChange::Renamed => "  [renamed]",
            FileChange::Copied => "  [copied]",
            FileChange::Modified if file.left_mode != file.right_mode => "  [mode]",
            FileChange::Modified => "",
        };
        let binary = if file.is_binary() { "  [binary]" } else { "" };
        format!("{}{note}{binary}", file.display_path())
    }
}

/// Reads a run of lines from the right-hand file, standing in for whatever a real host does.
///
/// Only some samples come with the file the diff was taken against, and a diff named on the
/// command line never does, so asking for lines that are not here is a normal outcome rather
/// than a bug. It is also the only way to reach the failure path the view has to show.
pub fn fetch(source: Option<&str>, right_start: u32, count: u32) -> Result<Vec<String>, String> {
    let source = source.ok_or("this sample was built without the file it came from")?;

    let lines: Vec<String> = source
        .lines()
        .skip(right_start.max(1) as usize - 1)
        .take(count as usize)
        .map(str::to_owned)
        .collect();

    if lines.len() as u32 != count {
        return Err(format!(
            "wanted {count} lines from {right_start}, found {}",
            lines.len()
        ));
    }
    Ok(lines)
}
