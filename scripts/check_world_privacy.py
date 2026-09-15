#!/usr/bin/env python3
"""D4.2 World 私有化守卫（dev 方案 §7.1）。

比赛真相字段不得在 `MatchEngine` 上公开可变（`pub`）：外部只能经只读
访问器读取，或经 `step()` / `snapshot()` 推进与观测。

判据：`MatchEngine` 结构体声明中不允许出现处于"真相字段"清单内的
`pub <field>:`。清单外的字段（规则、阵容、战术、物理后端等外部依赖与
配置）允许 `pub`——它们不是比赛状态的真相来源。

负面对照（`--self-test`）：临时把一条真相字段改回 `pub`，守卫必须变红。

退出码：0 = 通过，1 = 发现违规。
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE_SRC = ROOT / "crates" / "engine" / "src" / "match_engine.rs"

# 比赛真相字段：球权/球态/时钟/比分/阶段/回合上下文/事件会计。
# 每一条都必须私有，写入只能走引擎内部唯一入口（或显式的 *_for_test 钩子）。
TRUTH_FIELDS = {
    # 球权与球态
    "possession",
    "possession_id",
    "ball_state",
    "ball_pos_3d",
    "carrier_idx",
    "last_passer_id",
    # 时钟与时间
    "game_clock",
    "shot_clock",
    "current_time",
    "sub_phase",
    "sub_phase_timer",
    "inbound_elapsed",
    "backcourt_elapsed",
    "period_break_elapsed",
    "last_decision_time",
    # 比分与犯规
    "home_score",
    "away_score",
    "team_fouls_home",
    "team_fouls_away",
    "free_throws_remaining",
    "free_throw_attempt",
    "free_throw_shooter",
    "box_score",
    # 生命周期
    "game_flow",
    "period",
    "completed_possessions",
    "simulation_complete",
    "scope_active",
    "target_possessions",
    "inbound_baseline",
    # 回合上下文
    "current_possession_start_clock",
    "current_possession_start_time",
    "current_possession_passes",
    "current_possession_shooter",
    "current_possession_contest",
    "current_possession_turnover_player",
    "last_possession_summary_index",
    "pending_pass_receiver",
    "pending_loose_ball_terminal",
    "pending_pass_inbound",
    # 事件会计
    "event_sequence",
    "event_id_counter",
    "current_event_log",
    "current_event_types",
    "current_enforcements",
    "pending_events",
    "causal_links",
    # 观测量缓存
    "latest_spacing",
    "latest_contacts",
    "last_decision_trace",
    "active_windows",
    "current_event",
    "current_callout",
    "current_intensity",
    "invariant_checker",
    "last_tick_violations",
    "tactical_set",
}

STRUCT_START = re.compile(r"^pub struct MatchEngine \{")


def struct_body(source: str) -> str:
    """Return the MatchEngine struct declaration body (fields only)."""
    lines = source.splitlines()
    start = None
    for i, line in enumerate(lines):
        if STRUCT_START.match(line):
            start = i
            break
    if start is None:
        raise SystemExit("❌ guard error: `pub struct MatchEngine` not found")
    body: list[str] = []
    for line in lines[start + 1 :]:
        if line.startswith("}"):
            break
        body.append(line)
    return "\n".join(body)


def find_violations(source: str) -> list[str]:
    """返回 `MatchEngine` 上所有 `pub` 字段（判据已升级）。

    ## 为何从「清单」改为「零容忍」（D7 后续项）

    旧判据只拦 `TRUTH_FIELDS` 名单内的字段，因此 16 个「配置/外部依赖」
    类字段（`physics`/`rules`/`rng`/`coach`/…）长期为 `pub`。
    实测全仓外部使用面是**纯只读**的（无任何外部赋值或可变借用），
    所以这 16 个字段的可变性无任何合法用途，却允许调用方绕过阶段化
    推进与不变量检查（例如 `engine.rules.tick_seconds = f32::MAX`）。

    现改为：「`MatchEngine` 不得有任何 `pub` 字段」，即只允许经
    `step()` / `snapshot()` / 只读访问器 / 显式 `*_for_test` 钩子交互。
    这只使守卫更严（旧清单是它的子集），不会放过原有任何违规。
    """
    body = struct_body(source)
    return sorted(
        set(re.findall(r"^\s*pub ([a-z_][a-z_0-9]*)\s*:", body, re.M))
    )


def self_test() -> int:
    """负面对照：把一条真相字段改回 pub，守卫必须变红。"""
    original = ENGINE_SRC.read_text()
    field = "home_score"
    # 该字段当前应为私有（无 pub 前缀）。
    if re.search(rf"^\s*pub {field}\s*:", struct_body(original), re.M):
        print("❌ self-test: fixture field is already pub; cannot test")
        return 1
    mutated = re.sub(
        rf"^(\s*){field}:", rf"\1pub {field}:", original, count=1, flags=re.M
    )
    if mutated == original:
        print("❌ self-test: failed to inject a pub truth field")
        return 1
    violations = find_violations(mutated)
    if field not in violations:
        print("❌ self-test: guard did not detect the injected pub truth field")
        return 1
    # 正常源码必须无违规（否则守卫基线本身是红的）。
    if find_violations(original):
        print("❌ self-test: baseline source already has violations")
        return 1
    print("✅ World-privacy guard self-test passed (detects pub truth fields).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()

    source = ENGINE_SRC.read_text()
    violations = find_violations(source)
    if violations:
        print("❌ World-privacy guard FAILED — MatchEngine must expose no public fields:")
        for field in violations:
            print(f"   - MatchEngine.{field}")
        print(
            "   D7 / gap.md §20.1：外部只能经 step()、snapshot()、只读访问器或"
            "显式 `*_for_test` 钩子交互；直接公开字段即可绕过阶段化推进与不变量检查。"
        )
        return 1
    print(
        f"✅ World-privacy guard passed "
        f"(all MatchEngine state fields private; "
        f"{len(TRUTH_FIELDS)} truth fields + config/dependency fields covered)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
