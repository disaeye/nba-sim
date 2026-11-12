#!/usr/bin/env python3
"""资源与性能测量（full_simulation_plan.md §14、docs/quality.md §4）。

每个测量进程都走 `measure` 二进制的生产路径，进程级峰值常驻内存、
运行时间和退出状态由 `wait4` 的 rusage 给出；事件数、动作数、加时数
和每 tick 堆分配由测量二进制逐场输出。

场景里的「频繁犯规」「密集传球」「多次暂停」「录像复核」从一批完整
比赛里按实际事件计数挑选极值样本，选择发生在测量之后，不注入动作
结果，也不改规则与参数。

每个子进程受 2 GiB 地址空间上限约束：超限直接终止并由本脚本判定失败。

用法：

    python3 scripts/measure_resources.py --report
    python3 scripts/measure_resources.py --check
"""

from __future__ import annotations

import argparse
import json
import os
import resource
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MEASURE = ROOT / "target" / "release" / "measure"
ARTIFACT = ROOT / "docs" / "dev" / "evidence" / "performance.json"
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 常规场与 FIBA 场的批次：极值样本从这批完整比赛里挑。
BATCH = 24


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def run_measure(args: list[str]) -> tuple[list[dict], dict]:
    """跑一个测量进程，返回逐场结果与进程级资源读数。"""
    command = [str(MEASURE), *args]
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        preexec_fn=limit_address_space,
    )
    #  stderr 并入 stdout：只需读空一个管道，既不会因两个管道互相等待
    #  而卡住，也不需要 communicate() 提前回收子进程。
    output_bytes = process.stdout.read()
    # `wait4` 给出这一个子进程的 rusage；`RUSAGE_CHILDREN` 是所有子进程
    # 中的最大值，后续场景会复用前面场景的读数，不能当作单次峰值。
    _, status, usage = os.wait4(process.pid, 0)
    exit_code = os.waitstatus_to_exitcode(status)
    process.returncode = exit_code
    elapsed = time.monotonic() - started
    if exit_code != 0:
        raise SystemExit(
            f"measure failed ({exit_code}): {' '.join(command)}\n"
            f"{output_bytes.decode('utf-8', errors='replace')[-2000:]}"
        )
    samples = []
    for line in output_bytes.decode("utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        # 单场入口带 `MEASURE ` 前缀，扫描模式每行是裸 JSON。
        if line.startswith("MEASURE "):
            line = line[len("MEASURE ") :]
        samples.append(json.loads(line))
    peak_kb = usage.ru_maxrss
    return samples, {
        "command": " ".join(args),
        "elapsed_seconds": round(elapsed, 3),
        "peak_rss_kb": peak_kb,
        "peak_rss_bytes": peak_kb * 1024,
        "exit_code": exit_code,
        "address_space_limit_bytes": ADDRESS_SPACE_LIMIT,
        "samples": len(samples),
    }


def pick(samples: list[dict], field: str) -> dict:
    if not samples:
        raise SystemExit(f"场景样本为空，无法按 {field} 挑选")
    return max(samples, key=lambda sample: sample[field])


def scenario(name: str, selection: str, sample: dict, process: dict) -> dict:
    return {
        "name": name,
        "selection": selection,
        "sample": sample,
        "process": process,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="资源与性能测量")
    parser.add_argument(
        "--report", action="store_true", help="运行测量、写出工件并打印摘要"
    )
    parser.add_argument(
        "--check", action="store_true", help="只校验已有工件，不重新测量"
    )
    args = parser.parse_args()

    if not MEASURE.is_file():
        raise SystemExit(f"缺少测量二进制：{MEASURE.relative_to(ROOT)}")

    if args.check:
        payload = json.loads(ARTIFACT.read_text(encoding="utf-8"))
        failures = payload.get("failures", [])
        if failures:
            for failure in failures:
                print(f"❌ {failure}", file=sys.stderr)
            return 1
        print(f"资源工件通过：{len(payload['scenarios'])} 个场景")
        return 0

    scenarios: list[dict] = []
    failures: list[str] = []

    regular, regular_process = run_measure(
        ["--scan", f"1..{BATCH + 1}", "--league", "nba"]
    )
    scenarios.append(
        scenario(
            "regular",
            "NBA 完整比赛的批次中位数",
            sorted(regular, key=lambda sample: sample["events"])[len(regular) // 2],
            regular_process,
        )
    )
    for name, field, label in (
        ("frequent_fouls", "fouls", "犯规事件最多的一场"),
        ("dense_pass", "passes", "传球事件最多的一场"),
        ("many_timeouts", "timeouts", "暂停事件最多的一场"),
        ("replay_review", "reviews", "复核事件最多的一场"),
    ):
        sample = pick(regular, field)
        scenarios.append(scenario(name, label, sample, regular_process))
        if sample[field] <= 0:
            failures.append(f"场景 {name} 没有对应事件：{field}=0")

    overtime, overtime_process = run_measure(
        ["--scan", "1..5", "--league", "nba", "--scenario", "overtime"]
    )
    overtime_sample = pick(overtime, "overtimes")
    scenarios.append(
        scenario("consecutive_overtime", "连续加时场景里加时数最多的一场", overtime_sample, overtime_process)
    )
    if overtime_sample["overtimes"] == 0:
        failures.append("连续加时场景没有产生加时")

    fiba, fiba_process = run_measure(
        ["--scan", f"1..{BATCH + 1}", "--league", "fiba"]
    )
    scenarios.append(
        scenario(
            "fiba_profile",
            "FIBA 完整比赛的批次中位数",
            sorted(fiba, key=lambda sample: sample["events"])[len(fiba) // 2],
            fiba_process,
        )
    )

    for entry in scenarios:
        sample = entry["sample"]
        if not sample["finished"]:
            failures.append(f"场景 {entry['name']} 的比赛没有正常终场")
        if sample["allocations_per_tick"] <= 0:
            failures.append(f"场景 {entry['name']} 没有记录每 tick 分配")
        process = entry["process"]
        if process["peak_rss_bytes"] > ADDRESS_SPACE_LIMIT:
            failures.append(f"场景 {entry['name']} 峰值内存超过 2 GiB 上限")

    payload = {
        "version": 1,
        "address_space_limit_bytes": ADDRESS_SPACE_LIMIT,
        "batch_size": BATCH,
        "scenarios": scenarios,
        "failures": failures,
        "passed": not failures,
    }
    ARTIFACT.parent.mkdir(parents=True, exist_ok=True)
    ARTIFACT.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )

    if args.report:
        for entry in scenarios:
            sample = entry["sample"]
            print(
                f"{entry['name']:22s} ticks {sample['ticks']:7d} "
                f"events {sample['events']:6d} actions {sample['actions']:5d} "
                f"overtimes {sample['overtimes']:2d} "
                f"alloc/tick {sample['allocations_per_tick']:.3f} "
                f"peak {entry['process']['peak_rss_bytes'] // (1024 * 1024)} MiB "
                f"{entry['process']['elapsed_seconds']:.2f}s"
            )
        print(f"工件：{ARTIFACT.relative_to(ROOT)}")
    if failures:
        for failure in failures:
            print(f"❌ {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
