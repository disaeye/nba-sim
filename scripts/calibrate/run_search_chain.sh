#!/usr/bin/env zsh
# 在冻结基线上连续跑多个种子的极小步长搜索；每轮从上一轮最优继续。
set -uo pipefail
cd /home/ubuntu/workspace/code/nba-sim
: > .work/evidence/search-chain.log
for seed in 101 202 303 404; do
    printf '==== SEED %s ====\n' "$seed" >> .work/evidence/search-chain.log
    python3 -u scripts/calibrate/local_gates.py --rounds 250 --seed "$seed" \
        >> .work/evidence/search-chain.log 2>&1
    printf '==== SEED %s EXIT=%s ====\n' "$seed" "$?" >> .work/evidence/search-chain.log
done
printf 'CHAIN_DONE\n' >> .work/evidence/search-chain.log
