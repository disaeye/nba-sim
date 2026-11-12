#!/usr/bin/env python3
"""汇总全部历史参数向量，在最终行为上求解 39 条门禁。

每个历史向量先按当前主参数表补齐新增键；出手模型保真参数固定为当前
外部基准验证过的值。评分来自权威门禁的越界深度，改善立即写进独立最优
向量文件。最后检查被包夹护球是否由生产路径产出，再写回主参数表。

用法：

    python3 -u scripts/calibrate/harvest_final_solution.py
"""

from __future__ import annotations

import copy
import hashlib
import json
import os
import resource
import shutil
import subprocess
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "final-solution"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-final-solution.json"
REPORT = ROOT / ".work" / "evidence" / "final-solution-report.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024
DEADLINE_MINUTES = 22

MODEL_PREFIXES = (
    "shot_quality_skill_",
    "shot_phase_sigma_",
    "shot_aim_",
    "shot_rim_",
    "shot_fatigue_",
    "shot_distance_decay_",
    "shot_quality_contest_",
    "shot_gather_sigma_",
)
MODEL_EXACT = {
    "shot_quality_contest_sensitivity",
    "shot_distance_decay_three_scale_ft",
    "shoot_off_dribble_penalty",
}
BONUSES = [0.3, 0.4, 0.5, 0.6, 0.8, 1.0]
REFINE_LEVERS = [
    "cut_tendency",
    "cut_lane_scale",
    "cut_positional_bonus",
    "drive_base",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "bad_pass_rate",
    "gather_travel_rate",
    "shooting_foul_rate_two_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "transition_take_window_seconds",
    "seconds_per_possession",
    "reach_in_foul_rate",
    "hand_check_rate",
    "play_call_rate",
]
REFINE_FACTORS = [0.90, 0.95, 0.98, 0.995, 1.0, 1.005, 1.02, 1.05, 1.10]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def is_model_key(key):
    return key.startswith(MODEL_PREFIXES) or key in MODEL_EXACT


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        observed = layer["observed"]
        if detail.startswith("relative error"):
            low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        elif detail.startswith("上限"):
            bound = float(detail.removeprefix("上限 "))
            total += max(0.0, (observed - bound) / max(abs(bound), 1e-6))
        elif detail.startswith("下限"):
            bound = float(detail.removeprefix("下限 "))
            total += abs(min(0.0, (observed - bound) / max(abs(bound), 1e-6)))
        else:
            total += abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth_of(report), failed)


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


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
    production = WORK / "production" / "calibration"
    production.mkdir(parents=True)
    (WORK / "production" / "tactics").symlink_to(
        TACTICS, target_is_directory=True
    )


def evaluate(candidates, deadline):
    results = []
    for start in range(0, len(candidates), WORKERS):
        if time.monotonic() > deadline:
            results.extend([(None, "")] * (len(candidates) - len(results)))
            break
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
        if start and (start // WORKERS) % 10 == 0:
            print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
    return results


def normalize(candidate, current, bonus):
    result = copy.deepcopy(current)
    for section in ("weights", "geometry"):
        if section not in candidate:
            continue
        for key, value in candidate[section].items():
            if key not in result[section] or is_model_key(key):
                continue
            if isinstance(value, (int, float)) and not isinstance(value, bool):
                result[section][key] = value
    result["weights"]["trap_protection_bonus"] = bonus
    return result


def collect(current, default_bonus):
    paths = sorted(
        set(
            [str(POLICY)]
            + [str(path) for path in (ROOT / ".work").rglob("*.json")]
        )
    )
    vectors = {}
    for text_path in paths:
        path = Path(text_path)
        if str(WORK) in str(path):
            continue
        try:
            candidate = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(candidate, dict):
            continue
        if "weights" not in candidate or "geometry" not in candidate:
            continue
        normalized = normalize(candidate, current, default_bonus)
        digest = hashlib.sha256(
            json.dumps(normalized, sort_keys=True).encode("utf-8")
        ).hexdigest()[:12]
        vectors.setdefault(digest, (str(path), normalized))
    return list(vectors.items())


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


def write_best(policy, stdout=""):
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    if stdout:
        FINAL_LOG.write_text(stdout, encoding="utf-8")


def production_events(policy):
    calibration = WORK / "production" / "calibration"
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (calibration / "policy.json").write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    output = subprocess.run(
        [str(PROBE), str(calibration)],
        cwd=ROOT, capture_output=True, text=True,
        preexec_fn=limit_address_space,
    ).stdout.strip().splitlines()
    events, seeds = (int(value) for value in output[-1].split())
    return events, seeds


def main():
    prepare()
    deadline = time.monotonic() + DEADLINE_MINUTES * 60.0
    current = json.loads(POLICY.read_text(encoding="utf-8"))
    vectors = collect(current, BONUSES[1])
    print(f"唯一参数向量 {len(vectors)} 个", flush=True)

    ranked = []
    results = evaluate([policy for _, (_, policy) in vectors], deadline)
    for (digest, (path, policy)), (report, stdout) in zip(vectors, results):
        if report is None:
            continue
        ranked.append((score_of(report), digest, path, policy, stdout))
    ranked.sort(key=lambda row: row[0])
    if not ranked:
        raise SystemExit("没有可用的参数向量")
    print(f"最优历史起点 {ranked[0][1]}：{describe(ranked[0][0])}", flush=True)

    # 对前 16 个起点扫描最小扰动的被包夹护球先验。
    joint = []
    for score, digest, path, policy, _ in ranked[:16]:
        for bonus in BONUSES:
            trial = copy.deepcopy(policy)
            trial["weights"]["trap_protection_bonus"] = bonus
            joint.append((digest, path, bonus, trial))
    joint_results = evaluate([row[3] for row in joint], deadline)
    combined = []
    for (digest, path, bonus, policy), (report, stdout) in zip(joint, joint_results):
        if report is None:
            continue
        combined.append((score_of(report), digest, path, bonus, policy, stdout))
    combined.sort(key=lambda row: row[0])
    if not combined:
        raise SystemExit("先验扫描没有可用结果")
    best, digest, path, bonus, best_policy, best_stdout = combined[0]
    write_best(best_policy, best_stdout)
    print(
        f"先验起点 {digest} bonus={bonus:.1f} {path}：{describe(best)}",
        flush=True,
    )

    # 从最优起点做两轮细步长坐标下降。
    for sweep in range(1, 3):
        if best[0] == 0.0 or time.monotonic() > deadline:
            break
        improved = False
        for key in REFINE_LEVERS:
            if best[0] == 0.0 or time.monotonic() > deadline:
                break
            value = get_value(best_policy, key)
            candidates = []
            labels = []
            for factor in REFINE_FACTORS:
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, value * factor)
                candidates.append(trial)
                labels.append(f"{key}={value * factor:.6f}")
            for trial, label, (report, stdout) in zip(
                candidates, labels, evaluate(candidates, deadline)
            ):
                if report is None:
                    continue
                score = score_of(report)
                if score < best:
                    best, best_policy = score, trial
                    write_best(best_policy, stdout)
                    improved = True
                    print(f"[细扫 {sweep}] {label} {describe(best)}", flush=True)
        print(f"[细扫 {sweep}] 结束 {describe(best)}", flush=True)
        if not improved:
            break

    # 被包夹护球必须由完整比赛的生产路径产出。
    events, seeds = production_events(best_policy)
    if events == 0 and time.monotonic() < deadline:
        for candidate_bonus in sorted(set(BONUSES + [1.4, 2.0, 2.8])):
            trial = copy.deepcopy(best_policy)
            trial["weights"]["trap_protection_bonus"] = candidate_bonus
            report, stdout = evaluate([trial], deadline)[0]
            if report is None:
                continue
            score = score_of(report)
            candidate_events, candidate_seeds = production_events(trial)
            print(
                f"先验 {candidate_bonus:.1f}：{describe(score)}，被包夹护球 {candidate_events} 次",
                flush=True,
            )
            if candidate_events > 0 and (score < best or best[1] != 0):
                best, best_policy = score, trial
                events, seeds = candidate_events, candidate_seeds
                write_best(best_policy, stdout)
            if events > 0 and best[1] == 0:
                break

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    REPORT.write_text(
        json.dumps(
            {
                "score": {"depth": best[0], "failed": best[1]},
                "trap_protection_events": events,
                "trap_protection_seeds": seeds,
                "historical_top": [
                    {"digest": d, "path": p, "depth": s[0], "failed": s[1]}
                    for s, d, p, _, _ in ranked[:10]
                ],
            },
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    print(
        f"最终：{describe(best)}，被包夹护球 {events} 次 / {seeds} 场",
        flush=True,
    )


if __name__ == "__main__":
    main()
