import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA_FILES = [ROOT / "data/roster/home.json", ROOT / "data/roster/away.json"]
EXPECTED = {
    "shoot_frequency",
    "drive_frequency",
    "pass_frequency",
    "cut_frequency",
    "screen_frequency",
    "offensive_rebound_frequency",
    "gamble_steal",
    "block_aggressiveness",
    "help_aggressiveness",
    "physicality",
    "risk_tolerance",
    "transition_sprint",
}


def read_source(path):
    return path.read_text(encoding="utf-8")


def check_rosters():
    for path in DATA_FILES:
        data = json.loads(read_source(path))
        for player in data["players"]:
            tendencies = player["tendencies"]
            found = set(tendencies)
            if found != EXPECTED:
                raise SystemExit(
                    f"{path.relative_to(ROOT)} player {player['id']} tendency fields "
                    f"differ: missing={sorted(EXPECTED - found)}, extra={sorted(found - EXPECTED)}"
                )
            for name, value in tendencies.items():
                if not isinstance(value, (int, float)) or not 0.0 <= value <= 1.0:
                    raise SystemExit(
                        f"{path.relative_to(ROOT)} player {player['id']} {name} is outside 0..=1"
                    )
    print("R6 roster tendency schema: 16/16 players have 12 valid fields")


def check_rust_consumers():
    consumers = {
        "shoot_frequency": ["crates/decision/src/pipeline.rs"],
        "drive_frequency": ["crates/decision/src/pipeline.rs"],
        "pass_frequency": ["crates/decision/src/pipeline.rs"],
        "cut_frequency": ["crates/decision/src/tactics.rs"],
        "screen_frequency": ["crates/decision/src/tactics.rs"],
        "offensive_rebound_frequency": ["crates/engine/src/match_engine/contests.rs"],
        "gamble_steal": ["crates/engine/src/match_engine/contests.rs"],
        "block_aggressiveness": ["crates/engine/src/match_engine/block.rs"],
        "help_aggressiveness": ["crates/decision/src/tactics.rs"],
        "physicality": ["crates/officiating/src/resolution.rs"],
        "risk_tolerance": ["crates/engine/src/match_engine/contests.rs"],
        "transition_sprint": ["crates/engine/src/match_engine/tactics_phase.rs"],
    }
    for tendency, sources in consumers.items():
        if not any(re.search(rf"\.\s*{re.escape(tendency)}\b", read_source(ROOT / path)) for path in sources):
            raise SystemExit(f"R6 tendency has no configured production consumer: {tendency}")
    print(f"R6 tendency production references: {len(consumers)}/12")


def check_evaluator_wiring():
    source = read_source(ROOT / "crates/evaluator/src/lib.rs")
    individual = read_source(ROOT / "crates/evaluator/src/individual.rs")
    for criterion in (
        "USAGE_CONCENTRATION",
        "ASSIST_PARENT_CHAIN",
        "MATCHUP_RESPONSIBILITY",
        "LATE_GAME_STAMINA",
    ):
        if criterion not in individual or criterion not in source:
            raise SystemExit(f"R6 evaluator criterion is not wired: {criterion}")
    print("R6 individual evaluation criteria: 4/4 registered")


check_rosters()
check_rust_consumers()
check_evaluator_wiring()
