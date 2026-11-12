#!/usr/bin/env python3
"""最终行为上的门禁收敛。

约束：出手模型保真参数固定为 HEAD 值，`trap_protect` 必须由生产决策链产
出，其余因果杠杆按权威门禁的越界深度做坐标下降。搜索先汇总所有已保存的
参数向量，再对最优向量做小幅扫描；每个候选都只写进独立校准目录，主参数表
只在接受改善时更新。

用法：

    python3 -u scripts/calibrate/final_refit.py --sweeps 4
"""

from __future__ import annotations

import argparse
import copy
import glob
import hashlib
import json
import os
import resource
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "final-refit"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-final-refit-best.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
REPORT = ROOT / ".work" / "evidence" / "final-refit-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 出手模型保真参数由外部基准测试钉住，不参与分布搜索。
SHOT_MODEL_PREFIXES = (
    "shot_quality_skill_",
    "shot_phase_sigma_",
    "shot_aim_",
    "shot_rim_",
    "shot_fatigue_",
    "shot_distance_decay_",
    "shot_quality_contest_",
    "shot_gather_sigma_",
)
SHOT_MODEL_EXACT = {
    "shot_quality_contest_sensitivity",
    "shot_distance_decay_three_scale_ft",
    "shoot_off_dribble_penalty",
}
TRAP_BONUS = 0.6

# 六条越界判据的直接因果杠杆。
LEVERS = [
    "cut_tendency",
    "cut_lane_scale",
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
]
FACTORS = [0.80, 0.90, 0.96, 1.0, 1.04, 1.10, 1.25]


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
            total += abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def score_of(report):
    return (
        depth_of(report),
        sum(1 for layer in report["layers"] if not layer["passed"]),
    )


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


def read_head_policy():
    return json.loads(
        subprocess.check_output(
            ["git", "show", "HEAD:data/calibration/policy.json"],
            cwd=ROOT,
            text=True,
        )
    )


def normalize(policy, current, head):
    result = copy.deepcopy(policy)
    for section in ("weights", "geometry"):
        result.setdefault(section, {})
        # 当前内核新增的策略键必须存在：缺字段就地失败是生产加载器的契约。
        for key, value in current[section].items():
            result[section].setdefault(key, value)
        # 出手模型保真参数固定为外部基准验证过的 HEAD 值。
        for key, value in head[section].items():
            if key.startswith(SHOT_MODEL_PREFIXES) or key in SHOT_MODEL_EXACT:
                result[section][key] = value
    result["provenance"] = current["provenance"]
    result["weights"]["trap_protection_bonus"] = TRAP_BONUS
    return result


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


def persist(policy, stdout):
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def collect_vectors(current, head):
    paths = sorted(
        set(
            glob.glob(str(ROOT / ".work" / "evidence" / "**" / "policy*.json"), recursive=True)
            + glob.glob(str(ROOT / ".work" / "**" / "calibration" / "policy.json"), recursive=True)
            + [str(POLICY)]
        )
    )
    vectors = {}
    for path in paths:
        if str(WORK) in path:
            continue
        try:
            policy = json.loads(Path(path).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(policy, dict) or "weights" not in policy or "geometry" not in policy:
            continue
        normalized = normalize(policy, current, head)
        digest = hashlib.sha256(
            json.dumps(normalized, sort_keys=True).encode("utf-8")
        ).hexdigest()[:12]
        vectors.setdefault(digest, (path, normalized))
    return list(vectors.items())


def production_events(policy):
    calibration = WORK / "production" / "calibration"
    calibration.mkdir(parents=True, exist_ok=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    tactics_link = WORK / "production" / "tactics"
    if not tactics_link.exists():
        tactics_link.symlink_to(TACTICS, target_is_directory=True)
    (calibration / "policy.json").write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    output = subprocess.run(
        [str(PROBE), str(calibration)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        preexec_fn=limit_address_space,
    ).stdout.strip().splitlines()
    events, seeds = (int(value) for value in output[-1].split())
    return events, seeds


def main():
    parser = argparse.ArgumentParser(description="最终行为上的门禁收敛")
    parser.add_argument("--sweeps", type=int, default=4)
    parser.add_argument("--from-current", action="store_true",
                        help="跳过历史向量排名，直接从主参数表开始")
    args = parser.parse_args()

    prepare()
    current = json.loads(POLICY.read_text(encoding="utf-8"))
    head = read_head_policy()
    if args.from_current:
        vectors = [("current", (str(POLICY), normalize(current, current, head)))]
    else:
        vectors = collect_vectors(current, head)
    print(f"唯一参数向量 {len(vectors)} 个", flush=True)

    ranked = []
    batch_results = evaluate([policy for _, (_, policy) in vectors])
    for (digest, (path, policy)), (report, stdout) in zip(vectors, batch_results):
        if report is None:
            continue
        score = score_of(report)
        ranked.append((score, digest, path, policy, stdout))
    ranked.sort(key=lambda row: row[0])
    REPORT.write_text(
        json.dumps(
            [
                {"digest": digest, "path": path, "depth": score[0], "failed": score[1]}
                for score, digest, path, _, _ in ranked[:20]
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    if not ranked:
        raise SystemExit("没有可用的参数向量")
    best, digest, path, best_policy, best_stdout = ranked[0]
    persist(best_policy, best_stdout)
    print(f"起点 {digest} {path}：{describe(best)}", flush=True)

    for sweep in range(1, args.sweeps + 1):
        if best[1] == 0:
            break
        improved = False
        for key in LEVERS:
            current_value = get_value(best_policy, key)
            candidates = []
            values = []
            for factor in FACTORS:
                value = current_value * factor
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, value)
                candidates.append(trial)
                values.append(value)
            results = evaluate(candidates)
            for trial, value, (report, stdout) in zip(candidates, values, results):
                if report is None:
                    continue
                score = score_of(report)
                if score < best:
                    best = score
                    best_policy = trial
                    persist(best_policy, stdout)
                    improved = True
                    print(
                        f"[轮 {sweep}] {key} -> {value:.6f} {describe(best)}",
                        flush=True,
                    )
        print(f"[轮 {sweep}] 结束：{describe(best)}", flush=True)
        if not improved:
            break

    # 被包夹护球的生产证据是最终向量的硬约束：门禁全通过且真实产出。
    events, seeds = production_events(best_policy)
    print(f"生产证据：被包夹护球 {events} 次 / {seeds} 场", flush=True)
    for bonus in [0.4, 0.5, 0.6, 0.8, 1.0, 1.4, 2.0, 2.8]:
        if best[1] == 0 and events > 0:
            break
        trial = copy.deepcopy(best_policy)
        trial["weights"]["trap_protection_bonus"] = bonus
        report, stdout = evaluate([trial])[0]
        if report is None:
            continue
        score = score_of(report)
        candidate_events, candidate_seeds = production_events(trial)
        print(
            f"先验 {bonus:.1f}：{describe(score)}，被包夹护球 {candidate_events} 次",
            flush=True,
        )
        acceptable = candidate_events > 0 and (
            score < best or (events == 0 and best[1] != 0) or
            (candidate_events > 0 and score[1] == 0)
        )
        if acceptable:
            best, best_policy = score, trial
            persist(best_policy, stdout)
            events, seeds = candidate_events, candidate_seeds
    print(f"最终：{describe(best)}，被包夹护球 {events} 次 / {seeds} 场", flush=True)


if __name__ == "__main__":
    main()
