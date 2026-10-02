import unittest
from cuda_qualification import require_executed_tests, require_cli_reference_report


class QualificationTests(unittest.TestCase):
    def test_empty_platform_selection_is_not_hardware_proof(self):
        with self.assertRaises(RuntimeError):
            require_executed_tests("test result: ok. 0 passed; 0 failed; 0 ignored;", 1)

    def test_ignored_cases_are_not_executed_cases(self):
        with self.assertRaises(RuntimeError):
            require_executed_tests("test result: ok. 0 passed; 0 failed; 3 ignored;", 3)

    def test_insufficient_selection_fails(self):
        with self.assertRaises(RuntimeError):
            require_executed_tests("test result: ok. 1 passed; 0 failed; 0 ignored;", 3)

    def test_successful_physical_selection_counts(self):
        self.assertEqual(require_executed_tests("test result: ok. 3 passed; 0 failed; 0 ignored;", 3), 3)

    def test_failure_summary_is_not_success(self):
        with self.assertRaises(RuntimeError):
            require_executed_tests("test result: FAILED. 2 passed; 1 failed; 0 ignored;", 2)

    def test_other_passing_cases_do_not_replace_required_case(self):
        with self.assertRaises(RuntimeError):
            require_executed_tests("test other ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;", 1, ["needed"])

    def test_required_case_is_present_in_cargo_output(self):
        self.assertEqual(require_executed_tests("test suite::needed ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;", 1, ["needed"]), 1)


class CliReportTests(unittest.TestCase):
    def report(self):
        return {
            "status": "passed", "binary_sha256": "synthetic",
            "checks": ["crop decoded frames match FFmpeg", "copy decoded frames match FFmpeg",
                       "hflip decoded frames match FFmpeg", "vflip decoded frames match FFmpeg",
                       "fused decoded frames match FFmpeg",
                       "cut interval timeline and decoded frames match FFmpeg",
                       "decode-only frame count matches FFmpeg"],
        }

    def test_historical_complete_reference_report_retains_acceptance(self):
        require_cli_reference_report(self.report(), "synthetic")

    def test_named_checks_cannot_be_replaced_by_arbitrary_count(self):
        report = self.report()
        report["checks"] = ["unrelated"] * 7
        with self.assertRaisesRegex(RuntimeError, "incomplete"):
            require_cli_reference_report(report, "synthetic")

    def test_cargo_only_or_partial_new_report_cannot_qualify_cli(self):
        for flag in ["reference_requested", "reference_completed", "provided_cli_checks_completed"]:
            report = self.report()
            report.update(reference_requested=True, reference_completed=True, provided_cli_checks_completed=True)
            report[flag] = False
            with self.assertRaisesRegex(RuntimeError, "completed CLI"):
                require_cli_reference_report(report, "synthetic")

    def test_complete_current_report_requires_matching_binary(self):
        report = self.report()
        report.update(reference_requested=True, reference_completed=True, provided_cli_checks_completed=True)
        require_cli_reference_report(report, "synthetic")
        with self.assertRaisesRegex(RuntimeError, "match --binary"):
            require_cli_reference_report(report, "another")


if __name__ == "__main__":
    unittest.main()
