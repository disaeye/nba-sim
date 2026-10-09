"""stat_deltas.v1.json 登记校验（validation.md §6.3 数据准备任务）。

校验三件事：
1. 登记的 reference_value 与归档（.work/r7/shotdetail_2023.csv）重算一致；
2. 归档 SHA-256 与 ReferenceManifest 登记一致；
3. 全部 STAT 叶项都被 deltas 覆盖（full 或 blocked，缺项即失败）。
"""

import hashlib
import json
import sys
from pathlib import Path

import numpy as np
import pandas as pd

# 复用重建脚本的重建函数（单一实现，登记与校验两处调用）。
sys.path.insert(0, str(Path(__file__).parent))

import rebuild_joint_reference  # noqa: E402

REGISTRATION = Path("data/reference/stat_deltas.v1.json")
ARCHIVE = Path(".work/r7/shotdetail_2023.tar.xz")
CSV = Path(".work/r7/shotdetail_2023.csv")
PBP_ARCHIVE = Path(".work/pbp/repo/datasets/nbastats_2024.tar.xz")
PBP_CSV = Path(".work/pbp/nbastats_2024.csv")
PBPSTATS_ARCHIVE = Path(".work/pbp/repo/datasets/pbpstats_2024.tar.xz")
PBPSTATS_CSV = Path(".work/pbp/pbpstats_2024.csv")
ALL_STAT = [
    "STAT-G01", "STAT-G02", "STAT-S01", "STAT-S02", "STAT-R01",
    "STAT-T01", "STAT-B01", "STAT-P01", "STAT-M01", "STAT-I01",
]
ALL_JOINT = [
    "JOINT-C01", "JOINT-C02", "JOINT-C03", "JOINT-C04", "JOINT-C05",
    "JOINT-T01", "JOINT-T02", "JOINT-I01", "JOINT-Q01",
]


def main():
    reg = json.loads(REGISTRATION.read_text())
    frame = pd.read_csv(CSV)
    attempts = frame.loc[frame["SHOT_ATTEMPTED_FLAG"] == 1].copy()
    attempts["made"] = pd.to_numeric(attempts["SHOT_MADE_FLAG"], errors="coerce")
    attempts["is_three"] = attempts["SHOT_TYPE"].eq("3PT Field Goal")

    # 汇总命中率 / 三分占比 / 三分命中率
    grouped = attempts.groupby(["GAME_ID", "TEAM_NAME"]).agg(
        fga=("made", "size"),
        fgm=("made", "sum"),
        three_pa=("is_three", "sum"),
    )
    attempts["three_pm"] = attempts["made"] * attempts["is_three"]
    tpm = attempts.groupby(["GAME_ID", "TEAM_NAME"])["three_pm"].sum()
    fg = grouped["fgm"].sum() / grouped["fga"].sum()
    three_share = grouped["three_pa"].sum() / grouped["fga"].sum()
    three_pct = tpm.sum() / grouped["three_pa"].sum()
    assert abs(fg - 0.4743) < 0.0002, fg
    assert abs(three_share - 0.3949) < 0.0002, three_share
    assert abs(three_pct - 0.3657) < 0.0002, three_pct

    # 四区（CourtGeometry::shot_zone 边界：0/8/16/23.75）
    shot_x = 88.75 - pd.to_numeric(attempts["LOC_Y"]) / 10
    shot_y = 25.0 + pd.to_numeric(attempts["LOC_X"]) / 10
    radius = np.hypot(shot_x - 88.75, shot_y - 25.0)
    attempts["zone"] = pd.cut(
        radius, [0, 8, 16, 23.75, 100],
        labels=["rim", "near", "mid", "three"], right=False,
    ).astype(str)
    zone_attempts = attempts.groupby(["GAME_ID", "TEAM_NAME", "zone"]).size().unstack(fill_value=0)
    zone_share = zone_attempts.div(zone_attempts.sum(axis=1), axis=0).mean()
    zone_make = attempts.groupby(["GAME_ID", "TEAM_NAME", "zone"])["made"].sum().unstack(fill_value=0)
    zone_fg = {zone: zone_make[zone].sum() / zone_attempts[zone].sum() for zone in ["rim", "near", "mid", "three"]}

    # 投篮得分（不含罚球）与距离分位
    attempts["pts"] = attempts["made"] * np.where(attempts["is_three"], 3, 2)
    shot_pts = attempts.groupby(["GAME_ID", "TEAM_NAME"])["pts"].sum().mean()
    distance = pd.to_numeric(attempts["SHOT_DISTANCE"], errors="coerce").quantile([0.1, 0.25, 0.5, 0.75, 0.9])

    # 逐条核对登记值
    for delta in reg["deltas"]:
        metric = delta["metric"]
        if metric == "team-game FG%（汇总分子分母）":
            assert abs(delta["reference_value"] - fg) < 0.0002
        elif metric == "team-game 3PA/FGA 份额":
            assert abs(delta["reference_value"] - three_share) < 0.0002
        elif metric == "team-game 3P%":
            assert abs(delta["reference_value"] - three_pct) < 0.0002
        elif metric == "四区出手份额构成（Rim/Near/Mid/Three）":
            for zone, value in delta["reference_value"].items():
                assert abs(value - zone_share[zone]) < 0.001, (zone, value, zone_share[zone])
        elif metric == "四区各自 FG%（Rim/Near/Mid/Three）":
            for zone, value in delta["reference_value"].items():
                assert abs(value - zone_fg[zone]) < 0.001, (zone, value, zone_fg[zone])
        elif metric == "出手距离分布":
            keys = ["p10", "p25", "p50", "p75", "p90"]
            quantiles = [0.1, 0.25, 0.5, 0.75, 0.9]
            for key, quantile in zip(keys, quantiles, strict=True):
                assert abs(delta["reference_quantiles"][key] - distance[quantile]) < 0.01
        elif metric == "team-game 投篮得分（不含罚球）":
            assert abs(delta["reference_value"] - shot_pts) < 0.1

    # 归档哈希（shotdetail + PBP）
    digest = hashlib.sha256(ARCHIVE.read_bytes()).hexdigest()
    assert digest == reg["reference_manifest"]["data_sha256"], digest
    if PBP_ARCHIVE.exists():
        pbp_digest = hashlib.sha256(PBP_ARCHIVE.read_bytes()).hexdigest()
        assert pbp_digest == reg["reference_manifest"]["pbp_data_sha256"], pbp_digest
        verify_pbp_deltas(reg, PBP_CSV)
    else:
        print("PBP 归档不在本地，跳过 PBP 基准复核（登记含 pbp_data_sha256）")
    if PBPSTATS_ARCHIVE.exists():
        pbpstats_digest = hashlib.sha256(PBPSTATS_ARCHIVE.read_bytes()).hexdigest()
        assert pbpstats_digest == reg["reference_manifest"]["pbpstats_data_sha256"], pbpstats_digest
        verify_pbpstats_deltas(reg, PBPSTATS_CSV)
    else:
        print("pbpstats 归档不在本地，跳过逐回合基准复核")

    # 全部 STAT 叶项覆盖（full 或 blocked）
    covered = {entry["criterion_id"] for entry in reg["deltas"]}
    missing = sorted(set(ALL_STAT) - covered)
    assert not missing, f"STAT 叶项缺登记: {missing}"
    missing_joint = sorted(set(ALL_JOINT) - covered)
    assert not missing_joint, f"JOINT 叶项缺登记: {missing_joint}"
    for entry in reg["deltas"]:
        if entry["criterion_id"] in ALL_JOINT:
            if entry["data_coverage"] == "blocked":
                if entry.get("reason_code") == "reference_pipeline_pending":
                    # 数据在手的后续任务条目：须有重建说明与已登记组成部分
                    assert entry.get("data_requirement") or (
                        isinstance(entry.get("reference_value"), dict)
                        and entry["reference_value"].get("note")
                    ), entry
                else:
                    assert (
                        entry.get("missing_fields")
                        or entry.get("data_requirement")
                    ), entry
            else:
                assert entry["data_coverage"].startswith("full"), entry
                assert entry.get("rationale"), entry
    full_items = [
        entry["criterion_id"] for entry in reg["deltas"]
        if entry["data_coverage"].startswith("full")
    ]
    blocked_items = [
        entry["criterion_id"] for entry in reg["deltas"]
        if entry["data_coverage"] == "blocked"
    ]
    full_items = sorted(set(full_items))
    print("登记值与归档重算一致；SHA-256 一致；10 个 STAT 叶项与 9 个 JOINT 叶项全部登记")
    print("full 叶项:", full_items)
    print("blocked 叶项:", sorted(set(blocked_items)))


def verify_pbp_deltas(reg: dict, pbp_csv: Path) -> None:
    """从 PBP 归档重算已登记基准并逐项比对（防登记错）。"""
    frame = pd.read_csv(pbp_csv, low_memory=False)

    def team_game(events: pd.DataFrame) -> pd.DataFrame:
        out = events.copy()
        out = out[out["PLAYER1_TEAM_ID"].notna() & (out["PLAYER1_TEAM_ID"] != 0)]
        out["team_id"] = out["PLAYER1_TEAM_ID"]
        return out

    fg = team_game(frame[frame["EVENTMSGTYPE"].isin([1, 2])]).groupby(
        ["GAME_ID", "team_id"]).size().reset_index(name="fga")
    ft = team_game(frame[frame["EVENTMSGTYPE"] == 3])
    fdesc = (
        ft["HOMEDESCRIPTION"].fillna("")
        + ft["NEUTRALDESCRIPTION"].fillna("")
        + ft["VISITORDESCRIPTION"].fillna("")
    ).str.upper()
    ft["made"] = ~fdesc.str.contains("MISS")
    ftg = ft.groupby(["GAME_ID", "team_id"]).agg(
        fta=("made", "size"), ftm=("made", "sum")).reset_index()
    tov = team_game(frame[frame["EVENTMSGTYPE"] == 5])
    tvg = tov.groupby(["GAME_ID", "team_id"]).size().reset_index(name="tov")
    reb = team_game(frame[frame["EVENTMSGTYPE"] == 4])
    rdesc = reb["HOMEDESCRIPTION"].fillna("") + reb["VISITORDESCRIPTION"].fillna("")
    reb["off_count"] = rdesc.str.extract(r"Off:(\d+)")[0].astype(float)
    reb["def_count"] = rdesc.str.extract(r"Def:(\d+)")[0].astype(float)
    last = reb.groupby(["GAME_ID", "team_id", "PLAYER1_ID"]).agg(
        off_last=("off_count", "max"), def_last=("def_count", "max")).reset_index()
    trbg = last.groupby(["GAME_ID", "team_id"]).agg(
        oreb=("off_last", "sum"), drb=("def_last", "sum")).reset_index()
    foul = team_game(frame[frame["EVENTMSGTYPE"] == 6])
    foulg = foul.groupby(["GAME_ID", "team_id"]).size().reset_index(name="fouls")

    base = fg.merge(ftg, on=["GAME_ID", "team_id"], how="outer")
    base = base.merge(tvg, on=["GAME_ID", "team_id"], how="outer")
    base = base.merge(trbg, on=["GAME_ID", "team_id"], how="outer")
    base = base.merge(foulg, on=["GAME_ID", "team_id"], how="outer").fillna(0)
    base["poss"] = base["fga"] - base["oreb"] + base["tov"] + 0.44 * base["fta"]

    # Pace 分母为比赛钟实际运行分钟（validation.md §6.2：加时计入分母）
    periods = frame.groupby("GAME_ID")["PERIOD"].max()
    played_minutes = len(periods) * 48 + (periods - 4).clip(lower=0).sum() * 5
    total_poss = base.groupby("GAME_ID")["poss"].sum().sum()
    pace_adjusted = total_poss / 2 / played_minutes * 48

    checks = {
        "STAT-R01": {
            "FTA/FGA": base["fta"].sum() / base["fga"].sum(),
            "FTM/FTA": base["ftm"].sum() / base["fta"].sum(),
            "fouls_per100": 100 * base["fouls"].sum() / base["poss"].sum(),
        },
        "STAT-T01": {"tov_rate": base["tov"].sum() / base["poss"].sum()},
        "STAT-G02": {"pace": pace_adjusted},
    }
    for delta in reg["deltas"]:
        expected = checks.get(delta["criterion_id"])
        if not expected or "reference_value" not in delta:
            continue
        reference = delta["reference_value"]
        if not isinstance(reference, dict):
            continue
        for key, computed in expected.items():
            if key in reference:
                assert abs(reference[key] - computed) < 0.005, (
                    delta["criterion_id"], key, reference[key], computed)
    # JOINT-C01：Q4 末 3 分钟三分占比按比分差分桶（nbastats 自身口径）
    for delta in reg["deltas"]:
        if delta["criterion_id"] == "JOINT-C01" and "league_three_share_by_margin" in delta.get("reference_value", {}):
            computed = rebuild_joint_reference.rebuild_c01(frame)
            registered = delta["reference_value"]["league_three_share_by_margin"]
            for label, info in computed["league_three_share_by_margin"].items():
                assert abs(registered[label]["league_three_share"] - info["league_three_share"]) < 0.005, (
                    label, registered[label], info)
            assert delta["reference_value"]["total_attempts"] == computed["total_attempts"]

    # JOINT-C05：bonus 首进犯规序分布与 nbastats 源重算核对
    for delta in reg["deltas"]:
        if delta["criterion_id"] == "JOINT-C05" and "bonus_entry_foul_order_shares" in delta.get("reference_value", {}):
            computed = rebuild_joint_reference.rebuild_c05(frame)
            registered = delta["reference_value"]["bonus_entry_foul_order_shares"]
            for key, value in computed["bonus_entry_foul_order_shares"].items():
                assert key in registered, (key, value)
                assert abs(registered[key] - value) < 0.005, (key, registered[key], value)
            assert abs(
                delta["reference_value"]["team_periods_without_bonus_share"]
                - computed["team_periods_without_bonus_share"]
            ) < 0.005

    # ORB%
    orb_pairs = []
    for _, grp in base.groupby("GAME_ID"):
        if len(grp) != 2:
            continue
        a, b = grp.iloc[0], grp.iloc[1]
        if a["oreb"] + b["drb"] > 0:
            orb_pairs.append(a["oreb"] / (a["oreb"] + b["drb"]))
        if b["oreb"] + a["drb"] > 0:
            orb_pairs.append(b["oreb"] / (b["oreb"] + a["drb"]))
    assert orb_pairs, "no valid ORB% pairs computed from PBP"
    orb_mean = float(np.mean(orb_pairs))
    for delta in reg["deltas"]:
        if delta["criterion_id"] == "STAT-B01" and "reference_value" in delta:
            assert abs(delta["reference_value"]["orb_pct"] - orb_mean) < 0.005, (
                delta["reference_value"]["orb_pct"], orb_mean)


def verify_pbpstats_deltas(reg: dict, csv_path: Path) -> None:
    """从 pbpstats 逐回合归档重算 JOINT-Q01/C04 登记基准。

    源文件含按事件分块的重复行（同回合多行），必须先按
    (GAMEID, STARTTIME, ENDTIME, EVENTS) 去重；每场回合数
    （去重后 198.2，两队合计）与官方 Pace 基准交叉验证。
    """
    frame = pd.read_csv(csv_path, low_memory=False)
    frame = frame.drop_duplicates(
        subset=["GAMEID", "STARTTIME", "ENDTIME", "EVENTS"]
    )
    assert len(frame) == reg["reference_manifest"]["pbpstats_possessions"]
    games = frame["GAMEID"].nunique()
    per_game = len(frame) / games
    # 每队回合数须落在官方 Pace 基准带（2023-24 约 97-103）
    assert 190 < per_game < 206, f"per-game possessions {per_game} out of Pace band"

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

    frame["start_reason"] = frame["STARTTYPE"].map(start_reason)
    shares = frame["start_reason"].value_counts(normalize=True)
    with_oreb = frame["OFFENSIVEREBOUNDS"] > 0
    oreb_share = float(with_oreb.mean())
    oreb_poss = frame[with_oreb]
    second_attempt = float(((oreb_poss["FG2A"] + oreb_poss["FG3A"]) >= 2).mean())
    for delta in reg["deltas"]:
        if delta["criterion_id"] == "JOINT-Q01" and "reference_value" in delta:
            registered = delta["reference_value"]["start_reason_shares"]
            for reason, value in registered.items():
                assert abs(value - shares[reason]) < 0.005, (reason, value, shares[reason])
            # lag-1 转移矩阵逐格核对
            matrix = rebuild_joint_reference.rebuild_q01(frame)
            assert matrix["order"] == delta["reference_value"]["transition_matrix_order"]
            for row, cols in matrix["matrix"].items():
                for col, value in cols.items():
                    registered_cell = delta["reference_value"]["transition_matrix"][row][col]
                    assert abs(registered_cell - value) < 0.005, (row, col, registered_cell, value)
        if delta["criterion_id"] == "JOINT-C04" and "reference_value" in delta:
            reference = delta["reference_value"]
            assert abs(reference["oreb_possession_share"] - oreb_share) < 0.005
            assert abs(reference["second_attempt_share_after_oreb"] - second_attempt) < 0.005


if __name__ == "__main__":
    main()
