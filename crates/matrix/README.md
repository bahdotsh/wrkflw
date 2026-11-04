## wrkflw-matrix

Matrix expansion utilities used to compute all job combinations and format labels.

- Supports `include`, `exclude`, `max-parallel`, and `fail-fast`
- Keeps a matrix, an axis or `include`/`exclude` given as an expression (`${{ fromJSON(...) }}`) for the executor to resolve at run time; `expand_matrix` reports such a matrix as an error
- Provides display helpers for UI/CLI

### Example

```rust
use wrkflw_matrix::{MatrixConfig, expand_matrix};
use serde_yaml::Value;
use std::collections::HashMap;

let mut cfg = MatrixConfig::default();
cfg.parameters.insert("os".into(), Value::from(vec!["ubuntu", "alpine"])) ;

let combos = expand_matrix(&cfg).expect("expand");
assert!(!combos.is_empty());
```
