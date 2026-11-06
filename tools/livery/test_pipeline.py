import contextlib
import io
import json
import os
import runpy
import sys
import tempfile
import types
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import numpy as np

ROOT = Path(__file__).resolve().parent
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))
from decal_geometry import triangles_intersect_projected_rect


@contextlib.contextmanager
def in_directory(path):
    previous = Path.cwd()
    os.chdir(path)
    try:
        yield
    finally:
        os.chdir(previous)


class FakeStdin:
    def __init__(self, events):
        self.messages = []
        self.closed = False
        self.events = events

    def write(self, line):
        self.messages.append(json.loads(line))

    def flush(self):
        pass

    def close(self):
        self.closed = True
        self.events.append(("close",))


class FakeProcess:
    def __init__(self, messages):
        self.events = []
        self.stdin = FakeStdin(self.events)
        self.stdout = iter(json.dumps(message) + "\n" for message in messages)

    def wait(self, timeout):
        self.events.append(("wait", timeout))


class LiveryPipelineTests(unittest.TestCase):
    def run_pc(self, responses, calls):
        process = FakeProcess(responses)
        output = io.StringIO()
        with mock.patch.object(sys, "argv", [str(ROOT / "pc.py"), json.dumps(calls)]), \
             mock.patch("subprocess.Popen", return_value=process), \
             contextlib.redirect_stdout(output):
            try:
                runpy.run_path(str(ROOT / "pc.py"), run_name="__main__")
            except SystemExit as error:
                status = error.code
            else:
                status = 0
        return status, process, output.getvalue()

    def run_script(self, path, cwd, argv, run_result):
        output = io.StringIO()
        with in_directory(cwd), \
             mock.patch.object(sys, "argv", [str(path), *argv]), \
             mock.patch("subprocess.run", side_effect=run_result) as run, \
             contextlib.redirect_stdout(output):
            try:
                runpy.run_path(str(path), run_name="__main__")
            except SystemExit as error:
                status = error.code
            else:
                status = 0
        return status, run, output.getvalue()


    def test_body_fix11_preserves_n_peroni(self):
        layers = [
            {"id": 1, "name": "N peroni"},
            {"id": 2, "name": "N nose"},
            {"id": 3, "name": "R side"},
        ]
        first = json.dumps({"activeLayer": "body", "layers": layers}) + "\n"
        with tempfile.TemporaryDirectory() as temporary:
            Path(temporary, "decal_report.json").write_text("[]", encoding="utf-8")
            results = [SimpleNamespace(returncode=0, stdout=first, stderr=""),
                       SimpleNamespace(returncode=0, stdout="saved\n", stderr="")]
            _, run, _ = self.run_script(ROOT / "examples/body_fix11.py", temporary, [], results)

        calls = json.loads(run.call_args_list[1].args[0][3])
        deleted = [call["args"]["params"]["layer"] for call in calls
                   if call["tool"] == "command_run" and call["args"]["id"] == "layer.delete"]
        self.assertEqual(deleted, [2, 3])

    def test_pc_json_rpc_error_cleans_up_and_exits_nonzero(self):
        status, process, output = self.run_pc(
            [{"id": 0, "result": {}}, {"id": 1, "error": {"code": -32601, "message": "missing"}}],
            [{"tool": "bad", "args": {}}, {"tool": "must_not_run", "args": {}}],
        )
        self.assertEqual(status, 1)
        self.assertIn("error:", output)
        self.assertTrue(process.stdin.closed)
        self.assertEqual(process.events, [("close",), ("wait", 10)])
        sent_calls = [message for message in process.stdin.messages if message.get("method") == "tools/call"]
        self.assertEqual([message["id"] for message in sent_calls], [1])

    def test_pc_is_error_cleans_up_and_exits_nonzero(self):
        status, process, output = self.run_pc(
            [{"id": 0, "result": {}}, {"id": 1, "result": {
                "isError": True, "content": [{"type": "text", "text": "tool failed"}],
            }}],
            [{"tool": "bad", "args": {}}, {"tool": "must_not_run", "args": {}}],
        )
        self.assertEqual(status, 1)
        self.assertIn("stopping after error", output)
        self.assertTrue(process.stdin.closed)
        self.assertEqual(process.events, [("close",), ("wait", 10)])
        sent_calls = [message for message in process.stdin.messages if message.get("method") == "tools/call"]
        self.assertEqual([message["id"] for message in sent_calls], [1])

    def test_pc_keeps_no_document_close_as_a_successful_noop(self):
        status, process, _ = self.run_pc(
            [{"id": 0, "result": {}}, {"id": 1, "result": {
                "isError": True, "content": [{"type": "text", "text": "no document open"}],
            }}, {"id": 2, "result": {"content": []}}],
            [{"tool": "command_run", "args": {"id": "file.closeAll"}},
             {"tool": "doc_save", "args": {}}],
        )
        self.assertEqual(status, 0)
        self.assertTrue(process.stdin.closed)
        self.assertEqual(process.events, [("close",), ("wait", 10)])
        sent_calls = [message for message in process.stdin.messages if message.get("method") == "tools/call"]
        self.assertEqual([message["id"] for message in sent_calls], [1, 2])

    def test_parts_exits_nonzero_on_bridge_failure_without_export(self):
        fake_pil = types.ModuleType("PIL")
        fake_pil.Image = object()
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.dict(sys.modules, {"PIL": fake_pil}):
            status, run, _ = self.run_script(
                ROOT / "examples/parts.py", temporary, ["front_covers"],
                [SimpleNamespace(returncode=1, stdout="", stderr="bridge failure")],
            )

        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        calls = json.loads(run.call_args.args[0][3])
        self.assertFalse(any(call["tool"] in ("doc_save", "doc_export") for call in calls))

    def test_parts_exits_nonzero_on_batch_step_failure_without_export(self):
        fake_pil = types.ModuleType("PIL")
        fake_pil.Image = object()
        output = '[3] command_batch: {"steps":[{"ok":false}]}\n' \
                 '{"activeLayer":"body","layers":[]}\n'
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.dict(sys.modules, {"PIL": fake_pil}):
            status, run, _ = self.run_script(
                ROOT / "examples/parts.py", temporary, ["front_covers"],
                [SimpleNamespace(returncode=0, stdout=output, stderr="")],
            )

        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        calls = json.loads(run.call_args.args[0][3])
        self.assertFalse(any(call["tool"] in ("doc_save", "doc_export") for call in calls))

    def test_parts_exits_nonzero_on_save_or_export_error(self):
        fake_pil = types.ModuleType("PIL")
        fake_pil.Image = object()
        results = [
            SimpleNamespace(
                returncode=0,
                stdout='{"activeLayer":"body","layers":[{"id":1,"name":"Background"}]}\n',
                stderr="",
            ),
            SimpleNamespace(returncode=0, stdout='[2] doc_export: {"ok":false}\n', stderr=""),
        ]
        with tempfile.TemporaryDirectory() as temporary, \
             mock.patch.dict(sys.modules, {"PIL": fake_pil}):
            status, run, _ = self.run_script(
                ROOT / "examples/parts.py", temporary, ["front_covers"], results,
            )

        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 2)

    def test_trim_aborts_before_replacing_report_on_projection_error(self):
        fake_numpy = types.ModuleType("numpy")
        fake_pil = types.ModuleType("PIL")
        fake_pil.Image = object()
        fake_scipy = types.ModuleType("scipy")
        fake_scipy.ndimage = object()
        fakes = {"numpy": fake_numpy, "PIL": fake_pil, "scipy": fake_scipy}
        with tempfile.TemporaryDirectory() as temporary, mock.patch.dict(sys.modules, fakes):
            Path(temporary, "decal_report.txt").write_text(
                "REPORT " + json.dumps([{"name": "failed decal", "error": "no surface"}]),
                encoding="utf-8",
            )
            report = Path(temporary, "decal_report.json")
            report.write_text("previous complete report", encoding="utf-8")
            output = io.StringIO()
            with in_directory(temporary), contextlib.redirect_stdout(output):
                with self.assertRaises(SystemExit) as error:
                    runpy.run_path(str(TOOLS / "trim.py"), run_name="__main__")

            self.assertEqual(error.exception.code, 1)
            self.assertIn("ERR", output.getvalue())
            self.assertEqual(report.read_text(encoding="utf-8"), "previous complete report")

    def test_projected_vertex_bounds_include_triangle_crossing_decal_edge(self):
        triangles = np.array([
            [[0.0, 0.4, -0.1], [0.0, 0.8, 0.1], [0.0, 0.8, -0.1]],
            [[0.0, 0.7, -0.1], [0.0, 0.9, 0.1], [0.0, 0.8, -0.1]],
        ])
        center = np.array([0.0, 0.0, 0.0])
        base = np.array([0.0, 1.0, 0.0])
        up = np.array([0.0, 0.0, 1.0])
        centroids = triangles.mean(axis=1)
        self.assertGreater(abs((centroids[0] - center) @ base), 0.5)
        selected = triangles_intersect_projected_rect(triangles, center, base, up, 1.0, 0.5, 0.0)
        self.assertEqual(selected.tolist(), [True, False])


if __name__ == "__main__":
    unittest.main()
