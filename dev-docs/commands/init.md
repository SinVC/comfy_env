# Initialize a project

[Implementation](../../src/application/init.rs)

```
gov init [--comfyui-dir <absolute-path>] [--python <selector>]
```

Both flags are required on first initialization. Subsequent invocations use
the configured values for omitted flags. The selector can be a Python minor,
interpreter name, or path. `uv` must be available.

Prepare the configuration from existing values or `config.toml.template`.
Prepare the project from existing truth or `pyproject.toml.template`. Set
`requires-python` to the resolved minor series and `tool.uv.environments` to
the host platform and architecture. Lock the staged project before writing
production. Then create the standard state and cache directories, initialize
the registry if missing, and sync production against the locked truth.

On first init, a failure before the lock succeeds leaves the root truth files
unwritten. If sync fails after writing the configuration and locked truth,
retry init to finish the bootstrap.

For an initialized project with configuration and locked truth, reinit checks
production is stopped and uses the shared production commit. It backs up the
old configuration, truth, and registry. If sync fails, restore these files and
sync using the previous interpreter configuration. An interrupted operation can
be recovered with [op recover](ops.md).
