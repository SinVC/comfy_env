"""Release regressions with offline uv. Pass gov and a second local Python executable."""

import contextlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

from slice3_black_box import command, wheel, write, write_main


def wait_for(check):
    deadline = time.monotonic() + 15
    while not check():
        assert time.monotonic() < deadline, "timed out waiting for fixture"
        time.sleep(0.025)


class Fixture:
    def __init__(self, root, gov):
        self.gov = gov
        self.project = root / "project"
        self.ui = root / "ComfyUI"
        self.wheels = root / "wheels"
        self.config = self.project / "config.toml"
        self.main = self.ui / "main.py"
        self.state = self.project / "state"
        self.uv = shutil.which("uv")
        assert self.uv
        for directory in (self.project, self.ui, self.wheels):
            directory.mkdir(parents=True)
        self.env = dict(
            os.environ,
            UV_CACHE_DIR=str(root / "uv-cache"),
            UV_FIND_LINKS=str(self.wheels),
            UV_NO_INDEX="1",
            UV_OFFLINE="1",
            UV_PYTHON_DOWNLOADS="never",
            UV_LINK_MODE="copy",
        )
        self.prod = self.project / ".venv-prod" / (
            "Scripts/python.exe" if os.name == "nt" else "bin/python"
        )
        self.config_text = (
            f"[paths]\ncomfyui_dir = {json.dumps(self.ui.as_posix())}\n"
            f"[runtime]\npython = {json.dumps(sys.executable)}\n"
            '[policy]\ncore_packages = ["release-dep"]\n'
        )
        write(self.config, self.config_text)
        write(self.project / "pyproject.toml", (
            '[project]\nname = "gov-release-fixture"\nversion = "0.1.0"\n'
            'requires-python = ">=3.9"\ndependencies = []\n'
            '[dependency-groups]\ncore = []\noverrides = []\n'
            'torch = ["torch==1.0", "torchvision==1.0", "torchaudio==1.0"]\n'
        ))
        for name in ("torch", "torchvision", "torchaudio", "slice3-probe"):
            wheel(self.wheels, "1.0", name)
        self.run(self.uv, "lock", "--python", sys.executable)
        self.run(self.uv, "sync", "--locked", "--all-groups", "--python", sys.executable,
                 UV_PROJECT_ENVIRONMENT=str(self.project / ".venv-prod"))

        shim = root / "uv-shim.py"
        write(shim, (
            "import os, subprocess, sys, time\nfrom pathlib import Path\n"
            "args = sys.argv[1:]\n"
            'if args[:2] == ["python", "find"] and os.getenv("GOV_TEST_ALTERNATE"):\n'
            '    print(os.environ["GOV_TEST_ALTERNATE"])\n    raise SystemExit(0)\n'
            'if args[:1] == ["sync"] and os.getenv("GOV_TEST_GATE"):\n'
            '    gate = Path(os.environ["GOV_TEST_GATE"])\n'
            '    gate.with_suffix(".ready").write_text(str(os.getpid()))\n'
            '    while not gate.exists(): time.sleep(.025)\n'
            '    with gate.with_suffix(".log").open("ab") as log:\n'
            '        status = subprocess.call([os.environ["GOV_TEST_UV"], *args], stdout=log, stderr=log)\n'
            '    raise SystemExit(status)\n'
            'if args[:1] == ["sync"] and os.getenv("GOV_TEST_FAIL_SYNC"):\n'
            '    flag = Path(os.environ["GOV_TEST_FAIL_SYNC"])\n'
            '    if flag.exists():\n'
            '        subprocess.check_call([os.environ["GOV_TEST_UV"], *args])\n'
            '        flag.unlink()\n'
            '        raise SystemExit(91)\n'
            'raise SystemExit(subprocess.call([os.environ["GOV_TEST_UV"], *args]))\n'
        ))
        self.shim = root / ("uv-shim.cmd" if os.name == "nt" else "uv-shim")
        if os.name == "nt":
            write(self.shim, f'@echo off\n"{sys.executable}" "{shim}" %*\nexit /b %errorlevel%\n')
        else:
            write(self.shim, f"#!/bin/sh\nexec {shlex.quote(sys.executable)} {shlex.quote(str(shim))} \"$@\"\n")
            self.shim.chmod(0o755)

    def run(self, *args, expected=0, cwd=None, **env):
        result = subprocess.run(args, cwd=cwd or self.project, env=self.env | env, text=True,
                                capture_output=True, timeout=50)
        output = result.stdout + result.stderr
        assert result.returncode == expected, (args, result.returncode, output)
        return output

    def cmd(self, *args, **kwargs):
        return self.run(self.gov, *args, **kwargs)

    def records(self, pattern):
        return {p: json.loads(p.read_bytes()) for p in self.state.glob(pattern)}

    def shim_env(self, **env):
        return dict(GOV_UV_BIN=str(self.shim), GOV_TEST_UV=self.uv, **env)

    @contextlib.contextmanager
    def paused_sync(self, gate, *args):
        with subprocess.Popen([self.gov, *args], cwd=self.project,
                              env=self.env | self.shim_env(GOV_TEST_GATE=str(gate)),
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
            try:
                ready = gate.with_suffix(".ready")
                wait_for(lambda: ready.exists() and ready.stat().st_size > 0)
                yield process, int(ready.read_text())
            finally:
                gate.touch()
                try:
                    process.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                    raise


def resolve_and_promote(f, alternate):
    requirements = f.project / "requirements.txt"
    write(requirements, "slice3-probe==1.0\n")
    write(f.config, f.config_text[f.config_text.index("[runtime]"):])
    assert "paths.comfyui_dir" in f.cmd("install", expected=2)
    assert "paths.comfyui_dir" in f.cmd(
        "update", "run", "--requirements-file", str(requirements), expected=2
    )
    assert not f.records("ops/*/meta.json") and not f.records("transactions/*.json")
    write(f.config, f.config_text)
    assert "requirements file missing" in f.cmd("install", expected=2)

    repo = f.project.parent / "node"
    repo.mkdir()
    write(repo / "requirements.txt", "release-dep==1.0\t# keep the requirement\n")
    command(repo, "git", "init")
    command(repo, "git", "add", ".")
    command(repo, "git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-m", "fixture")
    f.cmd("node", "add", str(repo), "--id", "fixture")
    write_main(f.main, "pass\n")
    write(f.ui / "requirements.txt", "release-dep==1.0\n")
    f.cmd("pin", "add", "slice3-probe==1.0", "--override-requirements")
    f.cmd("tx", "run", "fixture")
    f.cmd("update", "run")

    truth_files = [f.project / "pyproject.toml", f.project / "uv.lock", f.state / "plugins.json"]
    baseline = [path.read_bytes() for path in truth_files]
    for path, record in f.records("transactions/*.json").items():
        tx = record["txid"]
        plugin = record["kind"] == "plugin"
        resolve = ("resolve",) if plugin else ("update", "resolve")
        assert record["status"] == "needs_resolution", record
        old_stage = Path(record["staged_workdir"])
        evidence = [path, *(f.state / "conflicts").glob(f"{tx}.*")]
        assert len(evidence) == 3, evidence
        before = [item.read_bytes() for item in evidence]
        stage_dirs = set((f.state / "work").iterdir())
        output = f.cmd(*resolve, tx, "--pin", "slice3-probe==2.0", expected=2)
        assert "forced override" in output, output
        assert [item.read_bytes() for item in evidence] == before and old_stage.is_dir()
        assert set((f.state / "work").iterdir()) == stage_dirs

    wheel(f.wheels, "2.0")
    wheel(f.wheels, "1.0", "release-dep")
    for path, record in f.records("transactions/*.json").items():
        tx = record["txid"]
        plugin = record["kind"] == "plugin"
        resolve = ("resolve",) if plugin else ("update", "resolve")
        promote = ("tx" if plugin else "update", "promote")
        f.cmd(*resolve, tx, "--pin", "slice3-probe==2.0", "--override-requirements")
        assert json.loads(path.read_bytes())["status"] == "resolved"

        output = f.cmd(*promote, tx, "--approve-core", expected=1,
                       **f.shim_env(GOV_TEST_ALTERNATE=str(alternate)))
        assert "resolved Python interpreter changed" in output, output
        assert json.loads(path.read_bytes())["status"] == "resolved"
        assert [p.read_bytes() for p in truth_files] == baseline
        assert "requires --approve-core" in f.cmd(*promote, tx, expected=1)

        smoke = [
            "-c",
            f"import subprocess, sys; subprocess.check_call([{f.uv!r}, 'pip', 'uninstall', "
            "'--python', sys.executable, 'release-dep'])",
        ]
        write(f.config, f.config_text + f'\n[tx.smoke_test]\nprogram = "python"\nargs = {json.dumps(smoke)}\n')
        output = f.cmd(*promote, tx, "--approve-core", expected=1)
        assert "production package set differs" in output, output
        assert json.loads(path.read_bytes())["status"] == "promote_failed", output
        assert [p.read_bytes() for p in truth_files] == baseline
        f.run(str(f.prod), "-c", "import slice3_probe; assert slice3_probe.VERSION == '1.0'")
        f.run(str(f.prod), "-c", "import release_dep", expected=1)
        write(f.config, f.config_text)

        before = f.records("transactions/*.json")
        f.cmd(*(("tx", "run", "fixture") if plugin else ("update", "run")))
        fresh = next(rec for p, rec in f.records("transactions/*.json").items() if p not in before)
        assert fresh["status"] == "completed", fresh
        f.cmd(*promote, fresh["txid"], "--approve-core")
        f.run(str(f.prod), "-c", "import release_dep")
        f.cmd("undo", f.records("transactions/*.json")[
            f.state / "transactions" / f'{fresh["txid"]}.json'
        ]["promotion"]["op_id"])
        assert [p.read_bytes() for p in truth_files] == baseline
    print("PASS both resolve retries, interpreter drift refusal, approval and package-set rollback", flush=True)


def linux_alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] not in {"Z", "X"}
    except FileNotFoundError:
        return False


def runtime_lock(f):
    write(f.main, "pass\n")
    lock = f.project / "uv.lock"
    hidden = f.project / "uv.lock.backup"
    lock.rename(hidden)
    try:
        assert "startup sync requires" in f.cmd("run", "--sync", expected=2)
        assert not f.records("ops/*/meta.json")
    finally:
        hidden.rename(lock)

    def busy(*args):
        assert "another gov command is using production" in f.cmd(*args, expected=1)

    with f.paused_sync(f.project / "launch", "run", "--sync") as (run, _):
        op = next(iter(f.records("ops/*/meta.json").values()))["op_id"]
        busy("pin", "add", "slice3-probe==1.0")
        busy("op", "recover", op)
    assert run.returncode == 0

    with f.paused_sync(f.project / "pin", "pin", "add", "slice3-probe==1.0") as (pin, _):
        busy("run")
        busy("run", "--sync")
    assert pin.returncode == 0

    baseline = (f.project / "pyproject.toml").read_bytes()
    with f.paused_sync(f.project / "crash", "run", "--sync") as (run, pid):
        pending = next(op for op in f.records("ops/*/meta.json").values() if op["status"] == "running")
        assert pending["kind"] == "runtime_sync", pending
        run.kill()
        run.wait()
        assert "needs recovery" in f.cmd("run", expected=1)
        if sys.platform == "linux":
            output = f.cmd("op", "recover", pending["op_id"], expected=2)
            assert "external process tree is still running" in output, output
    if sys.platform == "linux":
        wait_for(lambda: not linux_alive(pid))
    with f.paused_sync(f.project / "recover", "op", "recover", pending["op_id"]) as (recover, _):
        busy("run")
        busy("run", "--sync")
    assert recover.returncode == 0
    assert (f.project / "pyproject.toml").read_bytes() == baseline
    f.run(str(f.prod), "-c", "import slice3_probe")
    print("PASS both lock directions and interrupted production recovery", flush=True)


def first_init_rollback(f):
    project = f.project.parent / "first-init"
    project.mkdir()
    repo = Path(__file__).resolve().parents[1]
    for name in ("config.toml.template", "pyproject.toml.template"):
        shutil.copyfile(repo / name, project / name)
    flag = project / "fail-once"
    flag.touch()
    args = ("init", "--comfyui-dir", str(f.ui), "--python", sys.executable)
    output = f.cmd(*args, cwd=project, expected=1, **f.shim_env(GOV_TEST_FAIL_SYNC=str(flag)))
    assert "91" in output and not flag.exists(), output
    for path in ("config.toml", "pyproject.toml", "uv.lock", "state/plugins.json", ".venv-prod"):
        assert not (project / path).exists(), path
    f.cmd(*args, cwd=project)
    assert (project / ".venv-prod").is_dir()
    print("PASS first init rolls back a newly created environment and can be retried", flush=True)


def linux_runtime_group(f):
    child = f.ui / "worker"
    script = (
        "import pathlib, subprocess, sys, time\n"
        "worker = subprocess.Popen([sys.executable, '-c', {worker!r}], "
        "stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
        "pathlib.Path({pidfile!r}).write_text(str(worker.pid))\n"
        "{tail}"
    )
    write(f.main, script.format(worker="import time; time.sleep(90)", pidfile=str(child), tail=""))
    f.cmd("run")
    assert not linux_alive(int(child.read_text()))
    assert not (f.state / "comfyui.pid").exists()

    child.unlink()
    ready = f.ui / "worker-ready"
    worker = f"import signal, time, pathlib; signal.signal(signal.SIGTERM, signal.SIG_IGN); pathlib.Path({str(ready)!r}).touch(); time.sleep(90)"
    write(f.main, script.format(worker=worker, pidfile=str(child), tail="time.sleep(90)\n"))
    with subprocess.Popen([f.gov, "run"], cwd=f.project, env=f.env) as running:
        try:
            wait_for(lambda: ready.exists() and child.exists() and child.stat().st_size > 0)
            pid = int(child.read_text())
            running.kill()
            running.wait()
            assert "ComfyUI is already running" in f.cmd("pin", "add", "slice3-probe==1.0", expected=1)
            assert "ComfyUI stopped" in f.cmd("stop")
            assert not linux_alive(pid)
            assert not (f.state / "comfyui.pid").exists()
        finally:
            if (f.state / "comfyui.pid").exists():
                subprocess.run([f.gov, "stop"], cwd=f.project, env=f.env, timeout=45)
    print("PASS Linux main exit, owner crash and forced descendant termination", flush=True)


if __name__ == "__main__":
    gov, alternate = [Path(arg).resolve(strict=True) for arg in sys.argv[1:]]
    with tempfile.TemporaryDirectory(prefix="gov-release-") as directory:
        root = Path(directory)
        resolve_and_promote(Fixture(root / "transactions", str(gov)), alternate)
        locking = Fixture(root / "locking", str(gov))
        runtime_lock(locking)
        first_init_rollback(locking)
        if sys.platform == "linux":
            linux_runtime_group(Fixture(root / "runtime", str(gov)))
