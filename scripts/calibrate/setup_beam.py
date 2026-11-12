#!/usr/bin/env python3
"""以 possession_setup_seconds 的修复窗口为起点的种群搜索。

一维扫描确认：setup≈5.0 让持球人在八秒时限内完成首次决策，虚假八秒
失误消失（turnover 相对误差从 +0.23 降到 +0.03）。但节奏变快会引出
若干轻微越界，需要其余参数补偿。之前的搜索把 setup 下界锁在 7.8，
把这个窗口完全排除了。

每个 worker 独立校准目录，评分来自权威门禁的 39 条判定。四个起点
并行演化，按越界深度收敛；深度为 0 即全部通过。

用法：

    python3 -u scripts/calibrate/setup_beam.py --rounds 200
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "setup-beam"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

SETUP_STARTS = [4.5, 5.0, 5.5, 6.0]

# (下界, 上界, 步长)。步长小，避免大改动重排整条按地址抽样的链。
LEVERS = {
    "possession_setup_seconds": (4.0, 6.5, 0.15),
    "seconds_per_possession": (15.0, 21.0, 0.3),
    "cut_lane_scale": (0.30, 0.70, 0.02),
    "cut_tendency": (0.0, 0.09, 0.005),
    "cut_positional_bonus": (0.0, 0.02, 0.0015),
    "shoot_putback_bonus": (0.50, 1.10, 0.03),
    "shoot_catch_bonus": (0.0, 0.5, 0.02),
    "finish_min_distance_ft": (2.2, 3.9, 0.1),
    "finish_priority": (0.22, 0.62, 0.02),
    "drive_base": (1.3, 2.1, 0.05),
    "post_base": (0.38, 0.80, 0.02),
    "gather_travel_rate": (0.004, 0.06, 0.003),
    "shot_aim_sigma_scale": (0.90, 1.45, 0.02),
    "shot_distance_decay_three_scale_ft": (8.0, 13.0, 0.25),
    "pass_catch_steal": (0.32, 0.70, 0.015),
    "steal_gamble_base": (0.45, 1.05, 0.04),
    "steal_skill_base": (0.60, 1.15, 0.04),
    "off_ball_hold_rate": (0.0015, 0.010, 0.0005),
    "hand_check_rate": (0.0008, 0.009, 0.0006),
    "reach_in_foul_rate": (0.006, 0.035, 0.002),
    "play_call_rate": (0.68, 0.88, 0.008),
    "shooting_foul_rate_two_point": (0.10, 0.30, 0.008),
    "shooting_foul_rate_three_point": (0.015, 0.09, 0.005),
    "and_one_rate": (0.10, 0.30, 0.008),
    "transition_window_seconds": (7.5, 12.0, 0.25),
    "backcourt_margin_ft": (1.5, 3.0, 0.1),
    "drive_finish_distance_ft": (4.0, 8.0, 0.2),
    "block_base": (0.20, 0.42, 0.015),
    "rebound_boxout_weight": (0.6, 1.4, 0.04),
}
KEYS = sorted(LEVERS)


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
    processes = []
    for index, policy in enumerate(candidates):
        path = WORK / f"worker-{index}" / "calibration" / "policy.json"
        path.write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        processes.append(
            subprocess.Popen(
                [str(BINARY), "--gate-only", "--calibration", str(WORK / f"worker-{index}" / "calibration")],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            )
        )
    results = []
    for process in processes:
        output = process.stdout.read().decode("utf-8", errors="replace")
        process.wait()
        report = None
        for line in output.splitlines():
            if line.startswith("GATE "):
                report = json.loads(line[5:])
                break
        if report is None:
            # 某个候选触发引擎断言：记为最差，不让整轮失败。
            results.append((1e6, {"layers": [], "passed": False,
                                  "geometric_violations": 0, "context_violations": 0}, output))
            continue
        results.append((depth_of(report), report, output))
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


def perturb(policy, rng, width):
    trial = copy.deepcopy(policy)
    for key in rng.sample(KEYS, rng.choice((1, 1, 2, 2, 3, 3))):
        low, high, step = LEVERS[key]
        step *= width
        current = get_value(trial, key)
        candidate = current + rng.choice((-step, step))
        set_value(trial, key, min(max(candidate, low), high))
    return trial


def main():
    parser = argparse.ArgumentParser(description="以 setup 修复窗口为起点的种群搜索")
    parser.add_argument("--rounds", type=int, default=200)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    prepare()
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    population = []
    for setup in SETUP_STARTS:
        policy = copy.deepcopy(base)
        policy["geometry"]["possession_setup_seconds"] = setup
        population.append(policy)
    initial = evaluate(population)
    beam = [(score, policy, stdout) for policy, (score, _, stdout) in zip(population, initial)]
    beam.sort(key=lambda entry: entry[0])
    print("起始深度：" + " ".join(f"{e[0]:.5f}" for e in beam), flush=True)

    best = beam[0]
    for round_index in range(1, args.rounds + 1):
        parents = [entry[1] for entry in beam]
        children = [perturb(parent, rng, 1.0 if i < 2 else 2.2) for i, parent in enumerate(parents)]
        measured = evaluate(children)
        candidates = list(beam) + [
            (score, child, stdout)
            for child, (score, _, stdout) in zip(children, measured)
        ]
        # 按 setup 分桶保留多样性，避免四个成员全部退回同一个起点。
        buckets: dict[str, list] = {}
        for entry in candidates:
            setup = round(entry[1]["geometry"]["possession_setup_seconds"] * 2) / 2
            buckets.setdefault(str(setup), []).append(entry)
        chosen = []
        for key in sorted(buckets, key=lambda v: min(e[0] for e in buckets[v])):
            chosen.extend(sorted(buckets[key], key=lambda e: e[0])[:2])
        chosen.sort(key=lambda entry: entry[0])
        beam = chosen[:WORKERS]
        if beam[0][0] < best[0]:
            best = beam[0]
        if round_index % 10 == 0 or beam[0][0] == 0.0:
            lines = [line for line in beam[0][2].splitlines() if line.startswith("GATE ")]
            bad = []
            if lines:
                bad = [l["criterion"] for l in json.loads(lines[0][5:])["layers"] if not l["passed"]]
            print(
                f"[{round_index:3d}] 最优深度 {beam[0][0]:.5f} 失败 {len(bad)} 条 {bad}",
                flush=True,
            )
        if beam[0][0] == 0.0:
            break

    POLICY.write_text(json.dumps(best[1], indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    final = evaluate([best[1]] * WORKERS)[0]
    source = best[2] if best[0] == 0.0 else final[2]
    FINAL_LOG.write_text(source, encoding="utf-8")
    print(
        f"最终：失败 {sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"深度 {final[0]:.5f}，几何 {final[1]['geometric_violations']}，"
        f"语境 {final[1]['context_violations']}，通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)
    print(f"setup {best[1]['geometry']['possession_setup_seconds']}", flush=True)


if __name__ == "__main__":
    main()
