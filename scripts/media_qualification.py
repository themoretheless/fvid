"""Evidence required by the explicit CPU media reference performance gate."""


def require_cli_reference_report(report, binary_sha256, minimum=524):
    checks = report.get("checks", [])
    if report.get("status") != "passed" or len(checks) < minimum or any(
        not isinstance(check, dict) or check.get("status") != "passed" for check in checks
    ):
        raise RuntimeError("full media reference report is incomplete; run validate_media.py --benchmark-reference")
    flags = ("reference_requested", "reference_completed", "provided_cli_checks_completed")
    if any(flag in report for flag in flags) and not all(report.get(flag) is True for flag in flags):
        raise RuntimeError("media report does not prove completed CLI references; run validate_media.py --benchmark-reference")
    if report.get("binary_sha256") != binary_sha256:
        raise RuntimeError("validation report does not match --binary; run validate_media.py --benchmark-reference")
