# 测试目标并行执行器（scripts/par_test.py，正式脚本）。
# 背景：`cargo test --release --workspace` 由 cargo 自身调度，测试目标（单个
# 测试文件一个二进制）按依赖顺序串行执行；4 核机器上全部目标累计超过 1000 秒。
# 本脚本把「目标」作为调度单元：先收集全部测试二进制，再按 CPU 核数并行执行，
# 每个目标一个进程（二进制内测试默认仍并行，受 RUST_TEST_THREADS 约束）。
#
# 输出：每个目标一行耗时与结果，最终打印总 wall time 与失败目标清单；
# 每个目标的完整输出写入 .work/par_test_out/<目标>.log 供失败时检查。
import argparse
import concurrent.futures
import json
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent


def collect_binaries():
    # 与 cargo test --workspace 相同的目标集合：每个集成测试文件一个二进制，
    # 外加各 crate 的单元测试（lib 二进制）。
    out = subprocess.run(
        ["cargo", "test", "--release", "--workspace", "--no-run", "--message-format=json"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    bins = []
    for line in out.stdout.splitlines():
        if '"reason":"compiler-artifact"' not in line and '"reason": "compiler-artifact"' not in line:
            continue
        obj = json.loads(line)
        profile = obj.get("profile", {})
        if not profile.get("test"):
            continue
        exe = obj.get("executable")
        if exe:
            # 目标名取 package_id 里的包名 + executable 文件名，用于展示与排序
            name = f"{obj.get('target', {}).get('name', '?')}"
            bins.append((name, exe))
    return bins


def run_one(pkg, exe, timeout, env):
    t0 = time.monotonic()
    try:
        proc = subprocess.run(
            [exe, "--test-threads", env["RUST_TEST_THREADS_PER_BIN"]],
            cwd=ROOT, capture_output=True, text=True, timeout=timeout,
        )
        dur = time.monotonic() - t0
        ok = proc.returncode == 0
        tail = [l for l in proc.stdout.splitlines() if "test result" in l]
        return pkg, exe, dur, ok, tail[-1] if tail else "", proc.stdout, proc.stderr
    except subprocess.TimeoutExpired:
        dur = time.monotonic() - t0
        return pkg, exe, dur, False, f"TIMEOUT after {timeout}s", "", ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=os.cpu_count(),
                    help="并行目标进程数，默认 CPU 核数")
    ap.add_argument("--threads-per-bin", default="2",
                    help="每个二进制内部 --test-threads，默认 2（核数/2，避免超订）")
    ap.add_argument("--timeout", type=int, default=600,
                    help="单目标超时秒数，默认 600（最重目标 rules_complete_wiring 的实测上限）")
    ap.add_argument("--include", default="",
                    help="目标名过滤正则（匹配测试二进制名），空表示全部目标")
    ap.add_argument("--serial", default=r"stats_baseline",
                    help="匹配此正则的目标串行独占运行（默认 stats_baseline：内部已有 8 线程，且多目标同跑会触发 OOM）")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    env = dict(os.environ)
    env["RUST_TEST_THREADS_PER_BIN"] = args.threads_per_bin

    bins = collect_binaries()
    import re
    if args.include:
        pat = re.compile(args.include)
        bins = [(n, e) for n, e in bins if pat.search(n)]
    serial_pat = re.compile(args.serial) if args.serial else None
    if serial_pat is not None:
        parallel = [(n, e) for n, e in bins if not serial_pat.search(n)]
        serial = [(n, e) for n, e in bins if serial_pat.search(n)]
    else:
        parallel, serial = bins, []
    print(f"共 {len(bins)} 个测试目标（并行 {len(parallel)} + 串行 {len(serial)}），"
          f"jobs={args.jobs}, threads/bin={args.threads_per_bin}" +
          (f"，include={args.include}" if args.include else ""))
    t0 = time.monotonic()
    results = []
    out_dir = ROOT / ".work" / "par_test_out"
    out_dir.mkdir(exist_ok=True)

    def execute(name, exe):
        dur, ok, tail, stdout, stderr = run_one(name, exe, args.timeout, env)[2:]
        return name, exe, dur, ok, tail, stdout, stderr

    def record(name, dur, ok, tail):
        results.append((dur, name, ok, tail))
        mark = "OK " if ok else "FAIL"
        print(f"  [{mark}] {dur:7.1f}s  {name}  {tail if (args.verbose or not ok) else ''}")
        sys.stdout.flush()

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futs = {pool.submit(execute, name, exe): (name, exe)
                for name, exe in parallel}
        for fut in concurrent.futures.as_completed(futs):
            name, exe, dur, ok, tail, stdout, stderr = fut.result()
            safe = name.replace(" ", "_").replace("/", "_")
            (out_dir / f"{safe}.log").write_text(stdout + "\n=== STDERR ===\n" + stderr)
            record(name, dur, ok, tail)

    # 串行目标独占运行（在并行池全部结束后）：内存大户不同其他目标同跑。
    for name, exe in serial:
        name, exe, dur, ok, tail, stdout, stderr = execute(name, exe)
        safe = name.replace(" ", "_").replace("/", "_")
        (out_dir / f"{safe}.log").write_text(stdout + "\n=== STDERR ===\n" + stderr)
        record(name, dur, ok, tail)

    total = time.monotonic() - t0
    results.sort(reverse=True)
    failed = [(name, tail) for dur, name, ok, tail in results if not ok]
    print(f"\n总 wall time: {total:.1f}s")
    print("耗时前 8 名：")
    for dur, name, ok, tail in results[:8]:
        print(f"  {dur:7.1f}s  {name}")
    if failed:
        print(f"\n失败 {len(failed)} 个：")
        for name, tail in failed:
            print(f"  {name}: {tail}")
        return 1
    print("\n全部通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
