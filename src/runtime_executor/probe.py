"""Run ComfyUI's native quick test and record whether it reached its exit."""

import ast
from pathlib import Path
import runpy
import sys


main = Path(sys.argv[1]).resolve()
report = Path(sys.argv[2])
sys.argv[:] = [str(main), "--quick-test-for-ci"]
sys.path.insert(0, str(main.parent))

# An arbitrary zero exit may precede startup. Accept the SystemExit only
# when it came from the native quick-test branch in main.py.
exit_lines = set()
for node in ast.walk(ast.parse(main.read_bytes(), filename=str(main))):
    if not isinstance(node, ast.If) or ast.unparse(node.test) != "args.quick_test_for_ci":
        continue
    for statement in node.body:
        if isinstance(statement, ast.Expr) and ast.unparse(statement.value) in {
            "exit(0)", "sys.exit(0)", "quit(0)"
        }:
            exit_lines.add(statement.lineno)

if not exit_lines:
    report.write_text("unsupported", encoding="ascii")
    raise SystemExit("ComfyUI does not expose a recognized --quick-test-for-ci exit")

report.write_text("started", encoding="ascii")
try:
    runpy.run_path(str(main), run_name="__main__")
except SystemExit as error:
    trace = error.__traceback__
    while trace is not None:
        if trace.tb_frame.f_code.co_filename == str(main) and trace.tb_lineno in exit_lines:
            if error.code == 0:
                report.write_text("completed", encoding="ascii")
            break
        trace = trace.tb_next
    raise
