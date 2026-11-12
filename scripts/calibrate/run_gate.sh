#!/usr/bin/env zsh
# 用门禁自己的 16 个种子跑权威四层判定，输出到证据目录。
set -o pipefail
cd /home/ubuntu/workspace/code/nba-sim
cargo build --release -p nba-sim-core --bin sim2 || exit 1
./target/release/sim2 --seeds 1..17 --gate > .work/evidence/gate-current.log 2>&1
echo "SIM2_EXIT=$?"
./target/release/nba-sim sim-core --seed 1 --gate > .work/evidence/gate-cli-current.log 2>&1
echo "CLI_EXIT=$?"
