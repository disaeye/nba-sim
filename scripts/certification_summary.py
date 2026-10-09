"""认证结论汇总（validation.md §8 认证门槛的工件视图）。

读 validation 输出目录的工件（scenario_results / statistical_results /
mechanism_results / batch_statistical_results / branch_coverage），产出
certification.json：五层状态、认证范围限制、明确的结论段。
判定绝对如实：缺陷不淡化、证据不足不冒充通过。
"""

import argparse
import json
from pathlib import Path


def load(path: Path):
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as error:
        # 快崩语义：工件损坏时带文件名与原因直接失败，不静默跳过。
        raise SystemExit(f"工件损坏 {path}: {error}") from error


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("run_dir", help="validation 输出目录")
    args = parser.parse_args()
    run_dir = Path(args.run_dir)

    scenario = load(run_dir / "scenario_results.json")
    stat_single = load(run_dir / "statistical_results.json")
    mech = load(run_dir / "mechanism_results.json")
    batch_stat = load(run_dir / "batch_statistical_results.json")
    coverage = load(run_dir / "branch_coverage.json")

    certification: dict[str, object] = {"run_dir": str(run_dir)}

    # 第 1/2 层：场景与重建（单场验证的场景矩阵；verdict 为小写 pass）。
    if scenario:
        outcomes = scenario if isinstance(scenario, list) else scenario.get("outcomes", [])
        passes = sum(
            1
            for o in outcomes
            if str(o.get("verdict", "")).lower() == "pass"
        )
        certification["layer1_flow"] = {
            "scenarios_total": len(outcomes),
            "scenarios_pass": passes,
            "status": "pass" if outcomes and passes == len(outcomes) else "defect",
        }

    # 第 3/5 层：统计与组合（正式判定看批量；单场只作诊断）。
    if batch_stat:
        crit = batch_stat.get("criterion_verdicts", {})
        stat_defects = sorted(k for k, v in crit.items() if v == "Defect")
        stat_insufficient = sorted(k for k, v in crit.items() if v == "InsufficientEvidence")
        stat_pass = sorted(k for k, v in crit.items() if v == "Pass")
        certification["layer3_5_distribution"] = {
            "batch_games": batch_stat.get("n_games"),
            "scope": batch_stat.get("scope"),
            "pass": stat_pass,
            "defects": stat_defects,
            "insufficient": stat_insufficient,
            "status": "defect"
            if stat_defects
            else ("pass" if not stat_insufficient else "insufficient"),
            "defect_detail": [
                {
                    "criterion_id": v["criterion_id"],
                    "metric": v["metric"],
                    "simulated": v["simulated_mean"],
                    "reference": v["reference_value"],
                    "allowed_delta": v["allowed_delta"],
                }
                for v in batch_stat.get("verdicts", [])
                if v["verdict"] == "Defect"
            ],
        }
    elif stat_single:
        certification["layer3_5_distribution"] = {
            "status": "diagnostic_only",
            "note": "仅单场验证（无批量统计判定）；单场结果只作诊断，不作认证",
            "verdicts": stat_single.get("verdicts", []),
        }

    # 第 4 层：机制实验。
    if mech:
        crit = {v["criterion_id"]: v for v in mech.get("verdicts", [])}
        covered = [
            cid
            for cid, v in crit.items()
            if v.get("reason_code") != "experiment_spec_missing"
        ]
        defects = [cid for cid, v in crit.items() if v.get("verdict") == "Defect"]
        certification["layer4_mechanism"] = {
            "experiments_run": len(mech.get("experiments", [])),
            "criteria_with_experiments": sorted(covered),
            "defects": sorted(defects),
            "status": "defect" if defects else "insufficient",
            "note": (
                "存在档位非单调缺陷"
                if defects
                else "已覆盖实验的叶项当前 InsufficientEvidence（试验功效不足）；"
                "其余叶项无实验规格"
            ),
        }

    # 覆盖与认证范围限制。
    if coverage:
        certification["branch_coverage"] = {
            "sampled": coverage.get("sampled"),
            "scenario_only": coverage.get("scenario_only"),
            "uncovered": coverage.get("uncovered"),
        }
        certification["certification_limits"] = coverage.get("certification_limits", [])

    # 总体结论（不美化）：任一层有 Defect → defects_found。
    statuses = [
        section["status"]
        for section in certification.values()
        if isinstance(section, dict) and "status" in section
    ]
    if any(s == "defect" for s in statuses):
        certification["certification_status"] = "defects_found"
    elif any(s == "insufficient" for s in statuses):
        certification["certification_status"] = "insufficient_evidence"
    else:
        certification["certification_status"] = "pass"

    out = run_dir / "certification.json"
    out.write_text(json.dumps(certification, ensure_ascii=False, indent=1) + "\n")
    print(f"认证结论: {certification['certification_status']}")
    print(f"工件: {out}")


if __name__ == "__main__":
    main()
