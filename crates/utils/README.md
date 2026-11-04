## wrkflw-utils

Shared helpers used across crates.

- Workflow file detection (`.gitea/workflows/*.yml`, `.github/workflows/*.yml`, `.gitlab-ci.yml`)
- File-descriptor redirection utilities for silencing noisy subprocess output (Unix only; Windows support is limited)

### Example

```rust
use std::path::Path;
use wrkflw_utils::{is_workflow_file, fd::with_stderr_to_null};

assert!(is_workflow_file(Path::new(".github/workflows/ci.yml")));

let value = with_stderr_to_null(|| {
    eprintln!("this is hidden on Unix, visible on Windows");
    42
}).unwrap();
assert_eq!(value, 42);
```
