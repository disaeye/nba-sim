from pathlib import Path
import argparse
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = {"body.rs", "ball.rs"}
PATTERNS = [
    re.compile(r"\bplayers\s*\[[^\]]*\]\s*\.\s*pos\s*(?:=|\+=|-=|\*=|/=)"),
    re.compile(r"\bplayer\s*\.\s*pos\s*(?:=|\+=|-=|\*=|/=)"),
    re.compile(r"\bpos\s*\.\s*[xy]\s*(?:=|\+=|-=|\*=|/=)"),
]


def scan(path: Path) -> list[str]:
    violations = []
    if path.name in ALLOWED:
        return violations
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        for pattern in PATTERNS:
            if pattern.search(line):
                violations.append(f"{path}:{number}: {line.strip()}")
                break
    return violations


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=Path)
    args = parser.parse_args()
    if args.fixture:
        targets = [args.fixture]
    else:
        targets = sorted((ROOT / "crates" / "sim" / "src").rglob("*.rs"))
    violations = []
    for path in targets:
        violations.extend(scan(path))
    for violation in violations:
        print(violation)
    if violations:
        print(f"position writes outside body/ball: {len(violations)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
