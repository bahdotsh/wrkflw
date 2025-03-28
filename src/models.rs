pub struct ValidationResult {
    pub is_valid: bool,
    pub issues: Vec<String>,
}

impl ValidationResult {
    pub fn new() -> Self {
        ValidationResult {
            is_valid: true,
            issues: Vec::new(),
        }
    }

    pub fn add_issue(&mut self, issue: String) {
        self.is_valid = false;
        self.issues.push(issue);
    }
}
