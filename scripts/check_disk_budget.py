#!/usr/bin/env python3
"""测试资源守卫（gap.md §16.4 / problem.md §14.4）。

把「磁盘被打满」从运行事故变成**可提前拦截的守卫失败**。检查三类问题：

1. **磁盘余量**：根分区可用空间低于安全线即失败（默认 3 GiB）。
2. **构建目录体积**：`target/` 超过上限即失败（默认 8 GiB），并单独报告
   增量编译缓存（`target/debug/incremental`）——它是历史上最大的单点浪费。
3. **临时产物残留**：本机临时目录中属于本项目前缀（`nba_`）的孤儿文件
   数量/体积超限即失败。这些是测试 panic 后未清理的遗留物。

用法：
    python3 scripts/check_disk_budget.py            # 检查，超限退出 1
    python3 scripts/check_disk_budget.py --report   # 只打印，不失败
    python3 scripts/check_disk_budget.py --clean    # 清理残留后再检查
"""

import argparse
import os
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET_DIR = ROOT / "target"
TEMP_PREFIX = "nba_"

MIN_FREE_BYTES = 3 * 1024 * 1024 * 1024        # 3 GiB
MAX_TARGET_BYTES = 8 * 1024 * 1024 * 1024       # 8 GiB
MAX_TEMP_LEFTOVER_BYTES = 64 * 1024 * 1024      # 64 MiB
MAX_TEMP_LEFTOVER_FILES = 16


def dir_size(path: Path) -> int:
    if not path.exists():
        return 0
    total = 0
    for entry in path.rglob("*"):
        try:
            if entry.is_file() and not entry.is_symlink():
                total += entry.stat().st_size
        except OSError:
            continue
    return total


def free_bytes(path: Path) -> int:
    usage = shutil.disk_usage(path)
    return usage.free


def temp_roots() -> list:
    """扫描本项目临时产物的目录。

    首位是项目临时文件专用根目录（不与 /tmp、/dev/shm 混用）；
    其余为兼容历史残留的旧位置。
    """
    roots = []
    project_root = Path("/home/ubuntu/basketball")
    if project_root.exists():
        roots.append(project_root)
    for candidate in ("/tmp", "/dev/shm", "/var/tmp"):
        p = Path(candidate)
        if p.exists():
            roots.append(p)
    import os

    tmpdir = os.environ.get("TMPDIR")
    if tmpdir and Path(tmpdir).exists() and Path(tmpdir) not in roots:
        roots.append(Path(tmpdir))
    return roots


# 无 pid 的产物（如 `nba_batch_3.ndjson`）无法判断属主进程，改用
# “文件年龄”判活：超过该秒数即视为孤儿（正在运行的模拟不会几分钟不写）。
STALE_AFTER_SECONDS = 600


def _owner_alive(entry: Path) -> bool:
    """条目是否可能仍被存活进程使用（避免清理并行任务正在写的数据）。

    两类判据：
    1. 目录形如 `nba_test_<pid>`：pid 存在于 /proc 即视为存活；
    2. 其余形式：按“最后修改时间”判活——超过 `STALE_AFTER_SECONDS`
       即视为孤儿。

    为什么需要第 2 类：早先的实现把“无 pid 的条目”一律当作存活，
    于是 `nba_batch_*.ndjson` 这类真正的泄漏（单次实测累积 4.2 GB）
    永远不会被计入，守卫形同虚设。
    """
    import time

    name = entry.name
    match = re.match(rf"^{TEMP_PREFIX}test_(\d+)$", name)
    if match:
        pid = int(match.group(1))
        if pid == os.getpid():
            return True
        if Path("/proc").exists():
            return Path(f"/proc/{pid}").exists()
        return True
    try:
        age = time.time() - entry.stat().st_mtime
    except OSError:
        return True
    return age < STALE_AFTER_SECONDS


def find_leftovers(include_live: bool = False) -> list:
    found = []
    for root in temp_roots():
        try:
            entries = list(root.iterdir())
        except OSError:
            continue
        for entry in entries:
            if not entry.name.startswith(TEMP_PREFIX):
                continue
            if not include_live and _owner_alive(entry):
                # 并行测试正在使用；不计入泄漏，也不清理。
                continue
            if entry.is_dir():
                size = dir_size(entry)
            else:
                try:
                    size = entry.stat().st_size
                except OSError:
                    size = 0
            found.append((entry, size))
    # 有数据的条目优先，其次按名称稳定排序（便于确定性输出）。
    found.sort(key=lambda item: (-item[1], str(item[0])))
    return found


def human(n: int) -> str:
    for unit in ("B", "KiB", "MiB", "GiB", "TiB"):
        if n < 1024 or unit == "TiB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1024.0
    return f"{n:.1f} TiB"


def clean_leftovers(leftovers) -> int:
    freed = 0
    for path, size in leftovers:
        try:
            if path.is_dir():
                shutil.rmtree(path, ignore_errors=True)
            else:
                path.unlink(missing_ok=True)
            freed += size
        except OSError:
            continue
    return freed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", action="store_true", help="print the budget report and exit 0")
    parser.add_argument("--clean", action="store_true", help="remove project temp leftovers first")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="negative control: create a fake leak and assert the guard fails",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    failures = []

    leftovers = find_leftovers()
    if args.clean:
        freed = clean_leftovers(leftovers)
        print(f"🧹 Cleaned {len(leftovers)} stale project temp entries, freed {human(freed)}")
        leftovers = find_leftovers()

    # 空目录不占空间，但仍由 --clean 清除；只有实际数据才算泄漏。
    data_leftovers = [(p, s) for p, s in leftovers if s > 0]
    leftover_bytes = sum(size for _, size in data_leftovers)
    target_bytes = dir_size(TARGET_DIR)
    incremental = dir_size(TARGET_DIR / "debug" / "incremental")
    free = free_bytes(ROOT)

    print("📊 Test resource budget report")
    print(f"   Free space                : {human(free)} (min {human(MIN_FREE_BYTES)})")
    print(f"   target/                   : {human(target_bytes)} (max {human(MAX_TARGET_BYTES)})")
    print(f"     └─ debug/incremental    : {human(incremental)}")
    empty_leftovers = len(leftovers) - len(data_leftovers)
    print(
        f"   temp leftovers ({TEMP_PREFIX}*): {len(data_leftovers)} with data "
        f"({human(leftover_bytes)}), {empty_leftovers} empty dirs "
        f"(max {MAX_TEMP_LEFTOVER_FILES} entries / {human(MAX_TEMP_LEFTOVER_BYTES)})"
    )
    for path, size in leftovers[:8]:
        print(f"      {human(size):>10}  {path}")

    if free < MIN_FREE_BYTES:
        failures.append(
            f"free space {human(free)} below the {human(MIN_FREE_BYTES)} safety line"
        )
    if target_bytes > MAX_TARGET_BYTES:
        failures.append(
            f"target/ is {human(target_bytes)}, above {human(MAX_TARGET_BYTES)}; "
            f"run `cargo clean` or set CARGO_INCREMENTAL=0"
        )
    if len(data_leftovers) > MAX_TEMP_LEFTOVER_FILES or leftover_bytes > MAX_TEMP_LEFTOVER_BYTES:
        failures.append(
            f"{len(data_leftovers)} temp leftovers totalling {human(leftover_bytes)}; "
            f"tests must use nba-test-support::TempArtifact (panic-safe cleanup). "
            f"Run with --clean to remove them."
        )

    if args.report:
        return 0

    if failures:
        print("\n❌ Test resource guard FAILED:")
        for f in failures:
            print(f"   - {f}")
        return 1
    print("\n✅ Test resource guard passed.")
    return 0


def self_test() -> int:
    """负面对照：制造一个泄漏，断言守卫会失败；清理后断言通过。"""
    probe_root = Path("/home/ubuntu/basketball")
    probe_root.mkdir(parents=True, exist_ok=True)
    probe_dir = probe_root / f"{TEMP_PREFIX}test_888888"
    probe_dir.mkdir(parents=True, exist_ok=True)
    (probe_dir / "leak.ndjson").write_bytes(b"x" * (80 * 1024 * 1024))
    try:
        leftovers = [p for p, s in find_leftovers() if s > 0]
        if not leftovers:
            print("❌ self-test: fabricated leak was not detected")
            return 1
        # 清理后必须没有数据残留。
        clean_leftovers(find_leftovers())
        if [p for p, s in find_leftovers() if s > 0]:
            print("❌ self-test: cleanup did not remove the fabricated leak")
            return 1
        print("✅ Test resource guard self-test passed: detects and cleans fabricated leaks.")
        return 0
    finally:
        shutil.rmtree(probe_dir, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
