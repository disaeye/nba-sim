"""JOINT-C01/C05/Q01 参照值重建（todo #24，validation.md §6.3 数据准备）。

从本地冻结归档重建三个待补参照值并写入 stat_deltas.v1.json：
1. JOINT-C01 比分差×剩余时间条件下的三分出手占比带（nbastats_2024 PBP）；
2. JOINT-C05 bonus 首次进入的犯规序分布（nbastats_2024 PBP，.PN 官方标记）；
3. JOINT-Q01 回合开始原因 lag-1 转移矩阵（pbpstats_2024 逐回合，按主客队内转移）。

三个归档覆盖两个赛季：shotdetail_2023.csv 为 2023-24（GAME_ID 223xxxxx），
nbastats_2024.csv 与 pbpstats_2024.csv 为 2024-25（GAME_ID 224xxxxx，日期
2024-10-22 ~ 2025-04-13）。跨文件的比赛级关联不可行（比赛集合不同），本脚本
严格在各归档内部计算，赛季口径写入登记文件。

用法：python3 scripts/rebuild_joint_reference.py [--write]
  不带 --write 时只打印重算结果与登记值差异，不改文件。
"""

import argparse
import json
import sys
from pathlib import Path

import numpy as np
import pandas as pd

REGISTRATION = Path("data/reference/stat_deltas.v1.json")
NBASTATS_CSV = Path(".work/pbp/nbastats_2024.csv")
PBPSTATS_CSV = Path(".work/pbp/pbpstats_2024.csv")

# nbastats 犯规编码：1 个人 2 投篮 3 无球 4 进攻 26 进攻撞人 28 转换取犯规 31 转换取犯规。
# 技术犯规(11/17)与恶意犯规(14+)不计入团队犯规账（NBA 规则）。
TEAM_FOUL_ACTIONTYPES = [1, 2, 3, 4, 26, 28, 31]
# 回合开始原因的 7 类映射（与 stat_deltas.v1.json JOINT-Q01 分类一致）。
START_REASONS = [
    "field_goal", "free_throw", "steal", "dead_ball",
    "timeout", "blocked_shot", "other",
]


def remain_seconds(pctimestring: pd.Series) -> pd.Series:
    """PCTIMESTRING 'M:SS' → 节内剩余秒数。"""
    parts = pctimestring.astype(str).str.split(":", expand=True)
    return parts[0].astype(int) * 60 + parts[1].astype(int)


def rebuild_c01(pbp: pd.DataFrame) -> dict:
    """Q4 末 3 分钟窗口，按出手前比分差分桶的三分出手占比（per-game 分布）。

    比分差用比赛内向前填充（SCOREMARGIN 出手行偶有缺失）；
    分桶 |margin| ∈ [0,5] / (5,10] / (10,+∞)；
    参照值登记 league 汇总占比与 per-game 分布的 p5/p50/p95。
    """
    frame = pbp.sort_values(["GAME_ID", "EVENTNUM"]).copy()
    frame["margin"] = pd.to_numeric(
        frame.groupby("GAME_ID")["SCOREMARGIN"].ffill().replace("TIE", 0),
        errors="coerce",
    )
    desc = frame["HOMEDESCRIPTION"].fillna("") + frame["VISITORDESCRIPTION"].fillna("")
    frame["is_three"] = desc.str.contains("3PT")
    shots = frame[frame["EVENTMSGTYPE"].isin([1, 2])].copy()
    shots["remain"] = remain_seconds(shots["PCTIMESTRING"])
    late = shots[(shots["PERIOD"] == 4) & (shots["remain"] <= 180) & shots["margin"].notna()]

    league = {}
    per_game_rows = []
    buckets = pd.cut(
        late["margin"].abs(), [-0.1, 5, 10, np.inf],
        labels=["0-5", "6-10", "10+"],
    )
    late = late.assign(bucket=buckets)
    for label in ["0-5", "6-10", "10+"]:
        sub = late[late["bucket"] == label]
        share = float(sub["is_three"].mean())
        league[label] = {
            "league_three_share": round(share, 4),
            "attempts": int(len(sub)),
        }
        per_game = sub.groupby("GAME_ID")["is_three"].agg(["size", "mean"])
        per_game = per_game[per_game["size"] >= 5]
        per_game_rows.append(per_game["mean"].quantile([0.05, 0.5, 0.95]).tolist())
    per_game_bands = {
        label: [round(v, 4) for v in band]
        for label, band in zip(["0-5", "6-10", "10+"], per_game_rows)
    }
    return {
        "window": "Q4 remain<=180s, margin buckets by |score_margin|",
        "league_three_share_by_margin": league,
        "per_game_three_share_p5_p50_p95": per_game_bands,
        "total_attempts": int(len(late)),
    }


def rebuild_c05(pbp: pd.DataFrame) -> dict:
    """每队每节 bonus 首次进入时的团队犯规序分布。

    官方记录语义（实证核对 2024-25 全季）：团队犯规描述计数 T1..T4 递增；
    .PN 标记该犯规发生在 penalty（bonus）中，行后紧随 Free Throw 1 of 2。
    参照值 = 首个 .PN 行是该队该节第几次团队犯规行的分布，外加
    含/不含 .PN 的队节占比（低犯规队节整节未进 bonus）。
    """
    fouls = pbp[pbp["EVENTMSGACTIONTYPE"].isin(TEAM_FOUL_ACTIONTYPES)].copy()
    desc = fouls["HOMEDESCRIPTION"].fillna("") + fouls["VISITORDESCRIPTION"].fillna("")
    fouls = fouls[desc.str.contains("FOUL", na=False)].copy()
    fouls["is_pn"] = fouls["HOMEDESCRIPTION"].fillna("").str.contains(
        r"\.PN\)"
    ) | fouls["VISITORDESCRIPTION"].fillna("").str.contains(r"\.PN\)")

    first_pn_order = []
    periods_with_pn = 0
    total_team_periods = 0
    for _, grp in fouls.groupby(["GAME_ID", "PERIOD", "PLAYER1_TEAM_ID"]):
        total_team_periods += 1
        ordered = grp.sort_values("EVENTNUM")
        pn_idx = np.flatnonzero(ordered["is_pn"].to_numpy())
        if len(pn_idx):
            periods_with_pn += 1
            first_pn_order.append(int(pn_idx[0]) + 1)
    order = pd.Series(first_pn_order).value_counts().sort_index()
    dist = {f"foul_{k}": round(float(v) / periods_with_pn, 4) for k, v in order.items()}
    return {
        "definition": "首个 .PN（penalty）犯规行是该队该节第几次团队犯规",
        "bonus_entry_foul_order_shares": dist,
        "team_periods_total": total_team_periods,
        "team_periods_with_bonus": periods_with_pn,
        "team_periods_without_bonus_share": round(
            1 - periods_with_pn / total_team_periods, 4
        ),
    }


def rebuild_q01(pbpstats: pd.DataFrame) -> dict:
    """回合开始原因 lag-1 转移矩阵（同队相邻回合，行=前因 列=后果）。

    pbpstats 行即回合段；OREB 延续段以独立行存在，直接按行序取同队相邻对，
    与 JOINT-C04 的口径一致。占比按行归一（每行占比和为 1）。
    """
    frame = pbpstats.drop_duplicates(
        subset=["GAMEID", "STARTTIME", "ENDTIME", "EVENTS"]
    ).copy()

    def start_reason(start_type: str) -> str:
        if start_type.startswith("Off Steal"):
            return "steal"
        if start_type.startswith("Off Dead Ball"):
            return "dead_ball"
        if start_type.startswith("Off Timeout"):
            return "timeout"
        if start_type.startswith("Off FT"):
            return "free_throw"
        if start_type.startswith("Off Block"):
            return "blocked_shot"
        if "Make" in start_type or "Miss" in start_type:
            return "field_goal"
        return "other"

    frame["reason"] = frame["STARTTYPE"].map(start_reason)
    transition_counts = pd.DataFrame(0.0, index=START_REASONS, columns=START_REASONS)
    for _, game in frame.groupby("GAMEID"):
        # 行布局（实证核对）：每场比赛先排进攻方 A 的全部回合（OPPONENT=B），
        # 再排进攻方 B 的回合；每段内部时间倒序（Q4 → Q1）。段内反转行序即时间正序。
        for _, seg in game.groupby("OPPONENT"):
            seg = seg.iloc[::-1]
            reasons = seg["reason"].tolist()
            for i in range(1, len(reasons)):
                transition_counts.loc[reasons[i - 1], reasons[i]] += 1
    row_sums = transition_counts.sum(axis=1)
    matrix = transition_counts.div(row_sums.replace(0, np.nan), axis=0)
    return {
        "definition": "同队相邻回合开始原因转移占比，行=前回合开始原因，列=本回合开始原因",
        "order": START_REASONS,
        "matrix": {
            row: {col: round(float(matrix.loc[row, col]), 4) for col in START_REASONS
                  if row_sums[row] > 0}
            for row in START_REASONS
        },
        "row_counts": {row: int(row_sums[row]) for row in START_REASONS},
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true", help="把重算值写进登记文件")
    args = parser.parse_args()

    if not NBASTATS_CSV.exists() or not PBPSTATS_CSV.exists():
        sys.exit(f"归档缺失：{NBASTATS_CSV} / {PBPSTATS_CSV}")

    pbp = pd.read_csv(NBASTATS_CSV, low_memory=False)
    pbpstats = pd.read_csv(PBPSTATS_CSV, low_memory=False)

    c01 = rebuild_c01(pbp)
    c05 = rebuild_c05(pbp)
    q01 = rebuild_q01(pbpstats)

    print("JOINT-C01 Q4末3分钟三分占比（按|比分差|分桶）:")
    for label, info in c01["league_three_share_by_margin"].items():
        print(f"  |margin| {label}: {info['league_three_share']:.4f} "
              f"(n={info['attempts']})")
    print("JOINT-C05 bonus 首进犯规序分布:")
    for key, value in c05["bonus_entry_foul_order_shares"].items():
        print(f"  {key}: {value}")
    print(f"  未进 bonus 队节占比: {c05['team_periods_without_bonus_share']}")
    print("JOINT-Q01 转移矩阵（行=前因）:")
    for row, cols in q01["matrix"].items():
        cells = " ".join(f"{col}:{value:.3f}" for col, value in cols.items())
        print(f"  {row} (n={q01['row_counts'][row]}): {cells}")

    reg = json.loads(REGISTRATION.read_text())
    for delta in reg["deltas"]:
        cid = delta["criterion_id"]
        if cid == "JOINT-C01" and "league_three_share_by_margin" in delta.get(
            "reference_value", {}
        ):
            for label, info in c01["league_three_share_by_margin"].items():
                old = delta["reference_value"]["league_three_share_by_margin"][label]
                if abs(old["league_three_share"] - info["league_three_share"]) > 0.005:
                    print(f"差异 C01 {label}: 登记 {old['league_three_share']} "
                          f"重算 {info['league_three_share']}")
        if cid == "JOINT-C05" and "bonus_entry_foul_order_shares" in delta.get(
            "reference_value", {}
        ):
            for key, value in c05["bonus_entry_foul_order_shares"].items():
                old = delta["reference_value"]["bonus_entry_foul_order_shares"].get(key)
                if old is not None and abs(old - value) > 0.005:
                    print(f"差异 C05 {key}: 登记 {old} 重算 {value}")
        if cid == "JOINT-Q01" and "transition_matrix" in delta.get(
            "reference_value", {}
        ):
            for row, cols in q01["matrix"].items():
                for col, value in cols.items():
                    old = delta["reference_value"]["transition_matrix"][row][col]
                    if abs(old - value) > 0.005:
                        print(f"差异 Q01 {row}->{col}: 登记 {old} 重算 {value}")

    if args.write:
        for delta in reg["deltas"]:
            cid = delta["criterion_id"]
            if cid == "JOINT-C01":
                delta["statistic_kind"] = "composition"
                delta["data_coverage"] = "full"
                delta.pop("reason_code", None)
                delta["reference_value"] = c01
                delta["allowed_delta"] = {
                    "league_three_share_by_margin_abs": 0.06,
                    "per_game_three_share_band_w1": 0.08,
                }
                delta["rationale"] = (
                    "nbastats_2024（2024-25 赛季，1,230 场）PBP 出手行重算："
                    "Q4 末 3 分钟窗口按 |比分差| 三桶的三分占比（league 汇总）与 "
                    "per-game 分布分位带（每场窗口出手 ≥5 的比赛）。允许带取联盟"
                    "级占比 0.06 绝对差与 per-game 带 W1 0.08；出手选择在关键窗口"
                    "的合理性由桶间对比体现（防守收紧时占比应上升）。"
                )
            elif cid == "JOINT-C05":
                delta["statistic_kind"] = "composition"
                delta["data_coverage"] = "full"
                delta.pop("reason_code", None)
                delta["reference_value"] = c05
                delta["allowed_delta"] = {
                    "bonus_entry_foul_order_shares_tv": 0.12,
                    "fouls_per100": 3.5,
                }
                delta["rationale"] = (
                    "nbastats_2024 官方 .PN（penalty）标记重算：队节内首个 bonus "
                    "犯规的团队犯规序分布（5 次阈值占主体，4 次阈值约 12%），外加"
                    "整节未进 bonus 的队节占比。允许带 TV 0.12 覆盖引擎固定阈值 5 "
                    "与真实混合分布的差距；fouls_per100 允许带沿袭 STAT-R01。"
                )
            elif cid == "JOINT-Q01":
                value = delta["reference_value"]
                value["transition_matrix"] = q01["matrix"]
                value["transition_matrix_order"] = q01["order"]
                value["transition_row_counts"] = q01["row_counts"]
                value["scope"] = (
                    "full 覆盖：回合开始原因边缘构成 TV + lag-1 转移矩阵逐格"
                    "（同队相邻回合，行归一）。"
                )
                delta["allowed_delta"]["transition_matrix_cell_abs"] = 0.08
                delta["allowed_delta"]["transition_matrix_row_tv"] = 0.12
                delta["rationale"] = (
                    "pbpstats_2024（2024-25 赛季，去重 243,768 回合）同队相邻回合"
                    "转移矩阵；逐格允许带 0.08 绝对差、整行 TV 0.12，与边缘构成"
                    "允许带同档。"
                )
        REGISTRATION.write_text(
            json.dumps(reg, ensure_ascii=False, indent=1) + "\n"
        )
        print("登记文件已更新（C01/C05 翻 full，Q01 增矩阵）")
    else:
        print("（预览模式，加 --write 写入登记文件）")


if __name__ == "__main__":
    main()
