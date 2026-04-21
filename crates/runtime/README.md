## wrkflw-runtime

Runtime abstractions for executing steps in containers, on the host, or in a
local sandbox.

- Container management primitives used by the executor (Docker, Podman)
- Emulation mode helpers (run on host without containers)
- Secure emulation runtime: sandboxed host processes with filesystem and
  network restrictions for running untrusted workflows without a container
  runtime

### Example

```rust
// This crate is primarily consumed by `wrkflw-executor`.
// Prefer using the executor API instead of calling runtime directly.
```
