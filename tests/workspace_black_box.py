"""Exercise the release workspace layout with real Git and a local wheel index."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

from slice3_black_box import wheel, write, write_main


def run(cwd, env, *args, expected=0):
    result = subprocess.run(args, cwd=cwd, env=env, capture_output=True, text=True, timeout=120)
    output = result.stdout + result.stderr
    assert result.returncode == expected, (args, result.returncode, output)
    return result.stdout, output


def verify(root, binary):
    package = root / "package"
    package.mkdir()
    gov = Path(shutil.copy2(binary, package / binary.name))
    repo = Path(__file__).resolve().parents[1]
    for template in ("config.toml.template", "pyproject.toml.template"):
        shutil.copy2(repo / template, package / template)

    index = root / "index"
    for name in ("torch", "torchvision", "torchaudio", "workspace-test"):
        directory = index / name
        directory.mkdir(parents=True)
        wheel(directory, "1.0", name)
        filename = next(directory.glob("*.whl")).name
        write(directory / "index.html", f'<a href="{filename}">{filename}</a>')
    env = dict(
        os.environ,
        UV_CACHE_DIR=str(root / "uv-cache"),
        UV_DEFAULT_INDEX=index.as_uri(),
        UV_PYTHON_DOWNLOADS="never",
        UV_LINK_MODE="copy",
    )
    workspace = root / "workspace with spaces"
    workspace.mkdir()
    setup = (gov, "setup", "--workspace", workspace, "--mode", "migrate",
             "--python", sys.executable, "--torch-index-url", index.as_uri())
    _, output = run(root, env, *setup, expected=1)
    assert "Place the ComfyUI Git checkout" in output
    assert not (workspace / "bin").exists()

    # A subdirectory of an outer Git checkout must not pass as a ComfyUI checkout.
    ui = workspace / "ComfyUI"
    ui.mkdir()
    write(ui / "requirements.txt", "workspace-test==1.0\n")
    write_main(ui / "main.py", "import workspace_test")
    for checkout in (workspace, ui):
        if checkout == ui:
            _, output = run(root, env, *setup, expected=1)
            assert "must itself be the root" in output
        run(checkout, env, "git", "init", "--quiet")
        run(checkout, env, "git", "-c", "user.name=Gov tests", "-c", "user.email=test@example.invalid",
            "commit", "--quiet", "--allow-empty", "-m", "fixture")
    (ui / "custom_nodes" / "old-node").mkdir(parents=True)
    fresh = (gov, "setup", "--workspace", workspace, "--python", sys.executable)
    _, output = run(root, env, *fresh, expected=1)
    assert "Use --mode migrate" in output and not (workspace / "bin").exists()

    _, output = run(root, env, *setup)
    assert "ComfyUI core startup verified." in output and "old-node" in output
    installed = workspace / "bin" / gov.name
    sidecar = workspace / "ComfyUI.gov"
    stdout, _ = run(workspace, env, installed, "status", "--json")
    status = json.loads(stdout)
    assert Path(status["project_root"]) == sidecar
    assert status["runtime_state"] == "stopped"
    assert all(status[field] for field in (
        "config_ready", "prod_python_exists", "torch_ready", "core_ready", "lock_exists"
    ))
    run(workspace, env, installed, "verify")
    lock = (sidecar / "uv.lock").read_bytes()

    write(ui / "main.py", "print('older ComfyUI')\n")
    _, output = run(root, env, *setup)
    assert "Verify startup manually" in output
    assert (sidecar / "uv.lock").read_bytes() == lock
    run(workspace, env, installed, "verify", expected=3)

    write_main(ui / "main.py", "raise RuntimeError('probe fixture failure')")
    _, output = run(root, env, *setup, expected=1)
    assert "verifying ComfyUI startup" in output
    assert (sidecar / "uv.lock").read_bytes() == lock
    print("PASS workspace migration, ancestor rejection, resume, root resolution, and startup outcomes")


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="gov-workspace-test-") as directory:
        verify(Path(directory).resolve(), Path(sys.argv[1]).resolve(strict=True))
