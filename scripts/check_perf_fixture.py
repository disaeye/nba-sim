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
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    band = fixture["throughput_ticks_per_second"]
    seed42 = measure(20000, 42)
    print(f"seed_42 throughput = {seed42:.0f} ticks/s "
          f"(fixture band [{band['band_min']:.0f}, {band['band_max']:.0f}])")
    if args.write_budget:
        band["measured"]["seed_42"] = round(seed42, 1)
        FIXTURE.write_text(json.dumps(fixture, ensure_ascii=False, indent=2) + "\n",
                           encoding="utf-8")
        print("fixture 已更新（须附重测证据提交）")
        return
    if not band["band_min"] <= seed42 <= band["band_max"]:
        sys.exit(
            f"❌ 吞吐 {seed42:.0f} 越出基准带 [{band['band_min']:.0f}, {band['band_max']:.0f}]。\n"
            "   碰撞/行为类改动改变每 tick 成本属正常演化：重测全部种子后用 --write-budget\n"
            "   重冻结，并在提交信息附前后对比证据（quality.md §4.1）。"
        )
    print("✅ 吞吐基准带内")


if __name__ == "__main__":
    main()
