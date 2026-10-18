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
    # 动作链内核已在生产路径上消费的倾向。
    consumers = {
        "shoot_frequency": ["crates/sim/src/policy.rs"],
        "drive_frequency": ["crates/sim/src/policy.rs"],
        "screen_frequency": ["crates/sim/src/policy.rs"],
        "pass_frequency": ["crates/sim/src/world.rs"],
        "gamble_steal": ["crates/sim/src/resolve/pass.rs"],
        "block_aggressiveness": ["crates/sim/src/resolve/shot.rs"],
        "help_aggressiveness": ["crates/sim/src/resolve/mod.rs"],
        "physicality": ["crates/sim/src/resolve/shot.rs"],
    }
    # 尚未接入的倾向。登记在这里而不是删掉，使缺口始终可见；
    # 每项接入后从本表移入 consumers。
    pending = {
        "cut_frequency": "空切动作已有结算，但选择层还没读倾向",
        "offensive_rebound_frequency": "冲抢篮板还没有独立动作",
        "risk_tolerance": "传球风险容忍还没有进入选择层",
        "transition_sprint": "转换只按回合起始类型区分",
    }
    if set(consumers) | set(pending) != EXPECTED:
        raise SystemExit(
            "R6 tendency registry must cover every field: "
            f"missing={sorted(EXPECTED - set(consumers) - set(pending))}, "
            f"unknown={sorted((set(consumers) | set(pending)) - EXPECTED)}"
        )
    for tendency, sources in consumers.items():
        if not any(
            re.search(rf"\.\s*{re.escape(tendency)}\b", read_source(ROOT / path))
            for path in sources
        ):
            raise SystemExit(f"R6 tendency has no configured production consumer: {tendency}")
    print(
        f"R6 tendency production references: {len(consumers)}/{len(EXPECTED)} wired, "
        f"{len(pending)} registered as pending"
    )


check_rosters()
check_rust_consumers()
