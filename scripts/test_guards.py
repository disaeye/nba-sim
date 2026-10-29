from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "guard_fixtures"
CASES = [
    ("check_position_writes.py", "forbidden_position.rs", "clean_position.rs"),
    ("check_result_fields.py", "forbidden_result.rs", "clean_result.rs"),
    ("check_tick_source.py", "forbidden_tick.rs", "clean_tick.rs"),
]


def run(script: str, fixture: str) -> int:
    return subprocess.run(
        [sys.executable, str(ROOT / "scripts" / script), "--fixture", str(FIXTURES / fixture)],
        capture_output=True,
        text=True,
    ).returncode


def main() -> int:
    failed = []
    for script, forbidden, clean in CASES:
        if run(script, forbidden) != 1:
            failed.append(f"{script} accepted {forbidden}")
        if run(script, clean) != 0:
            failed.append(f"{script} rejected {clean}")
    for line in failed:
        print(line)
    if failed:
        return 1
    print("guards detect injected violations")
    return 0


if __name__ == "__main__":
    sys.exit(main())
