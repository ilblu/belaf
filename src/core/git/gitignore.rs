use crate::error::Result;
use std::fs;
use std::path::Path;

const GITIGNORE_ENTRIES: &str = r#"
# Belaf
belaf/.branches
"#;

pub fn update(project_root: &Path) -> Result<()> {
    if !project_root.join(".git").exists() {
        return Ok(());
    }

    let gitignore_path = project_root.join(".gitignore");

    let mut content = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    if content.contains("# Belaf") {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(GITIGNORE_ENTRIES);

    fs::write(&gitignore_path, content)?;
    Ok(())
}

pub fn is_git_repo(project_root: &Path) -> bool {
    project_root.join(".git").exists()
}
