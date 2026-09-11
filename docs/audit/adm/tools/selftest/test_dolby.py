"""Dolby tool wrappers: command and job construction (the tools themselves run only in the audit)."""
import unittest

from admaudit import dolby


class Commands(unittest.TestCase):
    def test_conversion_tool_argv(self):
        argv = dolby.conversion_tool_argv(r"E:\x\a.atmos", r"E:\x\out", "wav")
        self.assertEqual(argv[1:], ["-i", r"E:\x\a.atmos", "-o", r"E:\x\out", "-f", "wav"])
        self.assertTrue(argv[0].lower().endswith("cmdline_atmos_conversion_tool.exe"))

    def test_dee_convert_job_xml(self):
        xml = dolby.dee_convert_job_xml(r"E:\in", "a.atmos", r"E:\out", "a-dee.wav", "adm", r"E:\tmp")
        self.assertIn("<file_name>a.atmos</file_name>", xml)
        self.assertIn("<path>E:\\in</path>", xml)
        self.assertIn("<target_format>adm</target_format>", xml)
        self.assertIn("<file_name>a-dee.wav</file_name>", xml)
        self.assertIn("<convert_atmos_mezz version=\"2\">", xml)

    def test_atmos_info_argv_prefers_the_newer_validator(self):
        argv = dolby.atmos_info_argv(r"E:\x\a.wav", validate=True)
        self.assertIn("Dolby Media Encoder", argv[0])
        self.assertEqual(argv[1:], ["-i", r"E:\x\a.wav", "--validate", "1"])
        old = dolby.atmos_info_argv(r"E:\x\a.wav", validate=False, which="1.1")
        self.assertTrue(old[0].lower().endswith("c:\\dee\\atmos_info.exe"))
        self.assertEqual(old[1:], ["-i", r"E:\x\a.wav", "-s"])

    def test_log_findings_extract_warnings_and_errors(self):
        log = "INFO: hello\nWARNING: something odd\nERROR: bad thing\n[2026] Validation failed: x\n"
        f = dolby.log_findings(log)
        self.assertEqual([x["level"] for x in f], ["WARNING", "ERROR", "ERROR"])


if __name__ == "__main__":
    unittest.main()
