#!/usr/bin/env python3
"""在最终行为上按权威门禁重新定标参数。

评分是「越界深度」：39 条判据各自的越界量之和，深度为零就是全部通过。
每个杠杆在当前参数点附近取一组倍率，逐杠杆扫描并只接受真实改善；最优
向量立即写进独立文件，主参数表在搜索完成前保持不变。

可用 `--initial-factors` 传入一组预测倍率作为联合起步：五条越界判据各
有一条直接因果杠杆，先按线性响应一次施加，随后由坐标下降细化。

用法：

    python3 -u scripts/calibrate/refit_after_advance.py --sweeps 3 \
        --initial-factors '{"cut_tendency":0.76,"block_base":0.86}'
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "refit"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-refit-best.json"
WORKERS = 4

# 五条越界判据的直接因果杠杆，加上与它们共享生产链的邻域杠杆。
LEVERS = [
    "cut_tendency",
    "cut_lane_scale",
    "cut_positional_bonus",
    "block_base",
    "steal_gamble_base",
    "pass_catch_steal",
    "steal_skill_base",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "bad_pass_rate",
    "gather_travel_rate",
    "transition_window_seconds",
    "transition_take_window_seconds",
    "play_call_rate",
    "seconds_per_possession",
    "reach_in_foul_rate",
    "hand_check_rate",
]
FACTORS = [0.80, 0.90, 0.96, 1.0, 1.04, 1.10, 1.20, 1.40]


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
            total += abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth_of(report), failed)


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


def persist(policy, stdout):
    # 后台搜索写独立文件：主参数表保持不变，直到搜索完成并单独验证。
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


def main():
    parser = argparse.ArgumentParser(description="按最终行为重新定标门禁参数")
    parser.add_argument("--sweeps", type=int, default=3)
    parser.add_argument("--initial-factors", default="{}")
    args = parser.parse_args()

    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial_factors = json.loads(args.initial_factors)
    for key, factor in initial_factors.items():
        set_value(base, key, get_value(base, key) * factor)

    report, stdout = evaluate([base])[0]
    assert report is not None, "baseline gate produced no report"
    best = score_of(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    for sweep in range(1, args.sweeps + 1):
        if best[0] == 0.0:
            break
        improved_any = False
        for key in LEVERS:
            current = get_value(best_policy, key)
            candidates = []
            values = []
            for factor in FACTORS:
                value = current * factor
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, value)
                candidates.append(trial)
                values.append(value)
            results = evaluate(candidates)
            local_best = best
            chosen = None
            chosen_stdout = None
            chosen_value = None
            for trial, value, (candidate_report, candidate_stdout) in zip(
                candidates, values, results
            ):
                if candidate_report is None:
                    continue
                score = score_of(candidate_report)
                if score < local_best:
                    local_best = score
                    chosen = trial
                    chosen_stdout = candidate_stdout
                    chosen_value = value
            if chosen is not None:
                best = local_best
                best_policy = chosen
                persist(best_policy, chosen_stdout)
                improved_any = True
                print(
                    f"[轮 {sweep}] {key} -> {chosen_value:.6f} {describe(best)}",
                    flush=True,
                )
        print(f"[轮 {sweep}] 结束：{describe(best)}", flush=True)
        if not improved_any:
            break

    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
