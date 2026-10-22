#!/usr/bin/env bash
# cargo 目标程序的统一入口（.cargo/config.toml 的 runner）。
#
# 为什么需要它：失控的测试二进制曾经涨到 19 GiB，内核判定整机内存耗尽后挑目标
# 终止，systemd 又把被终止进程所在的 scope 标成失败并清理其余进程，同一个 tmux
# 窗格里的编辑器、web 服务被一起带走。把目标程序放进独立的 systemd scope 并设
# 上限后，超限只会让内核在这个 scope 内部挑进程终止，scope 之外完全不受影响。
#
# 上限可用 NBA_TEST_MEMORY_MAX 调整。只设硬上限，不设 MemoryHigh：软限会把进程
# 压进持续回收，失控的分配会变成长时间磨磗，而不是快速撞墙终止。
# 没有可用用户级 systemd 的环境（CI 容器等）直接执行原命令。
set -uo pipefail

MAX="${NBA_TEST_MEMORY_MAX:-8G}"

if command -v systemd-run >/dev/null 2>&1 &&
    timeout 5 systemd-run --user --scope --quiet --collect -p MemoryMax=1G true >/dev/null 2>&1; then
    exec systemd-run --user --scope --quiet --collect \
        --unit "nba-cargo-$$-${RANDOM}" \
        -p MemoryAccounting=yes \
        -p MemoryMax="$MAX" \
        -p MemorySwapMax=0 \
        -p OOMPolicy=continue \
        -- "$@"
fi

exec "$@"
