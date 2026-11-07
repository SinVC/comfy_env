"""Exercise the Slice 3 CLI with real Git and offline uv.

Usage: python tests/slice3_black_box.py <path-to-gov>
Requires Python >=3.9, Git, and uv.
"""
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import zipfile


def command(cwd, *args, expected=0):
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, timeout=45)
    output = result.stdout + result.stderr
    ok = result.returncode == expected if expected is not None else result.returncode != 0
    if not ok:
        raise AssertionError(f"{args} returned {result.returncode}, expected {expected}\n{output}")
    return output


def write(path, value):
    path.write_text(value, encoding="utf-8")


def wheel(directory, version):
    distribution = f"slice3_probe-{version}"
    with zipfile.ZipFile(directory / f"{distribution}-py3-none-any.whl", "w") as archive:
        archive.writestr("slice3_probe.py", f'VERSION = "{version}"\n')
        metadata = distribution + ".dist-info/"
        archive.writestr(metadata + "METADATA", f"Metadata-Version: 2.1\nName: slice3-probe\nVersion: {version}\n")
        archive.writestr(metadata + "WHEEL", "Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n")
        files = [*archive.namelist(), metadata + "RECORD"]
        archive.writestr(metadata + "RECORD", "".join(f"{name},,\n" for name in files))


def fail_one_sync(root, phase, production_only=False):
    uv = shutil.which("uv")
    if os.name == "nt":
        wrapper = root / "uv-fail.cmd"
        target_check = (
            'for %%I in ("%UV_PROJECT_ENVIRONMENT%") do if /i not "%%~nxI"==".venv-prod" goto run_uv\n'
            if production_only else ""
        )
        write(wrapper, (
            '@echo off\n'
            'if /i not "%~1"=="sync" goto run_uv\n'
            f'{target_check}'
            'if exist "%SLICE3_SYNC_FAILURE%" (\n'
            '  del "%SLICE3_SYNC_FAILURE%"\n'
            '  echo injected uv sync failure 1>&2\n'
            '  exit /b 91\n'
            ')\n'
            ':run_uv\n'
            f'"{uv}" %*\n'
        ))
    else:
        wrapper = root / "uv-fail"
        target_check = (
            ' && [ "${UV_PROJECT_ENVIRONMENT##*/}" = .venv-prod ]' if production_only else ""
        )
        write(wrapper, (
            '#!/bin/sh\n'
            f'if [ "$1" = sync ] && [ -f "$SLICE3_SYNC_FAILURE" ]{target_check}; then\n'
            '    rm "$SLICE3_SYNC_FAILURE"\n'
            '    echo "injected uv sync failure" >&2\n'
            '    exit 91\n'
            'fi\n'
            f'exec {shlex.quote(uv)} "$@"\n'
        ))
        wrapper.chmod(0o755)
    marker = root / f"{phase}.flag"
    write(marker, "fail once")
    os.environ["GOV_UV_BIN"] = str(wrapper)
    os.environ["SLICE3_SYNC_FAILURE"] = str(marker)
    return marker


def verify(binary, root):
    project = root / "project"
    project.mkdir()
    ui = root / "ComfyUI"
    nodes = ui / "custom_nodes"
    nodes.mkdir(parents=True)
    main = ui / "main.py"
    write(main, "import slice3_probe\nprint(slice3_probe.VERSION, flush=True)\n")
    wheels = root / "wheels"
    wheels.mkdir()
    wheel(wheels, "1.0")
    env_cache = root / "uv-cache"
    os.environ.update(
        UV_CACHE_DIR=str(env_cache),
        UV_FIND_LINKS=str(wheels),
        UV_NO_INDEX="1",
        UV_NO_PROGRESS="1",
        UV_PYTHON_DOWNLOADS="never",
        UV_OFFLINE="1",
        UV_LINK_MODE="copy",
    )
    config = (
        f"[paths]\ncomfyui_dir = {json.dumps(ui.as_posix())}\n"
        f"[runtime]\npython = {json.dumps(Path(sys.executable).as_posix())}\n"
        'prod_env = ".venv-prod"\ncandidate_root = ".venv-candidate"\n'
        '[tx]\ntimeout_seconds = 10\n'
        '[policy]\ncore_packages = ["slice3-probe"]\n'
    )
    write(project / "config.toml", config)
    write(project / "pyproject.toml", (
        '[project]\nname = "gov-slice3-fixture"\nversion = "0.1.0"\n'
        'requires-python = ">=3.9"\ndependencies = []\n'
        '[dependency-groups]\ncore = []\ntorch = []\noverrides = []\n'
    ))
    command(project, "uv", "lock", "--python", sys.executable)
    os.environ["UV_PROJECT_ENVIRONMENT"] = str(project / ".venv-prod")
    try:
        command(project, "uv", "sync", "--locked", "--all-groups", "--python", sys.executable)
    finally:
        del os.environ["UV_PROJECT_ENVIRONMENT"]
    repo = root / "plugin"
    repo.mkdir()
    write(repo / "requirements.txt", "slice3-probe==1.0\n")
    command(repo, "git", "init", "-q")
    command(repo, "git", "add", "requirements.txt")
    command(repo, "git", "-c", "user.name=Slice3", "-c", "user.email=slice3@example.invalid",
            "commit", "-qm", "Create fixture")
    url = repo.as_uri()
    state = project / "state"
    transactions = state / "transactions"

    def gov(*args, expected=0):
        return command(project, binary, *args, expected=expected)

    def read(path):
        return json.loads(path.read_text(encoding="utf-8"))

    def run(node, status, *args):
        previous = set(transactions.glob("*.json"))
        result = subprocess.run([binary, "tx", "run", node, *args], cwd=project,
                                capture_output=True, text=True, timeout=45)
        created = set(transactions.glob("*.json")) - previous
        output = result.stdout + result.stderr
        assert len(created) == 1, output
        record = read(created.pop())
        assert record["status"] == status, (record, output)
        if status == "completed":
            assert result.returncode == 0, output
        return record

    def current(record):
        return read(transactions / f'{record["txid"]}.json')

    def artifacts(record):
        return [project / record[name] for name in ("candidate_env", "staged_workdir")]

    def truth():
        return [(project / name).read_bytes() for name in ("pyproject.toml", "uv.lock")]

    def operations():
        return {path.parent.name for path in (state / "ops").glob("*/meta.json")}

    def expect_status(record, status):
        observed = current(record)
        assert observed["status"] == status, observed
        return observed

    gov("node", "add", url, "--id", "fixture")
    registry = state / "plugins.json"
    assert len(read(registry)) == 1, read(registry)
    assert (nodes / "fixture" / "requirements.txt").is_file()
    gov("node", "add", url, "--id", "fixture", expected=None)
    occupied = nodes / "occupied"
    occupied.mkdir()
    write(occupied / "sentinel.txt", "untouched")
    gov("node", "add", url, "--id", "occupied", expected=None)
    assert (occupied / "sentinel.txt").read_text() == "untouched"
    gov("node", "add", url, "--id", "bad-ref", "--ref", "missing-ref", expected=None)
    assert not (nodes / "bad-ref").exists()
    assert len(read(registry)) == 1, read(registry)
    print("PASS node add, duplicate and occupied destination, checkout cleanup", flush=True)

    baseline = truth()
    first = run("fixture", "completed")
    txid = first["txid"]
    assert all(path.exists() for path in artifacts(first)), first
    assert truth() == baseline
    inspected = gov("tx", "inspect", txid)
    assert "completed" in inspected and txid in inspected, inspected
    for path in artifacts(first):
        assert path.name in inspected, inspected
    gov("resolve", txid, "--pin", "slice3-probe==1.0", expected=None)
    gov("tx", "promote", txid, expected=None)
    assert truth() == baseline
    gov("tx", "promote", txid, "--approve-core", "--reason", "offline fixture")
    promoted = expect_status(first, "promoted")
    assert not any(path.exists() for path in artifacts(first)), first
    assert truth() != baseline
    assert "slice3-probe==1.0" in read(registry)[0]["managed_deps"], read(registry)
    assert list((state / "ops").glob("*/meta.json")), promoted
    assert list((state / "ops").glob("*/backup")), promoted
    prod = project / ".venv-prod" / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    command(project, str(prod), "-c", "import slice3_probe; assert slice3_probe.VERSION == '1.0'")
    print("PASS real uv candidate lock and sync, inspect, core gate, promote and cleanup", flush=True)

    managed_truth = truth()
    registry_before_remove = registry.read_bytes()
    smoke_failure = (
        config + '\n[tx.smoke_test]\nprogram = "python"\n'
        'args = ["-c", "raise SystemExit(9)"]\n'
    )
    write(project / "config.toml", smoke_failure)
    failure = gov("node", "remove", "fixture", "--purge-code", expected=None)
    assert "smoke" in failure.lower(), failure
    assert truth() == managed_truth and registry.read_bytes() == registry_before_remove
    assert (nodes / "fixture" / "requirements.txt").is_file()
    command(project, str(prod), "-c", "import slice3_probe")
    write(project / "config.toml", config)
    prior_ops = operations()
    gov("node", "remove", "fixture", "--purge-code")
    remove_ops = operations() - prior_ops
    assert len(remove_ops) == 1, remove_ops
    assert not (nodes / "fixture").exists()
    assert not read(registry)
    assert b"slice3-probe" not in b"".join(truth()), truth()
    command(project, str(prod), "-c", "import slice3_probe", expected=None)
    remove_op = remove_ops.pop()
    removed_truth = truth()
    marker = fail_one_sync(root, "undo")
    try:
        failure = gov("undo", remove_op, expected=None)
        assert "injected uv sync failure" in failure and not marker.exists(), failure
        assert truth() == removed_truth and not read(registry)
        assert not (nodes / "fixture").exists()
        command(project, str(prod), "-c", "import slice3_probe", expected=None)
    finally:
        del os.environ["GOV_UV_BIN"]
        del os.environ["SLICE3_SYNC_FAILURE"]
    gov("undo", remove_op)
    assert truth() == managed_truth and registry.read_bytes() == registry_before_remove
    assert (nodes / "fixture" / "requirements.txt").is_file()
    command(project, str(prod), "-c", "import slice3_probe")
    print("PASS remove smoke rollback, purge, and undo restored truth, registry, code and packages", flush=True)

    write(main, "import time\nprint('waiting', flush=True)\ntime.sleep(30)\n")
    timeout = run("fixture", "completed", "--timeout", "1")
    gov("tx", "abort", timeout["txid"])
    expect_status(timeout, "aborted")
    assert not any(path.exists() for path in artifacts(timeout)), timeout
    write(main, "raise SystemExit(7)\n")
    failed = run("fixture", "failed")
    gov("tx", "promote", failed["txid"], expected=None)
    gov("tx", "promote", failed["txid"], "--allow-failed-run", "--keep-artifacts")
    expect_status(failed, "promoted")
    assert all(path.exists() for path in artifacts(failed)), failed
    print("PASS observation timeout, abort cleanup, failed observation and retained artifacts", flush=True)

    write(main, "import slice3_probe\n")
    marker = fail_one_sync(root, "candidate")
    try:
        sync_failure = run("fixture", "failed")
        assert not marker.exists(), sync_failure
        gov("tx", "promote", sync_failure["txid"], "--allow-failed-run", expected=None)
    finally:
        del os.environ["GOV_UV_BIN"]
        del os.environ["SLICE3_SYNC_FAILURE"]
    gov("tx", "abort", sync_failure["txid"])
    print("PASS candidate sync failure cannot be promoted", flush=True)

    requirements = nodes / "fixture" / "requirements.txt"
    write(requirements, "slice3-probe==2.0\n")
    write(main, "import slice3_probe\nassert slice3_probe.VERSION == '2.0'\n")
    conflict = run("fixture", "needs_resolution")
    txid = conflict["txid"]
    assert (state / "conflicts" / f"{txid}.json").exists(), conflict
    gov("resolve", txid, "--pin", "slice3-probe>=2.0", expected=None)
    wheel(wheels, "2.0")
    resolve_output = gov("resolve", txid, "--pin", "slice3-probe==2.0")
    resolved = current(conflict)
    assert resolved["status"] == "resolved", (
        resolved, resolve_output, read(state / "conflicts" / f"{txid}.json")
    )
    assert resolved["resolution_pins"] == ["slice3-probe==2.0"], resolved
    assert all(path.exists() for path in artifacts(resolved)), resolved
    print("PASS conflict artifact, exact pin validation and resolve observation", flush=True)

    managed_truth = truth()
    registry_before_promote = registry.read_bytes()
    marker = fail_one_sync(root, "production", production_only=True)
    try:
        failure = gov("tx", "promote", txid, "--approve-core", "--reason", "sync rollback probe",
                      expected=None)
        assert "injected uv sync failure" in failure and not marker.exists(), failure
        expect_status(resolved, "promote_failed")
        assert truth() == managed_truth and registry.read_bytes() == registry_before_promote
        command(project, str(prod), "-c", "import slice3_probe; assert slice3_probe.VERSION == '1.0'")
    finally:
        del os.environ["GOV_UV_BIN"]
        del os.environ["SLICE3_SYNC_FAILURE"]
    gov("tx", "abort", txid)
    print("PASS promote sync failure restored truth, registry and production packages", flush=True)

    smoke_tx = run("fixture", "completed")
    txid = smoke_tx["txid"]
    write(project / "config.toml", smoke_failure)
    failure = gov("tx", "promote", txid, "--approve-core", "--reason", "rollback probe",
                  expected=None)
    assert "smoke" in failure.lower(), failure
    expect_status(smoke_tx, "promote_failed")
    assert truth() == managed_truth and registry.read_bytes() == registry_before_promote
    command(project, str(prod), "-c", "import slice3_probe; assert slice3_probe.VERSION == '1.0'")
    write(project / "config.toml", config)
    gov("tx", "abort", txid)
    print("PASS promote smoke failure restored truth, registry and production packages", flush=True)


if __name__ == "__main__":
    executable = str(Path(sys.argv[1]).resolve(strict=True))
    with tempfile.TemporaryDirectory(prefix="slice3-") as directory:
        verify(executable, Path(directory))
