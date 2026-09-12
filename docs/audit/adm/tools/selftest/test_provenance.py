"""Provenance records, command runs and evidence envelopes."""
import json
import os
import sys
import tempfile
import unittest

from admaudit import evidence, provenance


class Provenance(unittest.TestCase):
    def test_env_record_names_the_interpreter_and_numpy(self):
        r = provenance.env_record()
        self.assertEqual(r["python"], sys.version.split()[0])
        self.assertIn("numpy", r["packages"])
        self.assertIn("utc", r)

    def test_tool_record_hashes_the_executable(self):
        r = provenance.tool_record(sys.executable)
        self.assertEqual(len(r["sha256"]), 64)
        self.assertEqual(r["bytes"], os.path.getsize(sys.executable))

    def test_run_records_exit_code_output_and_scrubbed_environment(self):
        os.environ["OADEC_AUDIT_TEST_VAR"] = "1"
        try:
            r = provenance.run([sys.executable, "-c", "import os,sys; print('hi'); print(os.environ.get('OADEC_AUDIT_TEST_VAR')); sys.exit(3)"], scrub=("OADEC_",))
        finally:
            del os.environ["OADEC_AUDIT_TEST_VAR"]
        self.assertEqual(r.exit_code, 3)
        self.assertEqual(r.stdout.splitlines(), ["hi", "None"])
        self.assertIn("OADEC_AUDIT_TEST_VAR", r.scrubbed)
        self.assertGreaterEqual(r.seconds, 0.0)
        d = r.to_json()
        self.assertEqual(d["argv"][1], "-c")


class Evidence(unittest.TestCase):
    def test_write_and_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            p = evidence.write(d, "07-example", {"numbers": {"a": 1}}, title="Example", topics=["timing"],
                               evidence_class="MEASURED", structural_result="PASS", semantic_result="FAIL",
                               classification="defect", method="unit test", inputs=[])
            with open(p, encoding="utf-8") as f:
                doc = json.load(f)
            self.assertEqual(doc["id"], "07-example")
            self.assertEqual(doc["semantic_result"], "FAIL")
            self.assertEqual(doc["numbers"]["a"], 1)
            self.assertIn("utc", doc)
            m = evidence.manifest(d)
            self.assertEqual([e["file"] for e in m["files"]], ["07-example.json"])
            self.assertEqual(len(m["files"][0]["sha256"]), 64)

    def test_results_are_validated(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                evidence.write(d, "x", {}, title="x", topics=[], evidence_class="GUESS", structural_result="PASS", semantic_result="PASS", classification="", method="", inputs=[])
            with self.assertRaises(ValueError):
                evidence.write(d, "x", {}, title="x", topics=[], evidence_class="MEASURED", structural_result="OK", semantic_result="PASS", classification="", method="", inputs=[])


if __name__ == "__main__":
    unittest.main()
