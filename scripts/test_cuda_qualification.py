import unittest
from cuda_qualification import require_executed_tests


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


if __name__ == "__main__":
    unittest.main()
