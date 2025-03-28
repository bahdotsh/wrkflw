use std::path::Path;

pub fn is_workflow_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        if ext == "yml" || ext == "yaml" {
            // Check if the file is in a .github/workflows directory
            if let Some(parent) = path.parent() {
                return parent.ends_with(".github/workflows") || parent.ends_with("workflows");
            } else {
                // Check if filename contains workflow indicators
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_lowercase())
                    .unwrap_or_default();

                return filename.contains("workflow")
                    || filename.contains("action")
                    || filename.contains("ci")
                    || filename.contains("cd");
            }
        }
    }
    false
}
