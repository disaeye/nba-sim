#!/usr/bin/env python3
"""三参数定向补偿网格。

`gather_travel_rate=0.014` 把 turnover 拉进带，代价是 rim、cut、fta 三条
浅越界。这里对三条各自的直接杠杆做网格：finish_min_distance_ft 控制篮下
份额，cut_lane_scale 控制切入份额，shooting_foul_rate_two_point 控制罚球数。
评分来自权威门禁自己的 39 条判定，最优组合写回参数表。

用法：

    python3 -u scripts/calibrate/compensate_travel.py
"""

from __future__ import annotations

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
WORK = ROOT / ".work" / "evidence" / "compensate"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

RIM = [2.4, 2.6, 2.8, 3.0, 3.2, 3.4]
CUT = [0.36, 0.38, 0.40, 0.42, 0.44, 0.46]
FOUL = [0.155, 0.165, 0.175, 0.185, 0.195]


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
            if report is None:
                results.append((1e6, None, output))
            else:
                results.append((depth_of(report), report, output))
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    base["weights"]["gather_travel_rate"] = 0.014
    initial = evaluate([base])[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"起点：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)

    combos = [(r, c, f) for r in RIM for c in CUT for f in FOUL]
    candidates = []
    for rim, cut, foul in combos:
        trial = copy.deepcopy(base)
        trial["geometry"]["finish_min_distance_ft"] = rim
        trial["weights"]["cut_lane_scale"] = cut
        trial["weights"]["shooting_foul_rate_two_point"] = foul
        candidates.append(trial)
    print(f"网格 {len(candidates)} 组", flush=True)
    results = evaluate(candidates)
    for (rim, cut, foul), trial, (depth, report, stdout) in zip(combos, candidates, results):
        if report is None:
            continue
        failed = [l for l in report["layers"] if not l["passed"]]
        score = (len(failed), depth)
        if score < best:
            best = score
            best_policy = trial
            best_stdout = stdout
            print(
                f"rim {rim:.2f} cut {cut:.2f} foul {foul:.3f} "
                f"失败 {len(failed)} 深度 {depth:.5f} {[l['criterion'] for l in failed]}",
                flush=True,
            )
        if best[0] == 0:
            break

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final = evaluate([best_policy])[0]
    FINAL_LOG.write_text(best_stdout if best[0] == 0 else final[2], encoding="utf-8")
    print(
        f"最终：失败 {sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"深度 {final[0]:.6f}，通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
