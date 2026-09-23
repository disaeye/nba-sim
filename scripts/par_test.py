# 测试目标并行执行器（scripts/par_test.py，正式脚本）。
# Cargo target 使用 target.name；本脚本以 package::target 作为唯一目标名称，
# package 选择器展开为该 crate 的全部测试目标，target 选择器匹配一个精确目标。
import argparse
import concurrent.futures
import json
import os
import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent

TIER_SELECTORS = {
    "tier1": (
        ("target", "nba-domain::nba_domain"),
        ("target", "nba-domain::ball_ownership"),
        ("target", "nba-domain::game_flow_transitions"),
        ("target", "nba-domain::rules_geometry"),
        ("package", "nba-invariants"),
        ("target", "nba-engine::world_state_equivalence"),
        ("target", "nba-engine::engine_snapshot"),
        ("target", "nba-engine::projection"),
        ("target", "nba-engine::golden_hash"),
    ),
    "tier2": (
        ("target", "nba-decision::defense_responsibility_chain"),
        ("target", "nba-engine::nba_engine"),
        ("target", "nba-engine::attribute_perturbation"),
        ("target", "nba-engine::block"),
        ("target", "nba-engine::decision_wiring"),
        ("target", "nba-engine::defense"),
        ("target", "nba-engine::rules_complete_wiring"),
        ("target", "nba-engine::wiring_proof"),
        ("target", "nba-engine::defense_effect"),
        ("target", "nba-engine::fiba_scenarios"),
        ("target", "nba-domain::play_spec"),
        ("target", "nba-domain::playbook_data"),
        ("target", "nba-decision::play_selector"),
        ("target", "nba-decision::play_actions"),
        ("target", "nba-decision::play_decision"),
        ("target", "nba-engine::playbook_setup"),
        ("target", "nba-engine::action_phase_projection"),
        ("target", "nba-engine::shot_release_timing"),
        ("target", "nba-engine::defense_rotation_response"),
    ),
    "tier3": (
        ("target", "nba-engine::stats_baseline"),
    ),
}

TIER_REQUIRED_TARGETS = {
    "tier1": {
        "nba-domain::nba_domain",
        "nba-domain::ball_ownership",
        "nba-domain::game_flow_transitions",
        "nba-domain::rules_geometry",
        "nba-invariants::checker",
        "nba-invariants::nba_invariants",
    },
    "tier2": {
        "nba-domain::play_spec",
        "nba-domain::playbook_data",
        "nba-decision::play_selector",
        "nba-decision::play_actions",
        "nba-decision::play_decision",
        "nba-engine::playbook_setup",
        "nba-engine::action_phase_projection",
        "nba-engine::shot_release_timing",
        "nba-engine::nba_engine",
        "nba-engine::defense_rotation_response",
    },
    "tier3": {
        "nba-engine::stats_baseline",
    },
}

SERIAL_PATTERN = r"^nba-engine::(?:stats_baseline|defense_rotation_response)$"
SERIAL_TARGETS = {
    "nba-engine::stats_baseline",
    "nba-engine::defense_rotation_response",
}


def load_test_targets():
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    metadata = json.loads(out.stdout)
    workspace_members = set(metadata["workspace_members"])
    package_names = {
        package["id"]: package["name"]
        for package in metadata["packages"]
        if package["id"] in workspace_members
    }
    targets = {}
    for package in metadata["packages"]:
        if package["id"] not in workspace_members:
            continue
        for target in package["targets"]:
            if not target.get("test"):
                continue
            identity = f"{package['name']}::{target['name']}"
            if identity in targets:
                raise ValueError(f"duplicate Cargo test target identity: {identity}")
            targets[identity] = package["name"]
    return targets, package_names


def expand_tier_selectors(test_targets, selectors):
    expected = set()
    for kind, value in selectors:
        if kind == "package":
            package_targets = {
                identity for identity, package_name in test_targets.items()
                if package_name == value
            }
            if not package_targets:
                raise ValueError(f"package selector has no test targets: {value}")
            expected.update(package_targets)
        elif kind == "target":
            if value not in test_targets:
                raise ValueError(f"Cargo test target does not exist: {value}")
            expected.add(value)
        else:
            raise ValueError(f"unknown target selector kind: {kind}")
    return expected


def matches_selectors(identity, selectors):
    package_name, separator, _ = identity.partition("::")
    if not separator:
        return False
    return any(
        (kind == "package" and package_name == value)
        or (kind == "target" and identity == value)
        for kind, value in selectors
    )


def select_tier(tier, test_targets):
    selectors = TIER_SELECTORS[tier]
    expected = expand_tier_selectors(test_targets, selectors)
    actual = {
        identity for identity, package_name in test_targets.items()
        if matches_selectors(identity, selectors)
    }
    return expected, actual


def print_target_set(label, targets):
    print(f"{label}（{len(targets)}）:")
    for target in sorted(targets):
        print(f"  {target}")


def run_self_test():
    test_targets, _ = load_test_targets()
    serial_pattern = re.compile(SERIAL_PATTERN)
    failed = False
    for tier in ("tier1", "tier2", "tier3"):
        expected, actual = select_tier(tier, test_targets)
        print(f"\n{tier} 目标筛选自测")
        print_target_set("期望目标名称", expected)
        print_target_set("实际命中集合", actual)
        if expected != actual:
            print(f"FAIL {tier}: expected/actual target sets differ", file=sys.stderr)
            failed = True
        missing_required = TIER_REQUIRED_TARGETS[tier] - actual
        if missing_required:
            print(
                f"FAIL {tier}: required targets missing: {', '.join(sorted(missing_required))}",
                file=sys.stderr,
            )
            failed = True
        serial_actual = {
            target for target in actual if serial_pattern.search(target)
        }
        serial_expected = actual & SERIAL_TARGETS
        print_target_set("串行独占命中集合", serial_actual)
        if serial_actual != serial_expected:
            print(f"FAIL {tier}: serial target classification differs", file=sys.stderr)
            failed = True
    combined_expected, combined_actual = select_tier("tier2", test_targets)
    tier3_expected, tier3_actual = select_tier("tier3", test_targets)
    combined_expected |= tier3_expected
    combined_actual |= tier3_actual
    combined_serial = {
        target for target in combined_actual if serial_pattern.search(target)
    }
    print("\ntier2+3 串行独占自测")
    print_target_set("串行独占命中集合", combined_serial)
    if combined_expected != combined_actual or combined_serial != SERIAL_TARGETS:
        print("FAIL tier2+3: full-game targets are not classified as serial", file=sys.stderr)
        failed = True
    if failed:
        return 1
    print("\n分层目标筛选自测通过；目标清单来自当前 workspace Cargo metadata。")
    return 0


def collect_binaries(package_names):
    out = subprocess.run(
        ["cargo", "test", "--release", "--workspace", "--no-run", "--message-format=json"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    bins = {}
    for line in out.stdout.splitlines():
        obj = json.loads(line)
        if obj.get("reason") != "compiler-artifact":
            continue
        profile = obj.get("profile", {})
        if not profile.get("test"):
            continue
        exe = obj.get("executable")
        if not exe:
            continue
        package_id = obj.get("package_id")
        package_name = package_names.get(package_id)
        if package_name is None:
            raise ValueError(f"test artifact belongs to unknown workspace package: {package_id}")
        target_name = obj.get("target", {}).get("name")
        if not target_name:
            raise ValueError(f"test artifact has no Cargo target name: {package_id}")
        identity = f"{package_name}::{target_name}"
        if identity in bins:
            raise ValueError(f"duplicate compiled Cargo test target: {identity}")
        bins[identity] = exe
    return bins


def run_one(target, exe, timeout, env):
    t0 = time.monotonic()
    try:
        proc = subprocess.run(
            [exe, "--test-threads", env["RUST_TEST_THREADS_PER_BIN"]],
            cwd=ROOT, capture_output=True, text=True, timeout=timeout,
        )
        dur = time.monotonic() - t0
        ok = proc.returncode == 0
        tail = [line for line in proc.stdout.splitlines() if "test result" in line]
        return target, exe, dur, ok, tail[-1] if tail else "", proc.stdout, proc.stderr
    except subprocess.TimeoutExpired:
        dur = time.monotonic() - t0
        return target, exe, dur, False, f"TIMEOUT after {timeout}s", "", ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=os.cpu_count(),
                    help="并行目标进程数，默认 CPU 核数")
    ap.add_argument("--threads-per-bin", default="2",
                    help="每个二进制内部 --test-threads，默认 2（核数/2，避免超订）")
    ap.add_argument("--timeout", type=int, default=600,
                    help="单目标超时秒数，默认 600")
    ap.add_argument("--tier", choices=("tier1", "tier2", "tier3", "tier2+3"),
                    help="按 workspace Cargo metadata 选择分层目标")
    ap.add_argument("--include", default="",
                    help="目标名称正则，匹配 package::target；空表示全部目标")
    ap.add_argument("--serial", default=SERIAL_PATTERN,
                    help="匹配此正则的目标串行独占运行")
    ap.add_argument("--self-test", action="store_true",
                    help="使用当前 workspace Cargo metadata 检查全部分层筛选")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        if args.tier or args.include:
            ap.error("--self-test cannot be combined with --tier or --include")
        return run_self_test()
    if args.tier and args.include:
        ap.error("--tier cannot be combined with --include")

    env = dict(os.environ)
    env["RUST_TEST_THREADS_PER_BIN"] = args.threads_per_bin
    test_targets, package_names = load_test_targets()

    tier_names = ("tier2", "tier3") if args.tier == "tier2+3" else ((args.tier,) if args.tier else ())
    if tier_names:
        selectors = tuple(
            selector for tier in tier_names for selector in TIER_SELECTORS[tier]
        )
        expected = expand_tier_selectors(test_targets, selectors)
        filter_description = f"tier={args.tier}"
    elif args.include:
        include_pattern = re.compile(args.include)
        expected = {
            identity for identity in test_targets
            if include_pattern.search(identity)
        }
        filter_description = f"include={args.include}"
    else:
        expected = set(test_targets)
        filter_description = "workspace"

    if not expected:
        print(f"目标筛选没有命中 Cargo test targets：{filter_description}", file=sys.stderr)
        return 2

    print(f"选择范围：{filter_description}")
    print_target_set("期望目标名称", expected)
    bins = collect_binaries(package_names)
    actual = expected.intersection(bins)
    print_target_set("实际命中集合", actual)
    if expected != actual:
        missing = expected - actual
        print_target_set("未生成的期望目标", missing)
        print("Cargo metadata 与编译产物的目标集合不一致，停止运行。", file=sys.stderr)
        return 2

    serial_pattern = re.compile(args.serial) if args.serial else None
    if serial_pattern is not None:
        parallel = [(name, exe) for name, exe in bins.items()
                    if name in actual and not serial_pattern.search(name)]
        serial = [(name, exe) for name, exe in bins.items()
                  if name in actual and serial_pattern.search(name)]
    else:
        parallel = [(name, bins[name]) for name in sorted(actual)]
        serial = []
    print(f"共 {len(actual)} 个测试目标（并行 {len(parallel)} + 串行 {len(serial)}），"
          f"jobs={args.jobs}, threads/bin={args.threads_per_bin}")
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
        futures = {pool.submit(execute, name, exe): (name, exe)
                   for name, exe in parallel}
        for future in concurrent.futures.as_completed(futures):
            name, exe, dur, ok, tail, stdout, stderr = future.result()
            safe = name.replace(" ", "_").replace("/", "_").replace(":", "_")
            (out_dir / f"{safe}.log").write_text(stdout + "\n=== STDERR ===\n" + stderr)
            record(name, dur, ok, tail)

    # 整场目标在并行池结束后逐个独占运行。
    for name, exe in serial:
        name, exe, dur, ok, tail, stdout, stderr = execute(name, exe)
        safe = name.replace(" ", "_").replace("/", "_").replace(":", "_")
        (out_dir / f"{safe}.log").write_text(stdout + "\n=== STDERR ===\n" + stderr)
        record(name, dur, ok, tail)

    total = time.monotonic() - t0
    results.sort(reverse=True)
    failed = [(name, tail) for dur, name, ok, tail in results if not ok]
    print(f"\n总 wall time: {total:.1f}s")
    print("耗时前 8 名：")
    for dur, name, _, _ in results[:8]:
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
