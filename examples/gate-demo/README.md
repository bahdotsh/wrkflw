# gate-demo

The workflow `demo.tape` records: a test that fails until `greet.sh` prints
`Hello, world!`, a lint job, and a `macos-latest` packaging job that runs as
Linux unless `--macos-on-host` runs it on a Mac.

```bash
cd examples/gate-demo
wrkflw run .github/workflows/ci.yml    # fails: greet.sh drops the "!"
```

The failing test is deliberate; the tape fixes it, then restores it.
