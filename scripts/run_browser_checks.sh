#!/usr/bin/env zsh
# 两类球场渲染器的端到端验证：启动 debug-server，跑 3D 与 2D 脚本，停服务。
set -uo pipefail
cd /home/ubuntu/workspace/code/nba-sim

PORT=31100
BUILD_LOG=.work/evidence/browser-build.log
SERVER_LOG=.work/evidence/browser-server.log
# 构建与端到端进程都走统一的 2 GiB 地址空间上限入口。
scripts/mem-capped-runner.sh cargo build -p nba-debug-server > "$BUILD_LOG" 2>&1
BUILD=$?
if [ "$BUILD" -ne 0 ]; then
    tail -20 .work/evidence/browser-build.log
    exit "$BUILD"
fi

./scripts/mem-capped-runner.sh ./target/debug/nba-debug-server "$PORT" > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
cleanup() {
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

ready=0
for _ in $(seq 1 60); do
    if curl -fsS "http://127.0.0.1:$PORT/api/action-animation-catalog" > /dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 1
done
if [ "$ready" -ne 1 ]; then
    echo "debug-server 未在 60 秒内就绪" >&2
    tail -20 "$SERVER_LOG" >&2
    exit 1
fi

FAILED=0
./scripts/mem-capped-runner.sh node scripts/test_court3d.mjs > .work/evidence/browser-3d.log 2>&1
STATUS=$?
printf "%-40s %s\n" "test_court3d.mjs" "$STATUS"
if [ "$STATUS" -ne 0 ]; then
    tail -30 .work/evidence/browser-3d.log >&2
    FAILED=$((FAILED + 1))
fi

./scripts/mem-capped-runner.sh python3 scripts/test_court_visuals.py > .work/evidence/browser-2d.log 2>&1
STATUS=$?
printf "%-40s %s\n" "test_court_visuals.py" "$STATUS"
if [ "$STATUS" -ne 0 ]; then
    tail -30 .work/evidence/browser-2d.log >&2
    FAILED=$((FAILED + 1))
fi

exit "$FAILED"
