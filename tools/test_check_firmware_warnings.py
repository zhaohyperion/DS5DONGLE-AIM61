import json
import tempfile
import unittest
from pathlib import Path

from tools.check_firmware_warnings import check


class FirmwareWarningAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.baseline = self.root / "baseline.json"
        self.baseline.write_text(
            json.dumps(
                {
                    "schema": 1,
                    "warnings": [
                        {
                            "id": "known",
                            "pattern": r"^tool: warning: known warning$",
                            "maxCount": 1,
                        }
                    ],
                }
            ),
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def run_check(self, text: str):
        log = self.root / "build.log"
        log.write_text(text, encoding="utf-8")
        return check(log, self.baseline)

    def test_known_warning_with_ansi_is_accepted(self) -> None:
        errors, counts = self.run_check("\x1b[Ktool: warning: known warning\x1b[K\n")
        self.assertEqual(errors, [])
        self.assertEqual(counts["known"], 1)

    def test_new_warning_is_rejected(self) -> None:
        errors, _ = self.run_check("project.c: warning: new defect [-Wunused]\n")
        self.assertEqual(len(errors), 1)
        self.assertIn("unbaselined warning", errors[0])

    def test_count_above_baseline_is_rejected(self) -> None:
        errors, counts = self.run_check(
            "tool: warning: known warning\ntool: warning: known warning\n"
        )
        self.assertEqual(counts["known"], 2)
        self.assertTrue(any("baseline exceeded" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
