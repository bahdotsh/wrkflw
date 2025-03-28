use std::path::Path;

pub fn is_workflow_file(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        if ext == "yml" || ext == "yaml" {
            // Check if the file is in a .github/workflows directory
            // Or accept any YAML file if specifically chosen
            if let Some(parent) = path.parent() {
                return parent.ends_with(".github/workflows")
                    || path.to_string_lossy().contains("workflow");
            }
        }
    }
    false
}
