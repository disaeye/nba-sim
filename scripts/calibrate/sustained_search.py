#!/usr/bin/env python3
"""从当前参数向量做持续随机多杠杆搜索，收敛 39 条门禁。

响应面由地址抽样的离散事件构成，单杠杆梯度与线性求解都会卡在局部极小；
这里每轮随机挑 2–4 个直接因果杠杆施加小幅扰动，按权威门禁的
「越界深度、失败条数」评分，只接受改善。

搜索全程只写 `.work/evidence/policy-converged.json`；主参数表在结束时由
调用方显式复制，避免并发门禁读到写了一半的参数表。

用法：

    python3 -u scripts/calibrate/sustained_search.py --minutes 30
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import resource
import shutil
import subprocess
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "sustained"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-converged.json"
STATE = ROOT / ".work" / "evidence" / "sustained-state.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 出手模型保真参数不参与搜索；被包夹护球先验固定为满足生产证据的最小值。
# 其中一组参数在外部基准夹具里恒等为 1：夹具没有防守人、球员静止、
# 体能满值、只覆盖 Catch 与 Dribble 阶段，因此干扰、移动、疲劳与收球阶段
# 的系数在夹具里不进入乘积；真实比赛里它们都是真实生效的校准杠杆。
LEVERS = [
    "shot_quality_contest_sensitivity",
    "shot_contest_reference_rim",
    "shot_contest_reference_near",
    "shot_contest_reference_perimeter",
    "shot_aim_movement_sensitivity",
    "shot_movement_reference_rim",
    "shot_movement_reference_near",
    "shot_movement_reference_perimeter",
    "shot_fatigue_penalty",
    "shot_gather_sigma_scale",
    "shoot_pull_up_bonus",
    "shoot_catch_bonus",
    "drive_pull_up_stop_ft",
    "drive_variant_pull_up_stop_beat",
    "drive_variant_pull_up_stop_turnover",
    "finish_min_distance_ft",
    "cut_tendency",
    "cut_lane_scale",
    "cut_positional_bonus",
    "drive_base",
    "finish_priority",
    "post_base",
    "shoot_putback_bonus",
    "shoot_early_slack",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "steal_skill_scale",
    "steal_gamble_slope",
    "steal_distance_offset",
    "steal_distance_scale",
    "pass_intercept_base_ft",
    "pass_intercept_steal_ft",
    "lob_contest_steal",
    "poke_reach_steal_ft",
    "bad_pass_rate",
    "bad_pass_skill_base",
    "bad_pass_decision_weight",
    "pass_turnover_rate",
    "drive_turnover_rate",
    "pass_risk",
    "pass_base",
    "pass_open",
    "shot_quality_contest_center",
    "gather_travel_rate",
    "off_ball_hold_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "block_base",
    "transition_window_seconds",
    "transition_take_window_seconds",
    "play_call_rate",
    "seconds_per_possession",
    "possession_setup_seconds",
    "rebound_boxout_weight",
]
FACTORS = [
    0.70, 0.80, 0.86, 0.90, 0.93, 0.96, 0.98, 0.99, 0.995,
    1.005, 1.01, 1.02, 1.04, 1.07, 1.10, 1.16, 1.25, 1.40,
]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
            observed = layer["observed"]
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(layer["observed"] - layer["target"]) / max(
                abs(layer["target"]), 1e-6
            )
    return total


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth_of(report), failed)


def describe(value):
    return f"深度 {value[0]:.6f}，失败 {value[1]} 条"


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    for index in range(WORKERS):
        calibration = WORK / f"worker-{index}" / "calibration"
        calibration.mkdir(parents=True)
        for path in CALIBRATION.glob("*.json"):
            shutil.copy2(path, calibration / path.name)
        (WORK / f"worker-{index}" / "tactics").symlink_to(
            TACTICS, target_is_directory=True
        )


def evaluate(candidates):
    results = []
    for start in range(0, len(candidates), WORKERS):
        batch = candidates[start : start + WORKERS]
        processes = []
        for index, policy in enumerate(batch):
            path = WORK / f"worker-{index}" / "calibration" / "policy.json"
            path.write_text(
                json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            processes.append(
                subprocess.Popen(
                    [str(BINARY), "--gate-only", "--calibration",
                     str(WORK / f"worker-{index}" / "calibration")],
                    cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    preexec_fn=limit_address_space,
                )
            )
        for process in processes:
            output = process.stdout.read().decode("utf-8", errors="replace")
            process.wait()
            report = None
            for line in output.splitlines():
                if line.startswith("GATE "):
                    report = json.loads(line[5:])
                    break
            results.append((report, output))
    return results


def get_value(policy, key):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            return policy[section][key]
    raise KeyError(key)


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def persist(policy, stdout, score, trial):
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    STATE.write_text(
        json.dumps(
            {"depth": score[0], "failed": score[1], "trial": trial},
            indent=2,
        ) + "\n",
        encoding="utf-8",
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description="持续随机多杠杆门禁搜索")
    parser.add_argument("--minutes", type=int, default=30)
    parser.add_argument("--seed", type=int, default=20261110)
    parser.add_argument("--resume", action="store_true",
                        help="从 .work/evidence/policy-converged.json 续跑")
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    rng = random.Random(args.seed)
    if args.resume and BEST_VECTOR.is_file():
        base = json.loads(BEST_VECTOR.read_text(encoding="utf-8"))
        print("从最优合法向量续跑", flush=True)
    else:
        base = json.loads(POLICY.read_text(encoding="utf-8"))

    report, stdout = evaluate([base])[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score_of(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout, best, 0)
    print(f"基线：{describe(best)}", flush=True)

    trial = 0
    last_report = time.monotonic()
    while time.monotonic() < deadline and best[0] > 0.0:
        candidates = []
        for _ in range(WORKERS):
            candidate = copy.deepcopy(best_policy)
            for key in rng.sample(LEVERS, rng.randint(2, 4)):
                set_value(candidate, key, get_value(best_policy, key) * rng.choice(FACTORS))
            candidates.append(candidate)
        results = evaluate(candidates)
        for candidate, (candidate_report, candidate_stdout) in zip(candidates, results):
            if candidate_report is None:
                continue
            candidate_score = score_of(candidate_report)
            if candidate_score < best:
                best = candidate_score
                best_policy = candidate
                persist(best_policy, candidate_stdout, best, trial)
                print(f"[{trial:5d}] {describe(best)}", flush=True)
        trial += WORKERS
        if time.monotonic() - last_report >= 60.0:
            last_report = time.monotonic()
            print(f"[{trial:5d}] 当前 {describe(best)}", flush=True)

    print(f"最终：{describe(best)}，共判定 {trial} 个候选", flush=True)


if __name__ == "__main__":
    main()
