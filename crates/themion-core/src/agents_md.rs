use std::fs;
use std::path::Path;

pub const AGENTS_MD_FILENAME: &str = "AGENTS.md";
pub const AGENTS_MD_START_MARKER: &str = "# AGENTS.md instructions for ";
pub const AGENTS_MD_END_MARKER: &str = "</INSTRUCTIONS>";

pub fn load_root_agents_md(project_dir: &Path) -> Option<String> {
    let path = project_dir.join(AGENTS_MD_FILENAME);
    let contents = fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub fn build_agents_md_message(project_dir: &Path) -> Option<String> {
    let contents = load_root_agents_md(project_dir)?;
    Some(format!(
        "{AGENTS_MD_START_MARKER}{}\n\n<INSTRUCTIONS>\n{}\n{AGENTS_MD_END_MARKER}",
        project_dir.display(),
        contents
    ))
}
