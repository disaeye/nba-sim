#!/usr/bin/env zsh
# 连续坐标下降：每轮从上一轮写回的最优参数继续。
set -uo pipefail
cd /home/ubuntu/workspace/code/nba-sim
: > .work/evidence/climb.log
for sweep in 1 2 3 4 5 6; do
    printf '==== SWEEP %s ====\n' "$sweep" >> .work/evidence/climb.log
    python3 -u scripts/calibrate/coordinate_gates.py --sweeps 2 \
        >> .work/evidence/climb.log 2>&1
    printf '==== SWEEP %s EXIT=%s ====\n' "$sweep" "$?" >> .work/evidence/climb.log
done
printf 'CLIMB_DONE\n' >> .work/evidence/climb.log
