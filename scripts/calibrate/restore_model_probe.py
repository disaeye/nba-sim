#!/usr/bin/env python3
"""把历史最优向量的出手模型保真键恢复到基准值，测门禁与生产证据。

外部基准 `shot_geometry_benchmark` 在无防守夹具上只依赖出手类型倍率、
阶段倍率与 `shot_aim_sigma_scale`；`shot_quality_contest_sensitivity` 在
零干扰夹具下不影响预测命中率，因此保留为可校准杠杆。

用法：

    python3 -u scripts/calibrate/restore_model_probe.py
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "restore-probe"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-restore-probe.json"

HISTORICAL = ROOT / ".work" / "evidence" / "policy-before-close-edges.json"
# 外部基准钉住的键：这些键取当前主参数表中已经通过基准测试的值。
BENCHMARK_KEYS = [
    "shot_aim_sigma_scale",
    "shot_distance_decay_three_scale_ft",
    "shot_phase_sigma_catch",
    "shot_phase_sigma_dribble",
    "shot_phase_sigma_drive",
    "shot_quality_skill_rim",
    "shot_quality_skill_near",
    "shot_quality_skill_mid",
    "shot_quality_skill_three",
    "shot_fatigue_penalty",
    "shot_rim_in_probability",
    "shot_rim_contact_band",
]


def main():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)

    current = json.loads(POLICY.read_text(encoding="utf-8"))
    candidate = json.loads(HISTORICAL.read_text(encoding="utf-8"))
    for key in BENCHMARK_KEYS:
        for section in ("weights", "geometry"):
            if key in current[section]:
                candidate.setdefault(section, {})[key] = current[section][key]
    candidate["provenance"] = current["provenance"]
    for key, value in current["weights"].items():
        candidate["weights"].setdefault(key, value)
    for key, value in current["geometry"].items():
        candidate["geometry"].setdefault(key, value)
    candidate["weights"]["trap_protection_bonus"] = 0.6

    (calibration / "policy.json").write_text(
        json.dumps(candidate, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    BEST_VECTOR.write_text(
        json.dumps(candidate, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )

    gate = subprocess.run(
        [str(BINARY), "--gate-only", "--calibration", str(calibration)],
        cwd=ROOT, capture_output=True, text=True,
    )
    report = None
    for line in gate.stdout.splitlines():
        if line.startswith("GATE "):
            report = json.loads(line[5:])
            break
    if report is None:
        raise SystemExit(f"门禁没有输出报告：{gate.stdout[-1000:]} {gate.stderr[-1000:]}")
    print(f"门禁失败 {sum(1 for x in report['layers'] if not x['passed'])} 条")
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.4f}")
    production = subprocess.run(
        [str(PROBE), str(calibration)], cwd=ROOT, capture_output=True, text=True,
    )
    print(f"被包夹护球生产：{production.stdout.strip().splitlines()[-1]}")
    print(f"候选向量：{BEST_VECTOR}")


if __name__ == "__main__":
    main()
