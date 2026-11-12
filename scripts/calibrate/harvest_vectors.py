#!/usr/bin/env python3
"""汇总所有已经保存过的参数向量，用权威门禁逐个判定并保留最优解。

所有扫描阶段都把中间向量写进了 .work/evidence/。这些向量来自不同的局部
搜索阶段，彼此可能带有不同的优良片段。这里只做一次完整的重新判定，并按
「失败条数、越界深度」排序；结果写回主参数表与 gate-final.log。

用法：

    python3 -u scripts/calibrate/harvest_vectors.py
"""

from __future__ import annotations

import glob
import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "harvest"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
REPORT = ROOT / ".work" / "evidence" / "harvest-report.json"
WORKERS = 4


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


def failing(report):
    return sum(1 for layer in report["layers"] if not layer["passed"])


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


def evaluate(vectors):
    results = []
    for start in range(0, len(vectors), WORKERS):
        batch = vectors[start : start + WORKERS]
        processes = []
        for index, (_, (_, policy)) in enumerate(batch):
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
        print(f"已判定 {len(results)}/{len(vectors)}", flush=True)
    return results


def collect():
    paths = sorted(
        set(
            glob.glob(str(ROOT / ".work" / "**" / "policy*.json"), recursive=True)
            + glob.glob(str(ROOT / ".work" / "**" / "calibration" / "policy.json"), recursive=True)
            + [str(POLICY)]
        )
    )
    # provenance 只是来源说明，早期快照用的是纯字符串，当前加载器要求结构体。
    # 用主参数表的 provenance 统一补齐，让所有向量都能进入同一个内核。
    main_provenance = json.loads(POLICY.read_text(encoding="utf-8"))["provenance"]
    seen = {}
    for path in paths:
        if str(WORK) in path:
            continue
        try:
            policy = json.loads(Path(path).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(policy, dict):
            continue
        if "weights" not in policy or "geometry" not in policy:
            continue
        policy["provenance"] = main_provenance
        digest = hashlib.sha256(
            json.dumps(policy, sort_keys=True).encode("utf-8")
        ).hexdigest()[:12]
        if digest not in seen:
            seen[digest] = (path, policy)
    return list(seen.items())


def main():
    prepare()
    vectors = collect()
    print(f"唯一参数向量 {len(vectors)} 个", flush=True)
    results = evaluate(vectors)

    ranked = []
    for (digest, (path, _)), (report, output) in zip(vectors, results):
        if report is None:
            print(f"跳过 {digest} {path}：门禁没有输出报告", flush=True)
            continue
        ranked.append(
            {
                "digest": digest,
                "path": path,
                "failed": failing(report),
                "depth": depth_of(report),
                "failures": [
                    {"criterion": layer["criterion"], "observed": layer["observed"],
                     "relative_error": layer["relative_error"]}
                    for layer in report["layers"] if not layer["passed"]
                ],
            }
        )
    ranked.sort(key=lambda row: (row["failed"], row["depth"]))
    REPORT.write_text(json.dumps(ranked, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    for row in ranked[:10]:
        print(
            f"失败 {row['failed']} 深度 {row['depth']:.6f} {row['digest']} {row['path']}",
            flush=True,
        )

    if not ranked:
        return
    winner = ranked[0]
    digest, (_, policy) = next(
        (key, value) for key, value in vectors if key == winner["digest"]
    )
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    winner_index = next(
        index for index, (key, _) in enumerate(vectors) if key == winner["digest"]
    )
    winning_output = results[winner_index][1]
    FINAL_LOG.write_text(winning_output, encoding="utf-8")
    print(f"已保留 {digest}：失败 {winner['failed']} 条，深度 {winner['depth']:.6f}", flush=True)


if __name__ == "__main__":
    main()
