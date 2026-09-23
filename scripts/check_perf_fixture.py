import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "scripts" / "perf_fixture.json"


def measure(ticks: int, seed: int) -> float:
    # 直接复用 tests/throughput_bench.rs 的测量函数：--exact 只跑指定用例，
    # 从 stdout 解析 throughput= 行。
    cmd = [
        "cargo", "test", "--release", "-p", "nba-engine",
        "--test", "throughput_bench", "--", "--nocapture",
        f"bench_seed{seed}",
    ]
    result = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    for line in result.stdout.splitlines():
        if line.startswith(f"seed{seed}:") and "throughput=" in line:
            return float(line.split("throughput=")[1].split()[0])
    sys.exit(f"未能解析 seed{seed} 的吞吐输出：\n{result.stdout[-2000:]}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--write-budget", action="store_true",
                        help="把当前实测写入 fixture（附证据后的人工批次）")
    args = parser.parse_args()
    # pi-lens-ignore: unchecked-throwing-call-python
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    band = fixture["throughput_ticks_per_second"]
    measured = dict(band.get("measured", {}))
    # fixture 里登记了几个种子就测几个（当前 seed_42 / seed_1），
    # 任何一个越带都判失败，而不是只看第一个。
    failures = []
    for seed_key in sorted(measured):
        # pi-lens-ignore: unchecked-throwing-call-python
        seed = int(seed_key.removeprefix("seed_"))
        throughput = measure(20000, seed)
        print(f"{seed_key} throughput = {throughput:.0f} ticks/s "
              f"(fixture band [{band['band_min']:.0f}, {band['band_max']:.0f}])")
        if args.write_budget:
            measured[seed_key] = round(throughput, 1)
        elif not band["band_min"] <= throughput <= band["band_max"]:
            failures.append(
                f"{seed_key} 吞吐 {throughput:.0f} 越出基准带 "
                f"[{band['band_min']:.0f}, {band['band_max']:.0f}]"
            )
    if args.write_budget:
        band["measured"] = measured
        FIXTURE.write_text(json.dumps(fixture, ensure_ascii=False, indent=2) + "\n",
                           encoding="utf-8")
        print("fixture 已更新（须附重测证据提交）")
        return
    if failures:
        sys.exit(
            "❌ " + "；".join(failures) + "。\n"
            "   碰撞/行为类改动改变每 tick 成本属正常演化：重测全部种子后用 --write-budget\n"
            "   重冻结，并在提交信息附前后对比证据（quality.md §4.1）。"
        )
    print("✅ 吞吐基准带内")


if __name__ == "__main__":
    main()
