#!/usr/bin/env zsh
# 多起点持久化搜索链：每次运行都从当前最优参数继续，立即写回。
set -uo pipefail
cd /home/ubuntu/workspace/code/nba-sim
: > .work/evidence/search-chain-final.log
for seed in 424242 515151 606060; do
    printf '==== SEED %s ====\n' "$seed" >> .work/evidence/search-chain-final.log
    python3 -u scripts/calibrate/final_search.py --minutes 20 --seed "$seed" \
        >> .work/evidence/search-chain-final.log 2>&1
    printf '==== SEED %s EXIT=%s ====\n' "$seed" "$?" >> .work/evidence/search-chain-final.log
done
printf 'CHAIN_DONE\n' >> .work/evidence/search-chain-final.log
