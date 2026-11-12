#!/usr/bin/env zsh
# 宽步长多种子搜索链：每 20 轮从冻结基线大步重启，避免局部极值。
set -uo pipefail
cd /home/ubuntu/workspace/code/nba-sim
: > .work/evidence/wide-chain.log
for seed in 777777 888888 999999; do
    printf '==== SEED %s ====\n' "$seed" >> .work/evidence/wide-chain.log
    python3 -u scripts/calibrate/parallel_search.py --rounds 150 --seed "$seed" --width 1.0 \
        >> .work/evidence/wide-chain.log 2>&1
    printf '==== SEED %s EXIT=%s ====\n' "$seed" "$?" >> .work/evidence/wide-chain.log
done
printf 'WIDE_CHAIN_DONE\n' >> .work/evidence/wide-chain.log
