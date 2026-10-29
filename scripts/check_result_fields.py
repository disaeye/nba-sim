from pathlib import Path
import argparse
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("ActionIntent", "PendingShot")
FIELD = re.compile(r"\b(made|beaten|intercepted|rebounder)\s*:")


def blocks(text: str, name: str) -> list[tuple[int, str]]:
    found = []
    pattern = re.compile(rf"\bstruct\s+{name}\s*\{{")
    for match in pattern.finditer(text):
        depth = 1
        index = match.end()
        start = index
        while index < len(text) and depth > 0:
            if text[index] == "{":
                depth += 1
            elif text[index] == "}":
                depth -= 1
            index += 1
        if depth != 0:
            continue
        line = text.count("\n", 0, match.start()) + 1
        found.append((line, text[start:index]))
    return found


def scan(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    violations = []
    for name in TARGETS:
        for line, body in blocks(text, name):
            for offset, row in enumerate(body.splitlines()):
                if FIELD.search(row):
                    violations.append(f"{path}:{line + offset}: {row.strip()}")
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
        print(f"result fields on action state: {len(violations)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
